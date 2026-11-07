// Watchdog for the repo-wide interpreter gate (robustness-ci leaf
// 00-ci-core/LEAF-gate-watchdog).
//
// Problem (observed 2026-11-06): a wedged holder parked on the shared
// `aprun_gate_global.lock` for ~3h45m while every other interpreter-spawning
// suite queued behind a plain blocking `flock(LOCK_EX)`. This module replaces
// the blocking acquire with:
//
// 1. A bounded poll loop: `LOCK_EX | LOCK_NB` every 500ms until
//    `AP_GATE_WAIT_SECS` elapses (default 1800s; `0` restores the old
//    infinite-block behavior).
// 2. A holder-liveness record in the lock file — immediately after acquiring
//    the flock, the holder writes `AP_GATE_HOLDER pid=<pid> start=<secs>`
//    (process start time from `proc_pidinfo(PROC_PIDTBSDINFO)` on macOS,
//    `/proc/<pid>/stat` field 22 on Linux). The record is never cleaned up on
//    release; staleness is detected by readers.
// 3. Stale-holder takeover: while waiting, the waiter reads the record. If
//    the recorded process no longer exists (`kill(pid, 0) == ESRCH`, with the
//    recorded start time guarding against PID reuse), a live flock from a
//    dead process is impossible (the kernel closes its fds at death), so the
//    waiter retries `LOCK_EX | LOCK_NB` and takes over, warning on stderr.
//    If the non-blocking flock still fails, the record was stale but the
//    lock is live (a fresh holder that has not rewritten it yet): keep
//    waiting. A live pid with a mismatched start time is PID reuse, NOT a
//    dead holder — never taken over.
// 4. Suite-level deadline safety net: when the bounded wait expires, the
//    waiting test FAILS with an actionable panic naming the holder and the
//    knob, instead of blocking the suite fleet forever.
//
// Lock file path stays `temp_dir()/aprun_gate_global.lock` and the flock
// stays held-to-process-exit, so every existing suite interoperates without
// a flag day. `libc` + `std` only — no new dependencies.
//
// This file is duplicated per package under `tests/common/` (Cargo
// integration tests are separate crates; see the duplication note in each
// package's `mod.rs`) and `include!`d verbatim by the inline seam copies
// (xtask_gates, harness_selfcheck, ast_getattr_fence). Keep all copies
// byte-identical; edit this canonical copy and re-copy. This file is
// consumed two ways — as `mod gate_watchdog;` (no crate-level attrs in
// scope) and via `include!` after outer attributes (so NO inner
// attributes or doc comments may appear here); plain comments only, and
// unused-item allowances live on the items.

// NOTE: no `use` statements here — everything is fully qualified, because
// this file is include!d into crates that already import Duration/Instant
// etc. and a bare `use` would collide (E0252).

/// Poll interval while the gate is held by someone else.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Default bounded wait: generous headroom over legitimate long holds
/// (proptest shrinking runs minutes at high CPU), but no more multi-hour
/// silent queues. `AP_GATE_WAIT_SECS=0` disables the bound entirely
/// (old blocking behavior).
const DEFAULT_WAIT_SECS: u64 = 1800;

/// Reserved names for the `include!` consumers (their callers may not use
/// every helper). Harmless no-op in the `mod` consumers.
#[allow(dead_code)]
type GateWatchdogUnused = ();

/// Read `AP_GATE_WAIT_SECS`: `Some(secs)` = bounded wait, `None` = the old
/// infinite block (env value `0`, unparsable values fall back to the default).
fn gate_wait_secs() -> Option<u64> {
    match std::env::var("AP_GATE_WAIT_SECS") {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(secs),
            Err(_) => Some(DEFAULT_WAIT_SECS),
        },
        Err(_) => Some(DEFAULT_WAIT_SECS),
    }
}

/// Process start time (seconds since boot epoch) for `pid`, or `None` when
/// the process does not exist / cannot be inspected. Used as a PID-reuse
/// guard: a reused PID has a different start time.
#[cfg(target_os = "macos")]
pub fn process_start_time_secs(pid: u32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        )
    };
    if n as usize != std::mem::size_of::<libc::proc_bsdinfo>() {
        return None;
    }
    Some(info.pbi_start_tvsec as u64)
}

/// Linux twin: `/proc/<pid>/stat` field 22 (starttime, in clock ticks after
/// boot). Field 3 (`state`) is the first token after the closing paren of
/// the comm field, so starttime is the 20th token from there.
#[cfg(target_os = "linux")]
pub fn process_start_time_secs(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    stat[close + 2..]
        .split_whitespace()
        .nth(19) // field 3 + 19 = field 22 (starttime)
        .and_then(|t| t.parse::<u64>().ok())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn process_start_time_secs(_pid: u32) -> Option<u64> {
    None // no reuse guard available: treated as "alive but unverifiable"
}

/// Liveness verdict for a recorded gate holder.
#[allow(dead_code)] // Reused/Gone/Alive arms unused in some consumers
enum HolderLiveness {
    /// PID exists with the recorded start time — live holder, keep waiting.
    Alive,
    /// PID does not exist — the holder is dead; a live flock from it is
    /// impossible, safe to attempt takeover.
    Gone,
    /// PID exists but its start time differs from the record — PID reuse;
    /// the ORIGINAL holder is gone, but this pid is someone else's process.
    /// Never taken over (we would kill nothing, but the record tells us
    /// nothing about the CURRENT flock owner, so we keep waiting).
    Reused,
}

fn holder_liveness(pid: u32, recorded_start: u64) -> HolderLiveness {
    match process_start_time_secs(pid) {
        Some(start) if start == recorded_start => HolderLiveness::Alive,
        Some(_) => HolderLiveness::Reused,
        None => {
            // Uninspectable: fall back to signal-0 probing. On EPERM (a
            // foreign-uid process with the same pid) treat as Gone — the
            // takeover attempt is still protected by LOCK_NB.
            let rc = unsafe { libc::kill(pid as libc::c_int, 0) };
            if rc == 0 {
                HolderLiveness::Reused
            } else {
                HolderLiveness::Gone
            }
        }
    }
}

/// Read `AP_GATE_HOLDER pid=<pid> start=<secs>` from the lock file, if a
/// complete record is present.
fn read_holder_record(file: &mut std::fs::File) -> Option<(u32, u64)> {
    let mut buf = String::new();
    let f = file;
    std::io::Seek::seek(f, std::io::SeekFrom::Start(0)).ok()?;
    std::io::Read::read_to_string(f, &mut buf).ok()?;
    for line in buf.lines() {
        if let Some(rest) = line.strip_prefix("AP_GATE_HOLDER ") {
            let mut pid = None;
            let mut start = None;
            for kv in rest.split_whitespace() {
                if let Some(v) = kv.strip_prefix("pid=") {
                    pid = v.parse::<u32>().ok();
                } else if let Some(v) = kv.strip_prefix("start=") {
                    start = v.parse::<u64>().ok();
                }
            }
            if let (Some(p), Some(s)) = (pid, start) {
                return Some((p, s));
            }
        }
    }
    None
}

/// Overwrite the lock file with OUR holder record (ftruncate + write +
/// flush + fsync) so waiters can probe our liveness. Call only while
/// holding the flock.
fn write_holder_record(file: &mut std::fs::File) {
    let record = format!(
        "AP_GATE_HOLDER pid={} start={}\n",
        std::process::id(),
        process_start_time_secs(std::process::id()).unwrap_or(0)
    );
    let _ = file.set_len(0);
    let _ = std::io::Seek::seek(file, std::io::SeekFrom::Start(0));
    let _ = std::io::Write::write_all(file, record.as_bytes());
    let _ = std::io::Write::flush(file); // &mut File: Write impl is via &File
    let _ = file.sync_all();
}

/// One non-blocking exclusive flock attempt: `Ok(())` = acquired.
fn try_flock_ex(file: &std::fs::File) -> bool {
    use std::os::fd::AsFd as _;
    use std::os::fd::AsRawFd as _;
    let rc = unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    rc == 0
}

/// Acquire the repo-wide interpreter gate, with watchdog semantics.
///
/// Taken once per test-binary process (first call wins), held to process
/// exit. Behavior while the gate is held elsewhere:
/// - poll every 500ms, probing the recorded holder for liveness;
/// - a DEAD holder (kill(pid,0)==ESRH, start-time guarded) is taken over
///   immediately with a stderr warning;
/// - otherwise wait up to `AP_GATE_WAIT_SECS` (default 1800), then panic
///   with an actionable message instead of blocking forever;
/// - `AP_GATE_WAIT_SECS=0` restores the old unbounded blocking flock.
pub fn gate_watchdog_acquire() {
    static DONE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if DONE.swap(1, std::sync::atomic::Ordering::SeqCst) == 1 {
        return;
    }
    let path = std::env::temp_dir().join("aprun_gate_global.lock");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .expect("open interpreter-gate lock file");

    if try_flock_ex(&file) {
        write_holder_record(&mut file);
        std::mem::forget(file); // hold flock + record to process exit
        return;
    }

    let wait = gate_wait_secs();
    let deadline = wait.map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
    loop {
        // Probe the recorded holder each iteration (cheap: one file read +
        // one syscall per 500ms). A dead holder means the kernel has already
        // closed its fd, so a live flock from it is impossible — take over.
        if let Some((pid, start)) = read_holder_record(&mut file) {
            if matches!(holder_liveness(pid, start), HolderLiveness::Gone) && try_flock_ex(&file) {
                eprintln!("gate: took over stale lock from pid {pid}");
                write_holder_record(&mut file);
                std::mem::forget(file);
                return;
            }
        }
        if let Some(d) = deadline {
            if std::time::Instant::now() >= d {
                let mins = wait.unwrap_or(0) / 60;
                let holder = read_holder_record(&mut file)
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                panic!(
                    "interpreter gate held by pid {holder} with no progress for {mins} min; \
                     kill it and re-run (AP_GATE_WAIT_SECS=0 restores unbounded waiting)"
                );
            }
        }
        std::thread::sleep(POLL_INTERVAL);
        if try_flock_ex(&file) {
            write_holder_record(&mut file);
            std::mem::forget(file);
            return;
        }
    }
}

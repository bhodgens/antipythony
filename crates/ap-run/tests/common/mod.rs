//! Shared test-support helpers for the ap-run differential suites.
//!
//! Executor model (memory-OOM mitigation, 2026-11-05): every probe runs the
//! interpreter in a SUBPROCESS (`ap-run-probe` bin, `CARGO_BIN_EXE_`-fresh),
//! never in-process via `ap_run::run_module` inside the test binary. The
//! tracing-GC heap accumulated by probe N previously stayed resident into
//! probe N+1 — the gc_differential suite alone reached ~10 GB RSS and, when
//! two gate scripts launched overlapping `cargo test` invocations of the
//! same suite, ~21 GB, which contributed to an OOM that killed unrelated
//! processes on this box. A subprocess's heap dies with the process.
//!
//! Cross-process serialization: `run_aprun_eval` takes an exclusive flock
//! on `/tmp/aprun_heavy_gate.lock` for the duration of each probe. Two
//! cargo test invocations of the heavy suites can no longer overlap their
//! interpreters; the second waits instead of doubling the footprint.
//! The reference (CPython) side never takes the lock — it is megabytes.

#![allow(dead_code)] // each suite target compiles the subset it uses

mod gate_watchdog;

use std::os::fd::AsFd as _;
use std::os::fd::AsRawFd as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// True when `program --version` succeeds and reports Python 3.14.
/// Pre-3.8 interpreters print `--version` to stderr, so check both streams
/// (same discipline as `crates/ap-conform`'s `is_python_314`).
fn is_python_314(program: &str) -> bool {
    let out = match std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return false,
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.contains("Python 3.14")
}

/// One-shot resolution ladder, cached across the suite's tests.
/// `None` = no reference interpreter usable as CPython 3.14 on this host.
fn discover() -> Option<&'static str> {
    static REF: OnceLock<Option<String>> = OnceLock::new();
    REF.get_or_init(|| {
        if let Ok(p) = std::env::var("AP_CONFORM_REFERENCE_PYTHON") {
            if !p.trim().is_empty() {
                if is_python_314(&p) {
                    return Some(p);
                }
                eprintln!(
                    "warning: AP_CONFORM_REFERENCE_PYTHON={p} does not report Python 3.14; \
                     falling back to PATH python3.14"
                );
            }
        }
        if is_python_314("python3.14") {
            return Some("python3.14".to_string());
        }
        None
    })
    .as_deref()
}

/// True when the skip-to-fail policy is requested (CI sets this; master.md
/// "Skip policy" contract). Any value other than empty/"0" counts.
fn require_reference() -> bool {
    match std::env::var("AP_REQUIRE_REFERENCE") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

/// Reference interpreter program for differential suites.
///
/// `Some(program)` when a version-verified Python 3.14 was resolved.
/// When none is found:
/// - `AP_REQUIRE_REFERENCE=1`: panics with the canonical CI message — a
///   missing reference is a hard failure, never a silent green.
/// - otherwise: prints `SKIP: python3.14 not found` to stderr and returns
///   `None`; the calling test must return early (the suite's local-convenience
///   skip shape, kept per tests/README "Running" discipline).
pub fn reference_or_skip() -> Option<&'static str> {
    if let Some(program) = discover() {
        return Some(program);
    }
    if require_reference() {
        panic!("reference interpreter not found (AP_REQUIRE_REFERENCE=1)");
    }
    eprintln!("SKIP: python3.14 not found");
    None
}

// ---------------------------------------------------------------------------
// Subprocess executor (ap-run-probe) + heavy-suite cross-process lock
// ---------------------------------------------------------------------------

/// Sequence counter for unique temp probe filenames across a process.
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Suite-level single-tenancy lock (2026-11-06 OOM mitigation 4)
// ---------------------------------------------------------------------------

/// Exclusive whole-invocation lock for EVERY suite that spawns the
/// antipythony interpreter. Taken once per test-binary process (first call
/// wins, held to process exit), NOT per probe: two concurrent `cargo test`
/// invocations of any interpreter-spawning suites fully serialize instead
/// of running dozens of debug-build interpreters concurrently (24 test
/// threads here). This is the repo-wide "single tenancy" gate that the
/// per-probe `HeavyGateLock` alone cannot give (it only covers the suites
/// that call `run_aprun_eval`).
///
/// Lock file: /tmp/aprun_gate_global.lock. Persists in /tmp by the same
/// no-unlink rationale as HeavyGateLock (unlink-after-release race).
///
/// Every suite that spawns `antipythony`/`ap-run-eval`/`ap-run-probe` calls
/// this at the top of each test fn (cheap after the first: one atomic load).
///
/// Robustness (robustness-ci leaf 00-ci-core/LEAF-gate-watchdog): bounded
/// wait + holder-liveness record + stale-holder takeover live in
/// `gate_watchdog.rs`; a wedged holder no longer poisons the suite fleet
/// (see that file's header for the full contract).
pub fn interpreter_gate() {
    gate_watchdog::gate_watchdog_acquire();
}

/// Exclusive cross-process lock for the heavy in-process-tier executor.
/// Held only across one probe's subprocess lifetime; two concurrent
/// `cargo test` invocations of these suites serialize instead of running
/// ~10 GB debug-build interpreters simultaneously (the 2026-11-05 OOM).
struct HeavyGateLock {
    file: std::fs::File,
}

impl HeavyGateLock {
    fn acquire() -> Self {
        let path = std::env::temp_dir().join("aprun_heavy_gate.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .expect("open heavy-gate lock file");
        // Blocking flock: the second gate script waits here, it does not
        // fail. libc is a direct dependency of ap-run (os.utime arm), so it
        // is already in this test crate's dependency closure.
        let rc = unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_EX) };
        assert_eq!(rc, 0, "flock on heavy-gate lock failed");
        Self { file }
    }
}

impl Drop for HeavyGateLock {
    fn drop(&mut self) {
        let _ = unsafe { libc::flock(self.file.as_fd().as_raw_fd(), libc::LOCK_UN) };
        // Deliberately do NOT unlink: an unlink-after-release race lets the
        // waiting process's flock be held against a file no future acquirer
        // opens (classic lock-file footgun). The lock file persists in
        // /tmp — harmless.
    }
}

/// Run ANY command under the same fence as `run_aprun_eval` (RSS cap +
/// deadline + cross-process heavy lock), collecting full Output.
///
/// 2026-11-06 OOM mitigation 2: the six differential suites that spawn the
/// interpreter via `Command::new(...).output()` had NO memory fence — an
/// unfenced debug-build interpreter that hits a growth bug can take the
/// machine down (the 2026-11-05 jetsam events). Every interpreter-spawning
/// call goes through here instead of bare `.output()`.
///
/// The interpreter-side RSS self-guard (ap_run::rss_guard, default 25% of
/// RAM) also covers these spawns; this fence adds the tighter 6 GB test cap
/// + deadline + the cross-process lock.
pub fn fenced_output(cmd: &mut std::process::Command) -> std::process::Output {
    preflight_or_panic();
    let _lock = HeavyGateLock::acquire();
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn fenced cmd: {e}"));

    let timeout_secs: u64 = std::env::var("AP_PROBE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240);
    let rss_cap_bytes: u64 = std::env::var("AP_PROBE_MAX_RSS_GB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(6)
        .saturating_mul(1024 * 1024 * 1024);

    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut rss_fail_count: u32 = 0;
    const RSS_FAIL_LIMIT: u32 = 20; // 2s of sampling blindness -> kill
    let fence_violation: Option<String> = loop {
        match child.try_wait().expect("wait fenced child") {
            Some(_status) => break None,
            None => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Some(format!(
                        "deadline: exceeded {timeout_secs}s (hang or pathological growth)"
                    ));
                }
                if rss_cap_bytes > 0 {
                    match footprint_bytes_of(pid) {
                        Some(bytes) => {
                            rss_fail_count = 0;
                            if bytes > rss_cap_bytes {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Some(format!(
                                    "memory: footprint {:.2} GB exceeded the {}GB fence",
                                    bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                                    rss_cap_bytes / (1024 * 1024 * 1024)
                                ));
                            }
                        }
                        None => {
                            rss_fail_count += 1;
                            if rss_fail_count >= RSS_FAIL_LIMIT {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Some(format!(
                                    "memory-fence blind: {rss_fail_count} consecutive \
                                     footprint-sample failures; killed to avoid an \
                                     unfenced run"
                                ));
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    };
    if let Some(why) = fence_violation {
        panic!("fenced_output killed by fence: {why}");
    }
    child.wait_with_output().expect("collect fenced output")
}

/// Resolve the `ap-run-probe` bin path: `CARGO_BIN_EXE_ap-run-probe` when
/// cargo provides it (integration tests of the package that owns the bin —
/// bin-fresh by construction), else the shared target dir's debug path
/// (for suites that reference the bin from another package or a manual
/// `cargo test` with a prebuilt binary).
fn ap_run_probe_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_ap-run-probe") {
        return p.into();
    }
    // Fall back to the shared target dir (respects CARGO_TARGET_DIR for
    // shared-target builds — same discipline as re_differential.rs).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("mod.rs:267")
        .parent()
        .expect("workspace root above crate dir")
        .to_path_buf();
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-probe"),
        Err(_) => root.join("target/debug/ap-run-probe"),
    }
}

/// Preflight memory-pressure gate (2026-11-06 mitigation 5): refuse to start
/// a heavy interpreter probe when the machine is already under memory
/// pressure — the 2026-11-05 events show a probe STARTING while the box was
/// saturated turns a leak into a full-system stall. Reads
/// kern.memorystatus_vm_pressure_level (macOS, 1=normal 2=warning
/// 3=critical) or checks Linux PSI-free fallback (MemAvailable). Errors OPEN
/// (skip the gate) on any read failure: an unreadable pressure metric must
/// not break the suite, only a READ pressure value gates.
#[cfg(target_os = "macos")]
fn memory_pressure_ok() -> bool {
    let mut level: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr() as *const libc::c_char,
            (&mut level as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return true; // metric unreadable: gate open
    }
    level <= 2 // normal or warning: proceed; critical (3+): refuse
}

#[cfg(not(target_os = "macos"))]
fn memory_pressure_ok() -> bool {
    if let Ok(avail) = std::fs::read_to_string("/proc/meminfo") {
        for line in avail.lines() {
            if let Some(rest) = line.strip_prefix("MemAvailable:") {
                if let Ok(kb) = rest.trim().trim_end_matches(" kB").trim().parse::<u64>() {
                    return kb > 4 * 1024 * 1024; // > 4GB available
                }
            }
        }
    }
    true // metric unreadable: gate open
}

/// Panic with a distinctive message when the machine is already under
/// critical memory pressure. Called by both fenced runners.
fn preflight_or_panic() {
    assert!(
        memory_pressure_ok(),
        "memory-pressure preflight: machine under critical memory pressure \
         (kern.memorystatus_vm_pressure_level >= 3 / MemAvailable < 4GB); \
         heavy interpreter probe REFUSED to start — free the machine and rerun"
    );
}

/// Run one probe through the ap-run interpreter as a SUBPROCESS.
/// Returns (stdout, stderr, exit code).
///
/// Per-probe subprocess = per-probe heap teardown; no accumulation across
/// the suite's probes. The lock serializes heavy probes across processes.
///
/// Hard fence (2026-11-05 OOM mitigation, hardened 2026-11-06): each probe
/// gets a wall-clock deadline (default 240s, overridable with
/// AP_PROBE_TIMEOUT_SECS) and a resident-memory cap (default 6 GB,
/// overridable with AP_PROBE_MAX_RSS_GB; 0 disables). One unittest probe
/// was observed growing to ~44 GB RSS on this 64 GB box before dying; with
/// the fence a pathological probe is SIGKILLed at the cap, the suite's
/// assert fails with a distinctive message, and unrelated processes on the
/// box survive.
///
/// 2026-11-06 hardening (post-fence jetsam events 17:58/18:01): RSS now
/// comes from `proc_pid_rusage` (in-process syscall, jetsam's footprint
/// metric) instead of a `ps` subprocess — under memory pressure the `ps`
/// spawn could FAIL and silently disable the fence. Poll interval tightened
/// 250ms -> 100ms (the clone-leak class grows ~2 GB/s). A failed RSS read
/// is counted; N consecutive failures kill the probe (fail closed) instead
/// of polling forever blind.
pub fn run_aprun_eval(src: &str) -> (Vec<u8>, Vec<u8>, i32) {
    preflight_or_panic();
    let _lock = HeavyGateLock::acquire();
    let dir = std::env::temp_dir().join(format!(
        "aprun-probe-{}-{}",
        std::process::id(),
        PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create probe temp dir");
    let path = dir.join("probe.py");
    std::fs::write(&path, src).expect("write probe.py");

    let timeout_secs: u64 = std::env::var("AP_PROBE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240);
    let rss_cap_bytes: u64 = std::env::var("AP_PROBE_MAX_RSS_GB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(6)
        .saturating_mul(1024 * 1024 * 1024);

    let bin = ap_run_probe_bin();
    let mut child = std::process::Command::new(&bin)
        .arg(&path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));

    // Watchdog: poll exit status + footprint every 100ms. Footprint comes
    // from proc_pid_rusage (macOS) / VmRSS (Linux) — no subprocess, so the
    // fence survives memory pressure. A pathological probe is killed at the
    // cap; a hung one at the deadline.
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut rss_fail_count: u32 = 0;
    const RSS_FAIL_LIMIT: u32 = 20; // 20 x 100ms = 2s of blindness -> kill
    let outcome = loop {
        match child.try_wait().expect("wait probe") {
            Some(status) => {
                let out = child.wait_with_output().expect("collect probe output");
                break Ok((out.stdout, out.stderr, status.code().unwrap_or(-1)));
            }
            None => {
                let now = std::time::Instant::now();
                if now >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "deadline: exceeded {timeout_secs}s (hang or pathological growth); \
                         raise AP_PROBE_TIMEOUT_SECS only if the probe is legitimately slow"
                    ));
                }
                if rss_cap_bytes > 0 {
                    match footprint_bytes_of(pid) {
                        Some(bytes) => {
                            rss_fail_count = 0;
                            if bytes > rss_cap_bytes {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Err(format!(
                                    "memory: footprint {:.2} GB exceeded the \
                                     {}GB fence (AP_PROBE_MAX_RSS_GB); this probe's \
                                     interpreter path is growing unbounded",
                                    bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                                    rss_cap_bytes / (1024 * 1024 * 1024)
                                ));
                            }
                        }
                        None => {
                            rss_fail_count += 1;
                            // Fail closed: the fence must never degrade into
                            // an unfenced run (the 17:58/18:01 events).
                            if rss_fail_count >= RSS_FAIL_LIMIT {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Err(format!(
                                    "memory-fence blind: {rss_fail_count} consecutive \
                                     footprint-sample failures; killed to avoid an \
                                     unfenced run (proc_pid_rusage/VmRSS unreadable)"
                                ));
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    match outcome {
        Ok(triple) => triple,
        Err(why) => panic!("ap-run-probe killed by fence: {why}"),
    }
}

/// Resident footprint of `pid` in BYTES. macOS: proc_pid_rusage
/// ri_phys_footprint (jetsam's own metric). Linux: /proc/<pid>/status VmRSS.
/// `None` = unreadable (process gone, or the metric is unavailable) — the
/// watchdog counts consecutive failures and kills the probe (fail closed).
#[cfg(target_os = "macos")]
fn footprint_bytes_of(pid: u32) -> Option<u64> {
    unsafe {
        let mut info: libc::rusage_info_v0 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V0,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        );
        if rc != 0 {
            return None;
        }
        Some(info.ri_phys_footprint)
    }
}

#[cfg(not(target_os = "macos"))]
fn footprint_bytes_of(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

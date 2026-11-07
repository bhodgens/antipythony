//! Gate-watchdog acceptance tests (robustness-ci leaf 00-ci-core/LEAF-gate-watchdog).
//!
//! Acceptance item 1: process A takes the gate, writes its PID record, then
//! exits WITHOUT releasing explicitly; the gate must be acquirable within a
//! poll interval after A's death, and A's recorded start time must match its
//! live start time while it runs (PID-reuse guard input).
//!
//! Isolation discipline: this binary never calls the process-wide
//! `interpreter_gate()` seam (that would freeze this suite against the real
//! gate), and it SKIPS ITSELF when the real gate is currently held by a live
//! external suite (e.g. a running repr_fuzz lane) — a live holder is never
//! contended with, only probed. The child side re-executes this same test
//! binary under `--exact` with `AP_GATE_TEST_CHILD=1` and performs the raw
//! hold-then-die sequence (flock + record + exit).

mod common;

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsFd as _;
use std::os::fd::AsRawFd as _;
use std::time::{Duration, Instant};

const LOCK_PATH: &str = "aprun_gate_global.lock";

fn lock_path() -> std::path::PathBuf {
    std::env::temp_dir().join(LOCK_PATH)
}

/// Read the holder record straight from the real lock file.
fn read_record() -> Option<(u32, u64)> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .open(lock_path())
        .ok()?;
    let mut buf = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut buf).ok()?;
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

/// True when the real gate is currently FREE (acquirable LOCK_NB).
fn gate_free() -> bool {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path())
        .expect("open gate lock");
    let rc = unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_UN) };
        true
    } else {
        false
    }
}

/// Child mode (`AP_GATE_TEST_CHILD=1`, invoked under `--exact` by the
/// parent test): take the gate with a bounded non-blocking retry loop,
/// write OUR holder record, then EXIT WITHOUT releasing the flock — the
/// kernel closes the fd at process exit, which is exactly the real-world
/// release path the takeover logic must handle.
///
/// Exit codes: 0 = held and recorded, 3 = could not acquire in time
/// (external contention raced in; parent skips).
#[test]
fn gate_child_hold_then_die() {
    if std::env::var("AP_GATE_TEST_CHILD").as_deref() != Ok("1") {
        // Running as part of a normal `cargo test` sweep: nothing to do.
        eprintln!("SKIP: child mode not requested");
        return;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path())
        .expect("child: open gate lock");
    let deadline = Instant::now() + Duration::from_secs(8);
    let acquired = loop {
        if unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 } {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !acquired {
        std::process::exit(3); // external holder raced in: parent must skip
    }
    let record = format!(
        "AP_GATE_HOLDER pid={} start={}\n",
        std::process::id(),
        common::gate_watchdog::process_start_time_secs(std::process::id()).unwrap_or(0)
    );
    let _ = file.set_len(0);
    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.write_all(record.as_bytes());
    let _ = file.flush();
    // Hold briefly so the parent can observe the LIVE holder, then die
    // without any explicit release.
    std::thread::sleep(Duration::from_millis(1500));
}

#[test]
fn gate_holder_record_follows_the_lock() {
    // Skip when a live external suite holds the gate (never contend).
    if !gate_free() {
        eprintln!("SKIP: interpreter gate currently held by a live suite");
        return;
    }

    let self_exe = std::env::current_exe().expect("current_exe");
    let mut child = std::process::Command::new(&self_exe)
        .args(["--exact", "gate_child_hold_then_die", "--test-threads", "1"])
        .env("AP_GATE_TEST_CHILD", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn holder child");

    // Wait for the child to write its holder record: a pid that is not ours
    // AND that is currently alive (a stale record from a previous run's
    // dead child must never be accepted as THIS child's record).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut record = None;
    while Instant::now() < deadline {
        if let Some(r) = read_record() {
            if r.0 != std::process::id()
                && common::gate_watchdog::process_start_time_secs(r.0).is_some()
            {
                record = Some(r);
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some((pid, start)) = record else {
        let _ = child.kill();
        let _ = child.wait();
        if !gate_free() {
            eprintln!("SKIP: external holder raced in before child acquired");
            return;
        }
        panic!("child never wrote its holder record");
    };

    // While the child lives: its recorded start time must match the live
    // process start time (this is the PID-reuse guard input the takeover
    // loop compares).
    assert_eq!(
        common::gate_watchdog::process_start_time_secs(pid),
        Some(start),
        "recorded start time must match the live child's start time"
    );

    // Child exits WITHOUT releasing: the kernel drops the flock at fd close.
    let status = child.wait().expect("wait for child");
    if status.code() == Some(3) {
        eprintln!("SKIP: external holder raced in; child could not acquire");
        return;
    }
    assert!(status.success(), "child exited abnormally: {status:?}");

    // After death: start-time lookup fails (process gone) — exactly the
    // signal the takeover loop keys on.
    assert!(
        common::gate_watchdog::process_start_time_secs(pid).is_none(),
        "child pid {pid} still inspectable after exit"
    );

    // And the gate is acquirable again within a poll interval of the death.
    let deadline = Instant::now() + Duration::from_secs(2);
    let reacquired = loop {
        if gate_free() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(
        reacquired,
        "gate not acquirable within one poll interval after holder death"
    );
}

#[test]
fn gate_live_holder_is_never_falsely_stale() {
    // Acceptance item 2 (no false takeover): a LIVE holder must present a
    // matching start time — the property that keeps the waiter waiting
    // (for a CPU-busy proptest holder) instead of stealing the lock.
    // We hold the lock ourselves from this test thread, record ourselves,
    // and verify the liveness probe reports us alive-and-matching.
    if !gate_free() {
        eprintln!("SKIP: interpreter gate currently held by a live suite");
        return;
    }

    let mut holder = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path())
        .expect("open gate lock");
    assert!(
        unsafe { libc::flock(holder.as_fd().as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 },
        "gate was free but flock failed"
    );
    let record = format!(
        "AP_GATE_HOLDER pid={} start={}\n",
        std::process::id(),
        common::gate_watchdog::process_start_time_secs(std::process::id()).unwrap_or(0)
    );
    let _ = holder.set_len(0);
    let _ = holder.seek(SeekFrom::Start(0));
    let _ = holder.write_all(record.as_bytes());
    let _ = holder.flush();

    let (pid, start) = read_record().expect("record present");
    assert_eq!(pid, std::process::id());
    assert_eq!(
        common::gate_watchdog::process_start_time_secs(pid),
        Some(start),
        "live holder must present a matching start time (no false-stale)"
    );

    unsafe { libc::flock(holder.as_fd().as_raw_fd(), libc::LOCK_UN) };
}

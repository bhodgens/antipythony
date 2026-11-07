// Manual smoke harness for the AP_GATE_WAIT_SECS expiry panic (leaf
// acceptance: suite-level deadline safety net). Run:
//   1. start fake holder:  python3.14 or any process holding the gate:
//      use tests/common/gate_watchdog.rs helpers via this binary in child mode
//   2. run a tiny waiter with AP_GATE_WAIT_SECS=1 -> must panic with the
//      actionable message within ~1s, NOT block forever.
// Standalone test target so it never disturbs the real suites.
mod common;

use std::os::fd::AsFd as _;
use std::os::fd::AsRawFd as _;

#[test]
fn gate_wait_expiry_panics_with_actionable_message() {
    let path = std::env::temp_dir().join("aprun_gate_global.lock");

    // Waiter child mode: call acquire() expecting the actionable panic.
    // (Checked FIRST — the parent holds the gate, so a gate-free probe
    // would wrongly skip this branch.)
    if std::env::var("AP_GATE_WAITER").as_deref() == Ok("1") {
        std::panic::set_hook(Box::new(|info| {
            let msg = info.to_string();
            eprintln!("WAITER-PANIC: {msg}");
            assert!(
                msg.contains("interpreter gate held by pid") && msg.contains("kill it and re-run"),
                "panic message not actionable: {msg}"
            );
            std::process::exit(7); // distinct exit code = correct actionable panic
        }));
        common::interpreter_gate();
        // Must never reach here while the parent still holds the gate.
        eprintln!("WAITER-ACQUIRED-UNEXPECTEDLY");
        std::process::exit(8);
    }

    // Parent mode: only run when the real gate is free (never contend
    // with a live suite).
    let probe = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .expect("open");
    let free = unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if free {
        unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) };
    } else {
        eprintln!("SKIP: gate held by live suite");
        return;
    }
    drop(probe);

    let mut holder = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .expect("open holder");
    assert!(
        unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "gate was free but flock failed"
    );
    let record = format!(
        "AP_GATE_HOLDER pid={} start={}\n",
        std::process::id(),
        common::gate_watchdog::process_start_time_secs(std::process::id()).unwrap_or(0)
    );
    use std::io::{Seek, SeekFrom, Write as _};
    let _ = holder.set_len(0);
    let _ = holder.seek(SeekFrom::Start(0));
    let _ = holder.write_all(record.as_bytes());
    let _ = holder.flush();

    let self_exe = std::env::current_exe().expect("current_exe");
    let mut waiter = std::process::Command::new(&self_exe)
        .args(["--exact", "gate_wait_expiry_panics_with_actionable_message"])
        .env("AP_GATE_WAITER", "1")
        .env("AP_GATE_WAIT_SECS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn waiter");
    let status = waiter.wait().expect("wait waiter");
    assert_eq!(
        status.code(),
        Some(7),
        "waiter must exit 7 (actionable panic at deadline); got {status:?}"
    );

    // Release our hold.
    unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_UN) };
}

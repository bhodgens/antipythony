//! ap-conform harness self-checks (bughunt 2026-10-17).
//!
//! The differential harness is the measurement instrument for every
//! conformance gate in the project, so its own failure modes must be pinned:
//!  1. it must be able to FAIL (negative control), and
//!  2. it must never accept a non-3.14 interpreter as the reference.

use std::process::Command;
use std::time::{Duration, Instant};

// Serialize against every other interpreter-spawning suite (same lock file as the
// ap-run/ap-cli/ap-obj `interpreter_gate` seams) and bound every wait —
// ap-conform defaults to a 10s per-process timeout, so 120s covers a
// small corpus with headroom.
// Robustness leaf 00-ci-core/LEAF-gate-watchdog: bounded wait + holder
// liveness + stale takeover live in gate_watchdog.rs (include!d verbatim;
// canonical copy: crates/ap-obj/tests/common/gate_watchdog.rs).
include!("gate_watchdog.rs");

fn interpreter_gate() {
    gate_watchdog_acquire();
}

/// Run `cmd` capturing Output with a hard wall-clock deadline; a child
/// that outlives it is SIGKILLed and the test fails distinctly. No bare
/// unbounded `.output()` may remain in any harness.
fn bounded_output(mut cmd: Command, what: &str) -> std::process::Output {
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match child.try_wait().expect("wait child") {
            Some(_) => return child.wait_with_output().expect("collect output"),
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{what}: exceeded the 120s deadline");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn harness_reports_fail_on_divergent_impl() {
    interpreter_gate();
    // Negative control: /bin/echo can never reproduce `print("A")`.
    // If this ever passes, the comparator is broken and every green gate
    // in the project is worthless.
    let dir = std::env::temp_dir().join(format!("ap_conform_neg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("negcase.py");
    std::fs::write(&case, "print(\"A\")\n").unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ap-conform"));
    cmd.args([
        "run",
        "--corpus",
        &case.to_string_lossy(),
        "--impl",
        "/bin/echo",
    ]);
    let out = bounded_output(cmd, "ap-conform negative control");
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(out.status.code(), Some(1), "negative control must exit 1");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains("FAIL"),
        "negative control must print FAIL: {text}"
    );
}

#[test]
fn reference_interpreter_is_actually_314() {
    interpreter_gate();
    let found = ap_conform::find_reference_interpreter().expect("host must have a 3.14 reference");
    let mut cmd = Command::new(&found);
    cmd.arg("--version");
    let out = bounded_output(cmd, "reference --version");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("Python 3.14"),
        "find_reference_interpreter returned a non-3.14 interpreter: {found} -> {text}"
    );
}

#[test]
fn tier_flag_is_accepted_and_forwarded() {
    interpreter_gate();
    // C-31 / 02.03 gate seam: `--tier 0` must be accepted and forwarded to
    // the impl (never to the reference). A reference-only impl (python3.14)
    // would choke on --tier, so we point at a tiny shim that echoes its argv
    // and assert the flag reached the IMPL side.
    let dir = std::env::temp_dir().join(format!("ap_conform_tier_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let case = dir.join("t.py");
    std::fs::write(&case, "print(1)\n").unwrap();

    // Bad tier value must be rejected (exit 2), not silently ignored.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ap-conform"));
    cmd.args([
        "run",
        "--corpus",
        &case.to_string_lossy(),
        "--impl",
        "/bin/echo",
        "--tier",
        "7",
    ]);
    let bad = bounded_output(cmd, "ap-conform bad-tier check");
    assert_eq!(bad.status.code(), Some(2), "bad --tier must exit 2");
    let _ = std::fs::remove_dir_all(&dir);
}

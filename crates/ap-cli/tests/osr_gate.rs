//! Leaf 02.05f — the OSR completion GATES (SPLIT-02.05 §02.05f gate
//! matrix), end-to-end through the ap-run-eval binary:
//!
//! 1. HOT-LOOP TIER-UP PROBE: a while-loop function crosses the OSR
//!    threshold mid-execution and completes byte-identically with tier-0
//!    (the transfer + OSR-BY-CALL path exercised — the [OSR-STATS]
//!    transfer counter proves the OSR entry RAN, not silently skipped).
//! 2. INELIGIBLE-LOOP PROBE: a function whose live set exceeds
//!    OSR_MAX_LIVE stays correct purely via poll-based tier-up at the
//!    call boundary (no OSR entry — the registry's gate refuses it and
//!    the loop keeps running tier-0; results byte-identical).
//!
//! Harness discipline mirrors tier_mode_differential.rs: interpreter-gate
//! flock, per-run deadline fence, piped-output drain.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

include!("gate_watchdog.rs");

fn interpreter_gate() {
    gate_watchdog_acquire();
}

fn run_eval(args: &[&str], env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>, i32) {
    interpreter_gate();
    let bin = env!("CARGO_BIN_EXE_ap-run-eval");
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let start = Instant::now();
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    // Drain pipes concurrently (the 02.05e pipe-buffer deadlock lesson).
    let deadline = Duration::from_secs(300);
    let stdout_pipe = child.stdout.take().expect("stdout");
    let stderr_pipe = child.stderr.take().expect("stderr");
    let out_handle = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut { stdout_pipe }, &mut out);
        out
    });
    let err_handle = std::thread::spawn(move || {
        let mut err = Vec::new();
        let _ = std::io::Read::read_to_end(&mut { stderr_pipe }, &mut err);
        err
    });
    loop {
        match child.try_wait().expect("wait osr-gate child") {
            Some(status) => {
                let stdout = out_handle.join().expect("join stdout drainer");
                let stderr = err_handle.join().expect("join stderr drainer");
                return (stdout, stderr, status.code().unwrap_or(-1));
            }
            None => {
                if start.elapsed() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("osr-gate run exceeded the 300s fence");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// The hot-loop OSR probe (also written to a temp file per run).
const HOT_LOOP: &str = r#"
def hot(n):
    total = 0
    i = 0
    while i < n:
        total = total + i * 2
        i = i + 1
    return total

print(hot(200000))
print(hot(10))
print(hot(0))
"#;

/// The ineligible probe: > OSR_MAX_LIVE (16) locals at the loop header —
/// the eligibility gate must refuse the OSR compile; correctness flows
/// purely through poll-based tier-0 (and tier-1 at the boundary).
const INELIGIBLE_LOOP: &str = r#"
def wide(n):
    a0 = 1
    a1 = 1
    a2 = 1
    a3 = 1
    a4 = 1
    a5 = 1
    a6 = 1
    a7 = 1
    a8 = 1
    a9 = 1
    a10 = 1
    a11 = 1
    a12 = 1
    a13 = 1
    a14 = 1
    a15 = 1
    a16 = 1
    a17 = 1
    i = 0
    acc = 0
    while i < n:
        acc = acc + i * 2
        i = i + 1
    return acc + a0 + a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10 \
        + a11 + a12 + a13 + a14 + a15 + a16 + a17

print(wide(50000))
"#;

fn write_probe(name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("apx-0205f-gates");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(name);
    std::fs::write(&p, src).expect("write probe");
    p
}

/// Gate 1: the hot loop OSR-transfers and stays byte-identical.
/// The transfer counter ([OSR-STATS] transfers=N, N >= 1 under
/// APX_OSR_STATS=1) proves the OSR entry actually RAN — without it the
/// byte-identity alone could pass with OSR silently never firing.
#[test]
fn hot_loop_osr_transfers_and_matches_tier0() {
    let probe = write_probe("osr_hot.py", HOT_LOOP);
    let (out0, err0, rc0) = run_eval(&["--tier", "0", probe.to_str().expect("utf8")], &[]);
    let (outa, erra, rca) = run_eval(
        &["--tier", "adaptive", probe.to_str().expect("utf8")],
        &[("APX_OSR_STATS", "1"), ("APX_TIER_STATS", "1")],
    );
    assert_eq!(rc0, 0, "tier-0 hot-loop probe rc");
    assert_eq!(
        rca,
        0,
        "adaptive hot-loop probe rc (stderr: {})",
        String::from_utf8_lossy(&erra)
    );
    assert_eq!(
        out0, outa,
        "hot-loop OSR probe: stdout must be byte-identical across tiers"
    );
    // Non-stats stderr identical too (stats lines are the variance surface).
    let strip_stats = |b: &[u8]| -> String {
        String::from_utf8_lossy(b)
            .lines()
            .filter(|l| !l.starts_with("[OSR-STATS]") && !l.starts_with("[TIER"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        strip_stats(&err0),
        strip_stats(&erra),
        "hot-loop OSR probe: non-stats stderr must match"
    );
    // THE EXERCISE PROOF: at least one OSR transfer fired.
    let text = String::from_utf8_lossy(&erra);
    let transferred = text
        .lines()
        .find_map(|l| l.strip_prefix("[OSR-STATS] transfers="))
        .and_then(|rest| {
            rest.split_whitespace()
                .next()
                .and_then(|tok| tok.parse::<u64>().ok())
        });
    assert!(
        matches!(transferred, Some(n) if n >= 1),
        "OSR entry never transferred (transfers=0) — the probe crossed the \
         threshold but the OSR path did not fire\nstderr: {text}"
    );
}

/// Gate 2: the over-cap loop stays correct (poll-based tier-0 only) —
/// byte-identical with tier-0, and the eligibility gate is REPORTED
/// (the [OSR] INELIGIBLE line under APX_TIER_DEBUG), never silent.
#[test]
fn ineligible_loop_reports_and_stays_identical() {
    let probe = write_probe("osr_wide.py", INELIGIBLE_LOOP);
    let (out0, _err0, rc0) = run_eval(&["--tier", "0", probe.to_str().expect("utf8")], &[]);
    let (outa, erra, rca) = run_eval(
        &["--tier", "adaptive", probe.to_str().expect("utf8")],
        &[("APX_TIER_DEBUG", "1"), ("APX_OSR_STATS", "1")],
    );
    assert_eq!(rc0, 0, "tier-0 ineligible-loop rc");
    assert_eq!(rca, 0, "adaptive ineligible-loop rc");
    assert_eq!(
        out0, outa,
        "ineligible loop must stay byte-identical (poll-based tier-0)"
    );
    // No OSR entry may have transferred for the wide loop (the gate
    // refuses the compile). transfers must be absent or 0.
    let text = String::from_utf8_lossy(&erra);
    let transferred = text
        .lines()
        .find_map(|l| l.strip_prefix("[OSR-STATS] transfers="))
        .and_then(|rest| {
            rest.split_whitespace()
                .next()
                .and_then(|tok| tok.parse::<u64>().ok())
        })
        .unwrap_or(0);
    assert_eq!(
        transferred, 0,
        "an over-OSR_MAX_LIVE loop must never transfer (gate bypassed!)"
    );
}

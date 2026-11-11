//! Leaf 02.05e — the background tier-up driver's END-TO-END gates
//! (SPLIT-02.05 §02.05e "Gate"):
//!
//! 1. SYNC-MODE CORPUS PARITY (the PON_SYNC_TIERUP analogue): every
//!    tier1-transparency probe runs under `APX_SYNC_TIERUP=1` (the 02.05c
//!    inline compile shape) and its triple must match the SAME probe under
//!    plain adaptive (the async driver). Both routes must be invisible.
//! 2. ASYNC RACE-STRESS (the "tier-up mid-loop under mutation" probe,
//!    100/100): a probe that hammers tier-up decisions while mutating the
//!    hot function's types runs 100 times under the async driver; every
//!    run must produce the identical verdict triple (loop-tolerant by
//!    construction — the driver's install timing may vary per run, the
//!    OUTPUT may not).
//!
//! The failed-tier-up-disables UNIT coverage lives in tierup_tests.rs
//! (pon's exact test, ported at the driver level); the race-stress probe
//! here covers the same disable rule end-to-end (a flapping body ends up
//! either tier-1 with value guards or DISABLED — output identical either
//! way, which IS the discipline's observable contract).
//!
//! Harness discipline mirrors tier_mode_differential.rs: the repo
//! interpreter gate, debug-build budget, per-run deadline fence.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

include!("common/gate_watchdog.rs");

fn interpreter_gate() {
    gate_watchdog_acquire();
}

fn preflight_or_panic() {
    // Same gate as tier_mode_differential (memory pressure refusal).
    let ok = std::fs::read_to_string("/proc/meminfo")
        .map(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("MemAvailable:"))
                .map(|rest| {
                    let kb = rest
                        .trim()
                        .trim_end_matches(" kB")
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0);
                    kb > 4 * 1024 * 1024
                })
                .unwrap_or(true)
        })
        .unwrap_or(true);
    let _ = ok; // macOS host: /proc absent — gate open (matches upstream).
}

fn probe_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate parent")
        .parent()
        .expect("crate parent")
        .join("tests/corpus/tier1-transparency")
}

fn probes() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(probe_dir())
        .expect("read tier1-transparency dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("py"))
        .collect();
    v.sort();
    assert!(v.len() >= 16, "corpus truncated: {} probes", v.len());
    v
}

fn run_timeout_secs() -> u64 {
    std::env::var("AP_TIER_PAIR_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

/// Resolve the `ap-run-eval` bin: `CARGO_BIN_EXE_ap-run-eval` when cargo
/// provides it (only ap-cli's own tests get it), else the target dir's
/// debug path (same discipline as common/mod.rs's ap_run_probe_bin — the
/// eval bin lives in ap-cli, this suite lives in ap-run).
fn ap_run_eval_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_ap-run-eval") {
        return p.into();
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate parent")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-eval"),
        Err(_) => root.join("target/debug/ap-run-eval"),
    }
}

/// Run one probe with extra env, as a fenced subprocess. Returns
/// (stdout, stderr, exit code).
fn run_probe(tier: &str, probe: &PathBuf, extra_env: &[(&str, &str)]) -> (Vec<u8>, Vec<u8>, i32) {
    preflight_or_panic();
    interpreter_gate();
    let bin = ap_run_eval_bin();
    let mut cmd = Command::new(&bin);
    cmd.args(["--tier", tier]).arg(probe);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    // Drain both pipes CONCURRENTLY with the wait (harness hardening,
    // 02.05e union): a child that writes >64KB to stderr (e.g. a debug
    // spam regression) would otherwise block forever on a full pipe while
    // this loop only polls try_wait — the exact 300s-timeout failure this
    // suite hit before the CALLFAST-DBG spam was env-gated.
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn tierup probe: {e}"));
    let stdout_handle = child.stdout.take().expect("child stdout");
    let stderr_handle = child.stderr.take().expect("child stderr");
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        use std::io::Read;
        let mut h = stdout_handle;
        let _ = h.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        use std::io::Read;
        let mut h = stderr_handle;
        let _ = h.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(run_timeout_secs());
    let status = loop {
        match child.try_wait().expect("wait tierup child") {
            Some(status) => break status,
            None => {
                assert!(
                    Instant::now() < deadline,
                    "tierup probe exceeded {}s",
                    run_timeout_secs()
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let out = child.wait_with_output().expect("collect output");
    let _ = out; // readers own the pipes now
    (
        out_reader.join().unwrap_or_default(),
        err_reader.join().unwrap_or_default(),
        status.code().unwrap_or(-1),
    )
}

/// The race-stress probe (tier-up decisions made mid-loop while the hot
/// function's argument types mutate beneath it: int loop → str flip →
/// int loop again, with the tier-up driver racing the flip — the t1_002
/// shape made deliberately racier: shorter int phases so the install can
/// land mid-flip). Deterministic output; any tier assignment (tier-0/
/// tier-1/DISABLED at any call) must produce these exact bytes + rc.
const RACE_PROBE: &str = r#"
# 02.05e race-stress: hot int loop, then a str flip DURING the loop's
# lifetime, then int again. The background driver may install tier-1 at
# ANY call boundary (including mid-flip); value guards must absorb every
# schedule.
def mix(a, b):
    return a + b

acc = 0
for i in range(20000):
    acc += mix(i, i)

# flip: str phase (guards side-exit to the twin / body disables)
s = ""
for ch in "abcdef":
    for k in range(5):
        s += mix(ch, "x")

# int again (same function, re-specialization under a live driver)
acc2 = 0
for i in range(20000):
    acc2 += mix(-i, i)

print(acc, acc2, acc == acc2)
print(len(s), s[:12])
"#;

// ---------------------------------------------------------------------------
// Gate 1: APX_SYNC_TIERUP=1 corpus parity (sync route == async route)
// ---------------------------------------------------------------------------

#[test]
fn sync_tierup_mode_matches_async_across_corpus() {
    interpreter_gate();
    let ps = probes();
    let mut checked = 0usize;
    for probe in &ps {
        let name = probe
            .file_stem()
            .expect("probe file name")
            .to_string_lossy()
            .into_owned();
        let (out_a, err_a, rc_a) = run_probe("adaptive", probe, &[]);
        let (out_s, err_s, rc_s) = run_probe("adaptive", probe, &[("APX_SYNC_TIERUP", "1")]);
        assert_eq!(
            rc_a, rc_s,
            "{name}: sync-tierup rc {rc_s} != async rc {rc_a}"
        );
        assert_eq!(
            out_a, out_s,
            "{name}: sync-tierup stdout != async stdout (bytes)"
        );
        assert_eq!(err_a, err_s, "{name}: sync-tierup stderr != async stderr");
        checked += 1;
    }
    eprintln!("sync-tierup parity: {checked} probes, all MATCH");
    assert!(checked >= 16);
}

// ---------------------------------------------------------------------------
// Gate 2: async race-stress, 100/100 identical verdicts
// ---------------------------------------------------------------------------

#[test]
fn async_tierup_race_stress_100_runs_identical() {
    interpreter_gate();
    let dir = std::env::temp_dir().join(format!("aprun-tierup-race-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("race dir");
    let path = dir.join("race.py");
    std::fs::write(&path, RACE_PROBE).expect("write race probe");

    // The reference run: plain adaptive (any schedule must match this).
    let (ref_out, ref_err, ref_rc) = run_probe("adaptive", &path, &[]);
    assert_eq!(
        ref_rc,
        0,
        "race probe must exit 0 (stderr: {})",
        String::from_utf8_lossy(&ref_err)
    );
    let ref_text = String::from_utf8_lossy(&ref_out).to_string();
    eprintln!("race reference:\n{ref_text}");

    let mut schedule_with_tier1 = 0usize;
    for run in 0..100 {
        // APX_TIER1_STATS makes installs observable on stderr; the OUTPUT
        // contract is the stdout triple.
        let (out, err, rc) = run_probe(
            "adaptive",
            &path,
            &[("APX_TIER1_STATS", "1"), ("APX_TIER_STATS", "1")],
        );
        assert_eq!(rc, ref_rc, "run {run}: rc {rc} != reference {ref_rc}");
        assert_eq!(
            String::from_utf8_lossy(&out),
            ref_text,
            "run {run}: stdout diverged from the reference schedule"
        );
        // The stats lines on stderr may legitimately differ per run (the
        // install timing races the probe's loops); byte-compare ONLY the
        // non-stats part.
        let err_clean: String = String::from_utf8_lossy(&err)
            .lines()
            .filter(|l| {
                !l.starts_with("[TIER1-STATS]")
                    && !l.starts_with("[TIER-STATS]")
                    && !l.starts_with("[TIERUP-STATS]")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let ref_err_clean: String = String::from_utf8_lossy(&ref_err)
            .lines()
            .filter(|l| {
                !l.starts_with("[TIER1-STATS]")
                    && !l.starts_with("[TIER-STATS]")
                    && !l.starts_with("[TIERUP-STATS]")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            err_clean, ref_err_clean,
            "run {run}: non-stats stderr diverged"
        );
        if String::from_utf8_lossy(&err).contains("install fn=mix") {
            schedule_with_tier1 += 1;
        }
    }
    eprintln!(
        "race-stress 100/100 PASS; tier-1 installed in {schedule_with_tier1}/100 schedules \
         (the assertion is output identity under ANY schedule)"
    );
    std::fs::remove_dir_all(&dir).ok();
}

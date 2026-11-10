//! Leaf 02.05b — Risk-4 adversarial probe gate (docs/ic-version-audit.md).
//!
//! Runs the mutate-through-every-path matrix (tests/corpus/tier1-risk4/
//! risk4_matrix.py) under BOTH tier modes + APX_IC_AUDIT=1, asserting
//! the byte-expected verdict (`RISK4-OK …`) and rc 0. Any missed version
//! bump surfaces as a stale-wrong value inside the probe — fail-closed via
//! output divergence.
//!
//! Per-crate discipline: this file is a single test target; the env vars it
//! sets (APX_TIER / APX_IC_AUDIT) apply to its own subprocesses only (each
//! test file is a separate binary; no other suite reads these).

#![allow(dead_code)]
mod common;

const PROBE: &str = "tests/corpus/tier1-risk4/risk4_matrix.py";
const OK: &str = "RISK4-OK";

fn probe_path() -> std::path::PathBuf {
    // ap-run/tests/<this file> -> crates/ap-run -> crates -> workspace root.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf();
    root.join(PROBE)
}

fn run_mode(tier: &str, audit: bool) -> (Vec<u8>, i32) {
    std::env::set_var("APX_TIER", tier);
    if audit {
        std::env::set_var("APX_IC_AUDIT", "1");
    } else {
        std::env::remove_var("APX_IC_AUDIT");
    }
    let src = std::fs::read_to_string(probe_path()).expect("risk4 probe present");
    let (out, err, rc) = common::run_aprun_eval(&src);
    assert!(
        err.is_empty(),
        "[tier {tier} audit {audit}] stderr must be clean (C-29): {}",
        String::from_utf8_lossy(&err)
    );
    assert_eq!(rc, 0, "[tier {tier} audit {audit}] rc");
    (out, rc)
}

fn assert_ok(out: &[u8], tag: &str) {
    let text = String::from_utf8_lossy(out);
    assert!(
        text.contains(OK),
        "[{tag}] expected {OK} verdict, got: {text}"
    );
}

/// Tier 0 (tree-walker): the mutation paths' baseline correctness.
#[test]
fn risk4_matrix_tier0() {
    let (out, _) = run_mode("0", false);
    assert_ok(&out, "tier0");
}

/// Tier jit: the IC-populated route must be observation-invisible.
#[test]
fn risk4_matrix_tier_jit() {
    let (out, _) = run_mode("jit", false);
    assert_ok(&out, "tierjit");
}

/// APX_IC_AUDIT=1 cross-check under the JIT route (audit rule 3).
#[test]
fn risk4_matrix_tier_jit_audit() {
    let (out, _) = run_mode("jit", true);
    assert_ok(&out, "tierjit-audit");
}

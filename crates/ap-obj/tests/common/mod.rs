//! Shared CPython 3.14 reference-interpreter discovery for the Rust
//! differential suites (robustness-ci leaf 00.02, review findings #3/#4).
//!
//! Resolution ladder (plans/robustness-ci/master.md "Reference python
//! discovery" contract):
//!   1. `AP_CONFORM_REFERENCE_PYTHON` — accepted ONLY if its `--version`
//!      reports Python 3.14.
//!   2. PATH `python3.14` — same version-string check (mirrors the logic at
//!      `crates/ap-conform/src/lib.rs:90-117`).
//! Deliberately NO bare `python3` fallback and no hardcoded Homebrew path:
//! tests/README.md rule 5 says anything that runs "CPython" must require a
//! real Python 3.14 via the `--version` string check.
//!
//! Skip policy (master.md "Skip policy" contract): when no reference is
//! found, `AP_REQUIRE_REFERENCE=1` (set in every CI job that runs
//! differential tests) turns the run into a hard failure — a runner without
//! python3.14 must go red, not report green (finding #3). Without the env
//! var, suites skip with a visible `SKIP:` line so a local box lacking the
//! reference stays green instead of red.
//!
//! Duplication note: Cargo integration tests under `tests/` are separate
//! crates, so this helper is provided as a per-package `tests/common/mod.rs`
//! (declared with `mod common;` in each suite). No new path-dep crate was
//! added; if ap-obj/ap-cli suites are converted later they copy this file.

#![allow(dead_code)] // each suite target compiles the subset it uses

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

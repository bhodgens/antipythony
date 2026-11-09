//! Shared CPython 3.14 reference-interpreter discovery for the Rust
//! differential suites (robustness-ci leaf 00.02, review findings #3/#4).
//!
//! Resolution ladder (plans/robustness-ci/master.md "Reference python
//! discovery" contract):
//!   1. `AP_CONFORM_REFERENCE_PYTHON` — accepted ONLY if its `--version`
//!      reports Python 3.14.
//!   2. PATH `python3.14` — same version-string check (mirrors the logic at
//!      `crates/ap-conform/src/lib.rs:90-117`).
//!      Deliberately NO bare `python3` fallback and no hardcoded Homebrew path:
//!      tests/README.md rule 5 says anything that runs "CPython" must require a
//!      real Python 3.14 via the `--version` string check.
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

pub mod gate_watchdog;

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
// 2026-11-06 harness review: resource fencing for the CPython-reference
// probes this crate's suites spawn (ref_batch in repr_fuzz.rs, cpython()
// in cpython_differential.rs). Reference side only (megabytes), but the
// SAME single-tenancy + memory-pressure preflight + deadline discipline
// applies: no unbounded waits, no overlap with other interpreter-spawning
// suites, no probe start while the box is saturated.
// ---------------------------------------------------------------------------

/// Exclusive whole-invocation single-tenancy lock (same lock file + seam as
/// crates/ap-run + crates/ap-cli `interpreter_gate`). Taken once per
/// test-binary process, held to exit; cheap (one atomic load) after first.
///
/// Robustness (robustness-ci leaf 00-ci-core/LEAF-gate-watchdog): bounded
/// wait + holder-liveness record + stale-holder takeover live in
/// `gate_watchdog.rs`; a wedged holder no longer poisons the suite fleet
/// (see that file's header for the full contract).
pub fn interpreter_gate() {
    gate_watchdog::gate_watchdog_acquire();
}

/// True when the machine is NOT under critical memory pressure (same
/// metrics + open-on-unreadable policy as ap-run's common::memory_pressure_ok).
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
    level <= 2
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
/// critical memory pressure.
pub fn preflight_or_panic() {
    assert!(
        memory_pressure_ok(),
        "memory-pressure preflight: machine under critical memory pressure; \
         reference-interpreter probe REFUSED to start — free the machine and rerun"
    );
}

/// Wall-clock deadline for one reference probe. Env-overridable, never
/// unbounded: `ref_batch`/`cpython` are megabyte-scale CPython runs, so
/// 60s is generous headroom (2026-11-06 harness review: no bare
/// wait_with_output() may remain in any harness).
pub fn reference_deadline() -> std::time::Duration {
    let secs: u64 = std::env::var("AP_REF_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    std::time::Duration::from_secs(secs)
}

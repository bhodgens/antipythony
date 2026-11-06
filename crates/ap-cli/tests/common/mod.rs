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
// 2026-11-06 OOM mitigation (2+3+4): cross-process fence + single-tenancy
// gate, copied from crates/ap-run/tests/common/mod.rs (Cargo integration
// tests are separate crates; per the duplication note above, no shared
// path-dep crate exists). Same lock files, same env knobs.
// ---------------------------------------------------------------------------

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
pub fn interpreter_gate() {
    static DONE: AtomicU64 = AtomicU64::new(0);
    if DONE.swap(1, Ordering::SeqCst) == 1 {
        return;
    }
    let path = std::env::temp_dir().join("aprun_gate_global.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .expect("open interpreter-gate lock file");
    let rc = unsafe { libc::flock(file.as_fd().as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(rc, 0, "flock on interpreter-gate lock failed");
    // Deliberately leak `file` (no Drop): the lock must live until process
    // exit. One File per test-binary process, bounded.
    std::mem::forget(file);
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
            "kern.memorystatus_vm_pressure_level\0".as_ptr() as *const libc::c_char,
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

/// Resident footprint of `pid` in BYTES. macOS: proc_pid_rusage
/// ri_phys_footprint (jetsam's own metric). Linux: /proc/<pid>/status VmRSS.
/// `None` = unreadable - the watchdog counts consecutive failures and kills
/// the child (fail closed). Mirrors crates/ap-run/tests/common/mod.rs.
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

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
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    match std::env::var("CARGO_TARGET_DIR") {
        Ok(d) => std::path::Path::new(&d).join("debug/ap-run-probe"),
        Err(_) => root.join("target/debug/ap-run-probe"),
    }
}

/// Run one probe through the ap-run interpreter as a SUBPROCESS.
/// Returns (stdout, stderr, exit code).
///
/// Per-probe subprocess = per-probe heap teardown; no accumulation across
/// the suite's probes. The lock serializes heavy probes across processes.
///
/// Hard fence (2026-11-05 OOM mitigation): each probe gets a wall-clock
/// deadline (default 240s, overridable with AP_PROBE_TIMEOUT_SECS) and a
/// resident-memory cap (default 6 GB, overridable with AP_PROBE_MAX_RSS_GB;
/// 0 disables). One unittest probe was observed growing to ~44 GB RSS on
/// this 64 GB box before dying; with the fence a pathological probe is
/// SIGKILLed at the cap, the suite's assert fails with a distinctive
/// message, and unrelated processes on the box survive.
pub fn run_aprun_eval(src: &str) -> (Vec<u8>, Vec<u8>, i32) {
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

    // Watchdog: poll exit status + RSS every 250ms. RSS comes from `ps -o
    // rss=` (KB on macOS AND Linux). A pathological probe is killed at the
    // cap; a hung one at the deadline.
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
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
                    if let Ok(text) = String::from_utf8(rss_kb_of(pid)) {
                        if let Ok(kb) = text.trim().parse::<u64>() {
                            if kb.saturating_mul(1024) > rss_cap_bytes {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Err(format!(
                                    "memory: RSS {kb}KB exceeded the \
                                     {}GB fence (AP_PROBE_MAX_RSS_GB); this probe's \
                                     interpreter path is growing unbounded",
                                    rss_cap_bytes / (1024 * 1024 * 1024)
                                ));
                            }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    match outcome {
        Ok(triple) => triple,
        Err(why) => panic!("ap-run-probe killed by fence: {why}"),
    }
}

/// Resident set size of `pid` in KB, via `ps -o rss=` (macOS + Linux).
fn rss_kb_of(pid: u32) -> Vec<u8> {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default()
}

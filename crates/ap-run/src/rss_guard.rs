//! In-process RSS self-guard for every antipythony interpreter process.
//!
//! 2026-11-06 OOM campaign, mitigation 1: macOS has no effective OOM killer
//! for foreground processes. When anonymous memory plus the compressor
//! saturate, the machine stalls until forced restart (7 jetsam events on
//! 2026-11-05 named antipythony-family processes at 74-242 GB recorded
//! footprint). The external harness fences (tests/common/mod.rs) only cover
//! repo test suites; ad-hoc /tmp gate binaries (w112_bin, t1gate-bin,
//! cont-bin) spawn the interpreter with no fence at all. An in-process guard
//! covers every spawn path.
//!
//! Behavior:
//! - Background monitor thread samples the process footprint every 100 ms.
//! - Over the cap, prints one diagnostic line to stderr and aborts (SIGABRT,
//!   core-dumpable), mirroring the harness fence's kill semantics.
//! - Cap resolution order: AP_MAX_RSS_GB env var; else 25% of physical RAM
//!   (min 2 GB). `AP_MAX_RSS_GB=0` disables the guard entirely (CI escape
//!   hatch; the external harness fence still applies there).
//!
//! The footprint metric is `ri_phys_footprint` from `proc_pid_rusage` — the
//! same metric jetsam uses. A syscall per sample, no subprocess, so the
//! guard cannot be disabled by memory pressure the way the `ps`-spawn poll
//! in the test fence can (the 17:58/18:01 fence-overshoot events).

#![cfg(unix)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Sample interval, ms. The fastest observed leak rates (~2 GB/s in the
/// clone-leak class) gain at most ~200 MB per tick — well inside the cap's
/// headroom.
const SAMPLE_INTERVAL_MS: u64 = 100;

static GUARD_STARTED: AtomicBool = AtomicBool::new(false);

/// Cap in GB as configured (0 = disabled), kept for tests/diagnostics.
static CAP_GB: AtomicU64 = AtomicU64::new(0);

/// Resolve the cap in bytes: `AP_MAX_RSS_GB` if set, else 25% of physical
/// RAM (min 2 GB). `Ok(0)` means the guard is disabled.
fn resolve_cap_bytes() -> Result<u64, String> {
    if let Ok(v) = std::env::var("AP_MAX_RSS_GB") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            match v.parse::<u64>() {
                Ok(gb) => return Ok(gb.saturating_mul(1024 * 1024 * 1024)),
                Err(_) => {
                    // FAIL CLOSED: an invalid override must not disable the
                    // guard (a typo'd AP_MAX_RSS_GB otherwise reads as
                    // "unbounded"). Warn loudly and fall through to the RAM
                    // default instead.
                    eprintln!(
                        "antipythony-rss-guard: AP_MAX_RSS_GB: invalid value {v:?} \
                         (want a GB count, e.g. 6); using the RAM-default cap"
                    );
                }
            }
        }
    }
    // Default: 25% of physical RAM, min 2 GB. On this 64 GB box that is
    // 16 GB — comfortably above the legitimate debug-build scrapy-chain
    // footprint (~0.9-1.4 GB) while leaving the OS + other apps half the
    // machine.
    let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if pages <= 0 || page_size <= 0 {
        // Cannot size the machine: fail CLOSED with a conservative 4 GB so a
        // config-syscall failure never means "unbounded".
        return Ok(4 * 1024 * 1024 * 1024);
    }
    let total = pages as u64 * page_size as u64;
    Ok((total / 4).max(2 * 1024 * 1024 * 1024))
}

/// Current physical footprint in bytes (jetsam's metric). `None` = syscall
/// failed; the monitor treats repeated failure as over-cap (fail closed).
#[cfg(target_os = "macos")]
fn current_footprint_bytes() -> Option<u64> {
    unsafe {
        let mut info: libc::rusage_info_v0 = std::mem::zeroed();
        let rc = libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V0,
            &mut info as *mut _ as *mut libc::rusage_info_t,
        );
        if rc != 0 {
            return None;
        }
        Some(info.ri_phys_footprint)
    }
}

/// Non-macOS unix fallback: read VmRSS from /proc/self/status (Linux).
#[cfg(not(target_os = "macos"))]
fn current_footprint_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.trim().trim_end_matches(" kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

/// Start the guard. Idempotent; safe to call from every bin's `main`.
/// Returns the resolved cap in GB (0 = disabled) for callers that log it.
pub fn start() -> u64 {
    if GUARD_STARTED.swap(true, Ordering::SeqCst) {
        return CAP_GB.load(Ordering::SeqCst);
    }
    // A3 (memperf W1): snapshot the APX_* debug flags once, up front, so the
    // ~90 per-op probe sites read a cached bitset instead of scanning the
    // environment on every raise/binop/attr/GC cycle.
    ap_obj::debug_flags::snapshot();
    // resolve_cap_bytes is infallible: explicit env wins, invalid env warns
    // and falls back, sysconf failure fails closed at 4 GB.
    let cap_bytes = resolve_cap_bytes().unwrap_or_else(|_| 4 * 1024 * 1024 * 1024);
    CAP_GB.store(cap_bytes / (1024 * 1024 * 1024), Ordering::SeqCst);
    if cap_bytes == 0 {
        return 0; // AP_MAX_RSS_GB=0: explicit opt-out
    }
    let cap = cap_bytes;
    std::thread::Builder::new()
        .name("rss-guard".into())
        .stack_size(64 * 1024)
        .spawn(move || monitor(cap))
        .expect("spawn rss-guard thread");
    cap_bytes / (1024 * 1024 * 1024)
}

/// The monitor loop: sample, compare, abort.
fn monitor(cap_bytes: u64) {
    let mut fail_count: u32 = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(SAMPLE_INTERVAL_MS));
        match current_footprint_bytes() {
            Some(bytes) => {
                fail_count = 0;
                if bytes > cap_bytes {
                    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    let cap_gb = cap_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                    eprintln!(
                        "\nantipythony-rss-guard: ABORT: footprint {:.2} GB exceeded cap {:.2} GB \
                         (AP_MAX_RSS_GB, default = 25% of RAM); the interpreter path running this \
                         script grows unbounded — file this as a runtime bug, do not just raise \
                         the cap",
                        gb, cap_gb
                    );
                    // Abort, not exit: SIGABRT lands in crash reports with a
                    // native stack, so the leak class is debuggable after the
                    // fact.
                    std::process::abort();
                }
            }
            None => {
                fail_count += 1;
                // Fail closed: 20 consecutive sample failures (2 s) mean the
                // guard is blind, and a blind guard must not read as "safe".
                if fail_count >= 20 {
                    eprintln!(
                        "\nantipythony-rss-guard: ABORT: footprint syscall failed {} times \
                         consecutively; refusing to run unguarded",
                        fail_count
                    );
                    std::process::abort();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_resolution_env_and_defaults() {
        // Explicit env var wins.
        std::env::set_var("AP_MAX_RSS_GB", "6");
        assert_eq!(resolve_cap_bytes().unwrap(), 6 * 1024 * 1024 * 1024);
        std::env::set_var("AP_MAX_RSS_GB", "0");
        assert_eq!(resolve_cap_bytes().unwrap(), 0);
        // Invalid value: fail CLOSED -> falls back to the RAM default (never 0).
        std::env::set_var("AP_MAX_RSS_GB", "banana");
        let d = resolve_cap_bytes().unwrap();
        assert!(d >= 2 * 1024 * 1024 * 1024, "invalid env must not disable");
        std::env::remove_var("AP_MAX_RSS_GB");
        // Default: 25% of RAM, min 2 GB. On any real host the default sits
        // between those bounds.
        let d = resolve_cap_bytes().unwrap();
        assert!(d >= 2 * 1024 * 1024 * 1024);
        // 25% of RAM must also be <= total (sanity on huge/min machines).
        assert!(d <= u64::MAX / 2);
    }

    #[test]
    fn footprint_syscall_works() {
        // The guard must be able to see the test process itself.
        let fp = current_footprint_bytes().expect("footprint syscall");
        assert!(fp > 0, "footprint must be positive");
    }
}

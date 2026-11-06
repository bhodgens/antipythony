//! `ap-run-probe` — per-probe executor for the ap-run differential suites.
//!
//! Memory-OOM mitigation (2026-11-05): the differential suites previously
//! called `ap_run::run_module` IN-PROCESS inside the test binary, so the
//! tracing-GC heap accumulated across a suite's probes (the gc suite alone
//! reached ~10 GB RSS in a debug build). Suites now spawn this bin once per
//! probe via `CARGO_BIN_EXE_ap-run-probe` (cargo builds it before running
//! the suite's integration tests, so it is bin-fresh by construction); each
//! probe's interpreter heap dies with its process.
//!
//! Output contract mirrors `ap-run-eval` (crates/ap-cli): parse -> lower ->
//! tier-0 execute the file given as the sole argument; stdout/stderr pass
//! through; the process exit code is the module outcome's exit code.
//! Suites capture both streams and compare against CPython byte-exactly.

use std::io::Write;

fn main() {
    // 2026-11-06 OOM mitigation: in-process RSS self-guard (see
    // ap_run::rss_guard). AP_MAX_RSS_GB overrides; default = 25% of RAM.
    ap_run::rss_guard::start();
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: ap-run-probe <probe.py>");
            std::process::exit(2);
        }
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ap-run-probe: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let script_path = path.clone();

    // Same rationale as ap-run-eval: the tier-0 interpreter consumes ~50KB
    // of native stack per Python frame, so run the module on a dedicated
    // 256MB-stack thread (recursion_limit guard needs headroom to raise
    // RecursionError before the OS stack overflows).
    let exit_code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            let mut out = std::io::stdout();
            let mut err = std::io::stderr();
            let outcome = ap_run::run_module(&source, &script_path, &mut out, &mut err);
            let _ = out.flush();
            let _ = err.flush();
            outcome.exit_code
        })
        .expect("spawn interpreter thread")
        .join()
        .unwrap_or(134); // match a crash exit if the thread panics
    std::process::exit(exit_code);
}

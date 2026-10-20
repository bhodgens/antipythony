//! `ap-run-eval` — the T-05/T-06 testability bin: parse -> lower -> tier-0
//! execute a single .py file; stdout/stderr/exit mirror CPython (C-24).
//!
//! Accepts the documented gate seam `--tier <mode>` (C-31 / 02.03 gate,
//! T-06/T-07/T-08: `ap-conform run … --impl "<bin> --tier 0"`). This bin IS
//! the tier-0 executor, so `--tier 0` is the only mode it honors; any other
//! mode is rejected loudly rather than silently running tier-0 (a silent
//! accept would let an adaptive/binary gate report green on tier-0 output).
//!
//! robustness-ci 05.01: arg parsing moved to clap. The stdout/stderr/exit
//! bytes this bin emits for VALID invocations (and for the semantic errors
//! below) are the pinned test seam and are unchanged; only unrecognized-arg
//! / usage errors now render in clap's format (no suite pins those bytes —
//! verified by grep over tests/ before migration).

use clap::Parser;
use std::io::Write;

/// Parse -> lower -> tier-0 execute a single .py file (T-05/T-06 testability seam).
#[derive(Parser)]
#[command(name = "ap-run-eval", version)]
struct Args {
    /// Execution tier mode; this bin is the tier-0 executor, so only `0` is honored.
    #[arg(long, value_name = "MODE")]
    tier: Option<String>,

    /// Python source file to execute.
    file: String,
}

fn main() {
    let Args { tier, file: path } = Args::parse();

    if let Some(mode) = &tier {
        if mode != "0" {
            eprintln!(
                "ap-run-eval: --tier {mode} is not implemented; this bin runs tier-0 only \
                 (pass --tier 0 or omit --tier)"
            );
            std::process::exit(2);
        }
    }

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ap-run-eval: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let script_path = path.clone();

    // The tier-0 interpreter consumes ~50KB of native stack per Python frame,
    // so the default 8MB main-thread stack overflows (SIGABRT) at ~150 frames
    // - BEFORE the recursion_limit guard (1000, frame.rs enter_call) can
    // raise RecursionError. CPython raises RecursionError at depth 1000, so
    // the process must survive that deep: run the module on a dedicated
    // thread with a stack sized for the guard limit (1000 x ~50KB ~ 50MB,
    // rounded up to 256MB). Root fix (smaller interpreter frames) lives in
    // the executor; this keeps the conformance gate honest meanwhile.
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

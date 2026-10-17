//! `ap-run-eval` — the T-05/T-06 testability bin: parse -> lower -> tier-0
//! execute a single .py file; stdout/stderr/exit mirror CPython (C-24).
//!
//! Accepts the documented gate seam `--tier <mode>` (C-31 / 02.03 gate,
//! T-06/T-07/T-08: `ap-conform run … --impl "<bin> --tier 0"`). This bin IS
//! the tier-0 executor, so `--tier 0` is the only mode it honors; any other
//! mode is rejected loudly rather than silently running tier-0 (a silent
//! accept would let an adaptive/binary gate report green on tier-0 output).

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut tier: Option<String> = None;
    let mut path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--tier" => {
                let Some(mode) = args.get(i + 1) else {
                    eprintln!("ap-run-eval: --tier requires a mode (0 | adaptive)");
                    std::process::exit(2);
                };
                tier = Some(mode.clone());
                i += 2;
            }
            other if other.starts_with("--tier=") => {
                tier = Some(other["--tier=".len()..].to_string());
                i += 1;
            }
            other => {
                if path.is_some() {
                    eprintln!("ap-run-eval: unexpected argument '{other}'");
                    std::process::exit(2);
                }
                path = Some(other.to_string());
                i += 1;
            }
        }
    }

    let Some(path) = path else {
        eprintln!("usage: ap-run-eval [--tier 0] <file.py>");
        std::process::exit(2);
    };
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

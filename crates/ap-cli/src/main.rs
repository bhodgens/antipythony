//! `antipythany run <file.py>` — the conformance-facing runner (02.02+).
//! Tier-0 tree-walking executor over ap-ir; stdout/stderr/exit mirror
//! CPython (C-24/C-27/C-29). `--tier 0` is the only mode (this binary IS
//! tier-0; other modes are rejected loudly so adaptive/binary gates cannot
//! report green on tier-0 output).

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut tier: Option<String> = None;
    let mut path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "run" if path.is_none() && tier.is_none() => {
                // subcommand verb; ignored (single-verb binary)
            }
            "--tier" => {
                let Some(mode) = args.get(i + 1) else {
                    eprintln!("antipythany: --tier requires a mode (0 | adaptive)");
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
                    eprintln!("antipythany: unexpected argument '{other}'");
                    std::process::exit(2);
                }
                path = Some(other.to_string());
                i += 1;
            }
        }
    }

    let Some(path) = path else {
        eprintln!("usage: antipythany [--tier 0] <file.py>");
        std::process::exit(2);
    };
    if let Some(mode) = &tier {
        if mode != "0" {
            eprintln!(
                "antipythany: --tier {mode} is not implemented; this binary runs tier-0 only \
                 (pass --tier 0 or omit --tier)"
            );
            std::process::exit(2);
        }
    }

    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("antipythany: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let script_path = path.clone();

    // The tier-0 interpreter consumes ~50KB of native stack per Python frame
    // (recursion_limit 1000 needs ~50MB): run the module on a dedicated
    // thread with a 256MB stack (same contract as ap-run-eval).
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

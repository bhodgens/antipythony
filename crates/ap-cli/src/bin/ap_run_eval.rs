//! `ap-run-eval` — the T-05/T-06 testability bin: parse -> lower -> tier-0
//! execute a single .py file; stdout/stderr/exit mirror CPython (C-24).

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: ap-run-eval <file.py>");
        std::process::exit(2);
    };
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ap-run-eval: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let script_path = path.clone();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let outcome = ap_run::run_module(&source, &script_path, &mut out, &mut err);
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(outcome.exit_code);
}

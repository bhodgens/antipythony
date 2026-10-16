//! `ap-ir-dump <file.py>` — testability bin for the T-02 suite (gates 01.03).
//!
//! Contract (tests/t02_ir_shape): parse + lower the Python file and print the
//! IR dump to stdout, exit 0. On parse/lowering failure print the contract
//! diagnostic to stderr and exit 1. Missing input file: exit 2.
//!
//! Dump forms:
//! - default: T-02 tree form (one node-kind line per IR node, PascalCase
//!   kinds per the T-02 contract; CONTRACT-GAPS G-1);
//! - `--tree`: same as default (explicit);
//! - `--golden`: the golden formatter (`dump_module`), used for the
//!   checked-in `tests/fixtures/goldens/*.ir` byte-equality tests.

use std::process::ExitCode;

fn usage() -> ! {
    eprintln!("usage: ap-ir-dump [--tree|--golden] <file.py>");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let form = match args.first().map(String::as_str) {
        Some("--tree") | Some("--golden") => args.remove(0),
        Some(f) if f.starts_with("--") => usage(),
        _ => "--tree".to_string(),
    };
    let Some(path) = args.first().cloned() else {
        usage();
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("ap-ir-dump: cannot read input file: {path}");
            return ExitCode::from(2);
        }
    };
    match ap_parse::lower_source(&src) {
        Ok((module, _scopes)) => {
            let text = match form.as_str() {
                "--golden" => ap_ir::dump_module(&module),
                _ => ap_ir::dump_module_tree(&module),
            };
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

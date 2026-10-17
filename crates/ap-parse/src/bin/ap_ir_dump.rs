//! `ap-ir-dump <file.py>` — testability bin for the T-02 suite (gates 01.03),
//! with the leaf-01.04 `--verify` mode (gates T-04).
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
//!
//! Verify mode (`--verify`, leaf 01.04): parse + lower + verify and print
//! one diagnostic per line to STDERR in the form
//!
//! ```text
//! verify: {kind}: {qualname}: {message}
//! ```
//!
//! (with `path:line:col ` prefixed when the position resolved), exiting 3
//! when any UnboundLocal/UnresolvedName diagnostic was found, 0 with no
//! diagnostics, 1 on parse/lowering failure — 3 is distinct from the parse
//! error exit so callers can tell a semantic refusal from a syntax failure.

use std::process::ExitCode;

fn usage() -> ! {
    eprintln!("usage: ap-ir-dump [--tree|--golden|--verify] <file.py>");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let form = match args.first().map(String::as_str) {
        Some("--tree") | Some("--golden") | Some("--verify") => args.remove(0),
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
    match form.as_str() {
        "--verify" => match ap_parse::verify_source(&src) {
            Ok(diags) => {
                if diags.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    for d in &diags {
                        match (d.line, d.col) {
                            (Some(l), Some(c)) => eprintln!(
                                "verify: {path}:{l}:{c} {}: {}: {}",
                                d.kind.class(),
                                d.qualname,
                                d.message
                            ),
                            _ => eprintln!(
                                "verify: {}: {}: {}",
                                d.kind.class(),
                                d.qualname,
                                d.message
                            ),
                        }
                    }
                    ExitCode::from(3)
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        _ => match ap_parse::lower_source(&src) {
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
        },
    }
}

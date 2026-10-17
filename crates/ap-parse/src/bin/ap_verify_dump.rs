//! `ap-verify-dump <file.py>` — testability bin for the T-04 suite (gates 01.04).
//!
//! Contract (tests/t04_ir_verify, T-04 doc G-1 seam): parse + lower + verify
//! the Python file and print one diagnostic per line to STDOUT in the
//! machine-stable C-10 shape
//!
//! ```text
//! <path>:<line>:<col> <class>: <message>
//! ```
//!
//! with 1-based line/col (`<col>` anchored at CPython's fine-grained-error
//! caret per S-02-37 where derivable — implemented as
//! `col_file - indent + 4`, see ap_parse::verify_bridge docs). Exit 0 with
//! empty stdout when there are no diagnostics; exit 1 when at least one
//! diagnostic was printed. Parse/lowering failure: contract diagnostic to
//! stderr, exit 1, nothing on stdout (distinguishable). Missing input file:
//! exit 2.

use std::process::ExitCode;

fn usage() -> ! {
    eprintln!("usage: ap-verify-dump <file.py>");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        usage();
    };
    if path == "--help" || path == "-h" {
        usage();
    }
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("ap-verify-dump: cannot read input file: {path}");
            return ExitCode::from(2);
        }
    };
    match ap_parse::verify_source(&src) {
        Ok(diags) => {
            if diags.is_empty() {
                ExitCode::SUCCESS
            } else {
                for d in &diags {
                    // T-04 G-1 C-10 line: `<path>:<line>:<col> <class>: <message>`
                    // (message WITHOUT the qualname — the qualname rides in
                    // the Diagnostic and the ap-ir-dump --verify surface).
                    let pos = match (d.line, d.col) {
                        (Some(l), Some(c)) => format!("{path}:{l}:{c} "),
                        _ => format!("{path}:0:0 "),
                    };
                    println!("{}{}: {}", pos, d.kind.class(), d.message);
                }
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}

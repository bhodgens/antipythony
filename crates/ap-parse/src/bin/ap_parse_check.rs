//! `ap-parse-check <file.py>` — parse-diagnostics-only testability seam for
//! the T-01 suites (closes CONTRACT-GAPS G-1; robustness-ci leaf 05.02).
//!
//! Parse the file with `ap_parse::parse` (Python 3.14 pin, error recovery
//! off) and report the FIRST diagnostic. Both T-01 suites run this bin and
//! pin different renderings of the same `ParseError`, so the reject path
//! prints the identical (line, col, message) twice, once per pinned shape:
//!
//! ```text
//! stderr line 1: error at line L, column C: <message>   (tests/t01_parse)
//! stderr line 2: <path>:<L>:<C> <message>               (tests/t01_parse_diagnostics)
//! ```
//!
//! Neither regex matches the other's line, so the fail-fast "exactly ONE
//! diagnostic" counts hold on both sides (T-01 doc G-7).
//!
//! Contract:
//!
//! - accept: exit 0, stderr EMPTY, stdout `OK <n> statements` (n = top-level
//!   statement count; the `OK` shape is pinned by tests/t01_parse;
//!   tests/t01_parse_diagnostics ignores stdout).
//! - reject: exit 1, stdout empty, stderr carries exactly one diagnostic in
//!   each pinned shape, 1-based line + 1-based CHARACTER column (ap_parse's
//!   line_col_of counts Unicode scalars — the C-02 reference reading pinned
//!   by T-01 doc gap G-2).
//! - missing/unreadable input file: exit 2.
//!
//! The diagnostic text comes from `ParseError`'s existing Display + fields —
//! the same source ap-verify-dump renders for parse failures. This bin adds
//! no new semantics.

use clap::Parser;
use std::process::ExitCode;

/// Parse a Python file and report the first parse diagnostic (T-01 seam).
#[derive(Parser)]
#[command(name = "ap-parse-check", version)]
struct Args {
    /// Python source file to parse-check.
    file: String,
}

fn main() -> ExitCode {
    let Args { file: path } = Args::parse();
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("ap-parse-check: cannot read input file: {path}");
            return ExitCode::from(2);
        }
    };
    match ap_parse::parse(&src) {
        Ok(ast) => {
            // Pinned byte-exact (no trailing newline): tests/t01_parse
            // asserts stdout == b"OK 1 statements".
            print!("OK {} statements", ast.module().body.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            // Line 1: tests/t01_parse shape — ParseError's byte-exact Display.
            eprintln!("{e}");
            // Line 2: tests/t01_parse_diagnostics shape — the C-10/C-29
            // file:line:col family over the same fields.
            eprintln!("{path}:{}:{} {}", e.line, e.col, e.message);
            ExitCode::from(1)
        }
    }
}

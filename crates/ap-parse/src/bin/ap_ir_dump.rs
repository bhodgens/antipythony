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
//!
//! robustness-ci 05.01: arg parsing moved to clap. The dump/verify/`cannot
//! read` output bytes for valid invocations are the pinned test seam and are
//! unchanged; only unrecognized-arg / usage errors now render in clap's
//! format (no suite pins those bytes — verified by grep over tests/).

use clap::{ArgGroup, Parser};
use std::process::ExitCode;

/// Parse + lower a Python file and dump its IR (T-02/T-03/T-04 seam).
#[derive(Parser)]
#[command(name = "ap-ir-dump", version)]
#[command(group(ArgGroup::new("form").args(["tree", "golden", "verify"])))]
struct Args {
    /// T-02 tree form (one node-kind line per IR node). Default.
    #[arg(long)]
    tree: bool,
    /// Golden formatter (dump_module) for checked-in *.ir byte-equality.
    #[arg(long)]
    golden: bool,
    /// Verify mode: diagnostics to stderr, exit 3 on semantic refusals.
    #[arg(long)]
    verify: bool,

    /// Python source file to dump.
    file: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let form = if args.verify {
        "verify"
    } else if args.golden {
        "golden"
    } else {
        "tree"
    };
    let path = args.file;
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("ap-ir-dump: cannot read input file: {path}");
            return ExitCode::from(2);
        }
    };
    match form {
        "verify" => match ap_parse::verify_source(&src) {
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
                let text = match form {
                    "golden" => ap_ir::dump_module(&module),
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

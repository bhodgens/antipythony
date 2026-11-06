//! `ap-obj-eval` — T-05 single-expression mode: evaluate one expression from
//! argv (or stdin) and print its repr; reuses the tier-0 pipeline
//! (`ap-obj-eval '1 + 2'` -> `3`). Flips the T-05 xfail gates.
//!
//! robustness-ci 05.01: arg parsing moved to clap. Output bytes for valid
//! invocations are unchanged; argv is still joined with single spaces, so
//! `ap-obj-eval 1 + 2` -> `print(repr((1 + 2)))` exactly as before.

use clap::Parser;
use std::io::Read;
use std::io::Write;

/// Evaluate one Python expression from argv (or stdin) and print its repr.
#[derive(Parser)]
#[command(name = "ap-obj-eval", version)]
struct Args {
    /// Expression tokens (joined with spaces); empty -> read stdin.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    expr: Vec<String>,
}

fn main() {
    // 2026-11-06 OOM mitigation: in-process RSS self-guard (see
    // ap_run::rss_guard). AP_MAX_RSS_GB overrides; default = 25% of RAM.
    ap_run::rss_guard::start();
    let Args { expr } = Args::parse();
    let expr_src = if expr.is_empty() {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("ap-obj-eval: cannot read stdin");
            std::process::exit(2);
        }
        buf
    } else {
        expr.join(" ")
    };
    // Wrap as a print of the expression: one-statement pipeline reuse.
    let source = format!("print(repr(({expr_src})))\n");
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let outcome = ap_run::run_module(&source, "<expr>", &mut out, &mut err);
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(outcome.exit_code);
}

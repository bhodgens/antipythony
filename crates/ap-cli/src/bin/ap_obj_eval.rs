//! `ap-obj-eval` — T-05 single-expression mode: evaluate one expression from
//! argv (or stdin) and print its repr; reuses the tier-0 pipeline
//! (`ap-obj-eval '1 + 2'` → `3`). Flips the T-05 xfail gates.

use std::io::Read;
use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let expr_src = if args.is_empty() {
        let mut buf = String::new();
        if std::io::stdin().read_to_string(&mut buf).is_err() {
            eprintln!("ap-obj-eval: cannot read stdin");
            std::process::exit(2);
        }
        buf
    } else {
        args.join(" ")
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

//! pp2-tier1 unit repro + FIX VERIFICATION: the Cranelift "invalid block
//! reference block3" verifier failure on the dict-get loop shape.
//!
//! Black-box trigger (tests/bench/primitive/probes/_pp2_minimize11.py q1):
//! a while loop whose body contains a Call with a Subscript-produced arg
//! AND a BinOp over loop-live locals.
//!
//! Lowered shape: ONE body block (b1) whose insts are
//!   OsrPoll, ..., Subscript, Call, BinOp, ..., Compare(term Branch)
//! i.e. the OsrPoll is at the TOP of the same block as 3 fallible helper
//! sites. Why shape-sensitive: cranelift's FunctionBuilder requires that
//! a block be TERMINATED exactly once; the OsrPoll arm emits a brif +
//! (only on the transfer path) returns, and when osr_plan_resolved() is
//! None it EARLY-RETURNS from emit_cfg_with_osr via `return Ok(())` at
//! the middle of block b1's inst loop — leaving b1 UNTERMINATED with all
//! remaining insts unemitted, while earlier helper sites in OTHER blocks
//! already referenced the blocks[] handles (block3 = b2/b3's handle,
//! created but whose layout insertion happens later). Cranelift verifies
//! "invalid block reference block3".
//!
//! The FIX (verified here): don't early-return mid-block; emit a
//! terminator + continue (fail-closed status-2 return, matching the
//! Unreachable arm's discipline), so every block keeps a consistent
//! terminator sequence.
//!
//! run: cargo run -p ap-codegen-native --example repro_verify
//! (CARGO_TARGET_DIR=$PWD/target-pp2-tier1)

use ap_codegen_native::cfg;
use ap_codegen_native::clif::{compile_baseline, compile_baseline_with_osr};
use ap_codegen_native::osr;
use ap_parse::lowering::lower_source;

const SRC: &str = r#"
ks = [0, 1, 0, 1, 0, 1, 0, 1]
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(ks[i])
        i = i + 1
    return c

probe()
print("ok")
"#;

const SRC_PLAIN_ARG: &str = r#"
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(0)
        i = i + 1
    return c

probe()
print("ok")
"#;

fn try_compile(src: &str, label: &str) {
    let (module, _scope) = match lower_source(src) {
        Ok(m) => m,
        Err(e) => {
            println!("{label}: lower_source failed: {e:?}");
            return;
        }
    };
    for item in &module.items {
        if let ap_ir::Item::FnDef(f) = item {
            if f.name != "probe" {
                continue;
            }
            let func = match cfg::lower_fn(f) {
                Ok(f) => f,
                Err(e) => {
                    println!("{label}: cfg::lower_fn failed: {e:?}");
                    return;
                }
            };
            // Which header does loop_headers pick for the poll?
            let headers = osr::loop_headers(&func);
            println!(
                "{label}: loop_headers = {:?}",
                headers.iter().map(|h| h.0).collect::<Vec<_>>()
            );
            for &h in &headers {
                match osr::osr_plan(&func, h) {
                    osr::OsrEligibility::Eligible { locals, live_in } => {
                        println!(
                            "  header {} eligible: {} locals + {} live_in",
                            h.0,
                            locals.len(),
                            live_in.len()
                        );
                    }
                    osr::OsrEligibility::Ineligible { reason, live_count } => {
                        println!("  header {} INELIGIBLE ({live_count}): {reason}", h.0);
                    }
                }
            }
            match compile_baseline(&func) {
                Ok(_) => println!("{label} [no-osr]: COMPILED"),
                Err(e) => println!("{label} [no-osr]: FAILS: {e}"),
            }
            match compile_baseline_with_osr(&func, 42) {
                Ok(_) => println!("{label} [osr]: COMPILED"),
                Err(e) => println!("{label} [osr]: REPRODUCED: {e}"),
            }
        }
    }
}

fn main() {
    try_compile(SRC, "q1(sub-arg+binop) ");
    try_compile(SRC_PLAIN_ARG, "q3(lit-arg+binop) ");
}

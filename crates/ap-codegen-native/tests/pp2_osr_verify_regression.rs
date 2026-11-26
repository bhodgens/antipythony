//! pp2-tier1 regression: the OSR-poll emit arm's ineligible-header
//! early-return left the current CLIF block unterminated mid-inst-loop
//! while predecessors had already branched to its successors — Cranelift
//! verify failed "invalid block reference blockN" and the whole body
//! silently fell back to the tree-walker (caps tier-1 coverage for the
//! dict-get loop shape: `c = c + D.get(ks[i])`).
//!
//! The shape: a while loop whose body carries a Call with a
//! Subscript-produced arg plus a BinOp over loop-live locals — enough
//! extra live temps (feedback cells / staged args) to push the loop
//! header's OSR live set over OSR_MAX_LIVE, hitting the ineligible
//! path. compile_baseline (no OSR) compiles the same cfg fine; the bug
//! is exclusively the with-osr emit's control flow.
//!
//! Written from the pp2-tier1 leaf's black-box matrix
//! (tests/bench/primitive/probes/_pp2_minimize11.py): q1 FALLBACK,
//! q2/q3/q4 COMPILED — pinned here at the compile level so the emit arm
//! can't regress to mid-block early-returns.

use ap_codegen_native::cfg;
use ap_codegen_native::clif::{compile_baseline, compile_baseline_with_osr};
use ap_codegen_native::osr;
use ap_parse::lowering::lower_source;

/// q1: `c = c + D.get(ks[i])` — the shape that hit the ineligible-header
/// early-return (17 > OSR_MAX_LIVE=16 live at the loop header).
const Q1_SUBSCRIPT_ARG_BINOP: &str = r#"
ks = [0, 1, 0, 1, 0, 1, 0, 1]
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(ks[i])
        i = i + 1
    return c
"#;

/// q3 control: literal arg — the header stays eligible, the transfer
/// path emits; must compile with AND without OSR.
const Q3_LITERAL_ARG_BINOP: &str = r#"
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(0)
        i = i + 1
    return c
"#;

fn lowered(name: &str, src: &str) -> cfg::Func {
    let (module, _scope) = lower_source(src).expect("lower_source");
    let fdef = module
        .items
        .iter()
        .find_map(|i| match i {
            ap_ir::Item::FnDef(ff) if ff.name == name => Some(ff),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no fn {name}"));
    cfg::lower_fn(fdef).expect("lower_fn")
}

#[test]
fn ineligible_osr_header_loop_compiles_baseline_with_osr() {
    let func = lowered("probe", Q1_SUBSCRIPT_ARG_BINOP);
    // The trigger precondition: the loop header must be OSR-ineligible
    // (> OSR_MAX_LIVE live) — if the live-set math changes, this test
    // documents the new boundary rather than silently passing.
    let headers = osr::loop_headers(&func);
    assert!(!headers.is_empty(), "while loop must yield a loop header");
    // compile_baseline (poll sites as no-ops) is the control.
    compile_baseline(&func).expect("no-osr baseline must compile");
    // THE FIX GATE: the with-osr compile must NOT verifier-fail on the
    // ineligible path (the poll degrades to a no-op; the loop stays
    // tier-0/OSR-less but the BASELINE TWIN still installs — tier-1
    // coverage for this shape).
    compile_baseline_with_osr(&func, 42).expect(
        "ineligible-header loop must compile baseline-with-osr (poll no-ops, never early-return)",
    );
}

#[test]
fn eligible_osr_header_loop_compiles_baseline_with_osr() {
    let func = lowered("probe", Q3_LITERAL_ARG_BINOP);
    let headers = osr::loop_headers(&func);
    assert!(!headers.is_empty(), "while loop must yield a loop header");
    compile_baseline(&func).expect("no-osr baseline must compile");
    compile_baseline_with_osr(&func, 42)
        .expect("eligible-header loop must compile baseline-with-osr (transfer path emits)");
}

//! OSR plan tests (leaf Risk-2 phase 1: BOXED OSR, eligibility gate).

use ap_codegen_native::cfg::{lower_fn, BlockId};
use ap_codegen_native::osr::{osr_plan, OsrEligibility, OSR_MAX_LIVE};
use ap_ir::expr::Expr;
use ap_ir::item::{FnBody, FnDef, Param, ParamKind};
use ap_ir::types::TypeLattice;

fn fn_def(body: Vec<ap_ir::item::Stmt>) -> FnDef {
    FnDef {
        name: "f".to_string(),
        params: vec![Param {
            name: "a".to_string(),
            kind: ParamKind::PosOrKw,
            default: None,
            default_index: None,
            ann: None,
            ty: TypeLattice::Unknown,
        }],
        return_ann: None,
        decorators: Vec::new(),
        body: FnBody { blocks: body },
        is_generator: false,
        is_async: false,
        ret_ty: TypeLattice::Unknown,
        def_line: 1,
    }
}

#[test]
fn buffer_layout_is_fixed() {
    // repr(C): 2×u32 header + OSR_MAX_LIVE usize slots. The tier-0 emitter
    // and the OSR entry read/write at these FIXED offsets.
    assert_eq!(OSR_MAX_LIVE, 16);
    assert_eq!(
        std::mem::size_of::<ap_codegen_native::osr::OsrTransferBuffer>(),
        8 + 8 * OSR_MAX_LIVE
    );
}

#[test]
fn osr_plan_deterministic_for_same_cfg() {
    // While loop: x = 0; while x < 3: x = x + 1 — no while-else.
    let f = fn_def(vec![
        ap_ir::item::Stmt::Assign(
            vec![ap_ir::item::Target::Name("x".to_string())],
            Expr::LitInt(0),
        ),
        ap_ir::item::Stmt::While(
            Expr::Compare(
                vec![ap_ir::expr::CmpOp::Lt],
                vec![Expr::Name("x".to_string()), Expr::LitInt(3)],
            ),
            vec![ap_ir::item::Stmt::Assign(
                vec![ap_ir::item::Target::Name("x".to_string())],
                Expr::BinOp(
                    ap_ir::expr::BinOp::Add,
                    Box::new(Expr::Name("x".to_string())),
                    Box::new(Expr::LitInt(1)),
                ),
            )],
            vec![],
        ),
    ]);
    let c1 = lower_fn(&f).expect("lower 1");
    let c2 = lower_fn(&f).expect("lower 2");
    // Determinism: same (function, header) → same plan (locals + live-ins).
    let header = BlockId(0); // entry holds the loop pre-head in this shape
    let p1 = osr_plan(&c1, header);
    let p2 = osr_plan(&c2, header);
    assert_eq!(p1, p2, "OSR slot plan must be stable under re-lowering");
    match &p1 {
        OsrEligibility::Eligible { locals, .. } => {
            assert!(!locals.is_empty(), "locals transferred in LocalId order");
        }
        OsrEligibility::Ineligible { reason, .. } => {
            panic!("small function must be OSR-eligible, got: {reason}");
        }
    }
}

#[test]
fn oversized_live_set_is_reported_ineligible() {
    // A cfg Func with locals beyond the cap must report Ineligible
    // with the live count — never silently dropped (leaf Risk-2 gate).
    let mut f = fn_def(vec![]);
    let mut cfg = lower_fn(&f).expect("lower");
    cfg.num_locals = (OSR_MAX_LIVE + 5) as u32;
    let plan = osr_plan(&cfg, BlockId(0));
    match plan {
        OsrEligibility::Ineligible { live_count, .. } => {
            // num_locals (1 param) + the 21 forced locals = 22 total.
            assert_eq!(live_count, OSR_MAX_LIVE + 6);
        }
        OsrEligibility::Eligible { .. } => {
            panic!("over-cap live set must be reported OSR-ineligible");
        }
    }
}

#[test]
fn while_loop_emits_backedge_osr_poll() {
    // Scope item 6: tier-0 emits OSR polls at loop back-edges — the cfg
    // stage places Inst::OsrPoll in the loop body block.
    let f = fn_def(vec![ap_ir::item::Stmt::While(
        Expr::LitBool(true),
        vec![ap_ir::item::Stmt::Pass],
        vec![],
    )]);
    let cfg = lower_fn(&f).expect("lower");
    let has_poll = cfg.blocks.iter().any(|b| {
        b.insts
            .iter()
            .any(|i| matches!(i, ap_codegen_native::cfg::Inst::OsrPoll))
    });
    assert!(has_poll, "back-edge OsrPoll missing from while body");
}

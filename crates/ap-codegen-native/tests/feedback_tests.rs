//! Feedback substrate tests (leaf Risk-4/Risk-5 probes) + SlotPlan
//! determinism (final-lowering slot assignment).

use ap_codegen_native::cfg::lower_fn;
use ap_codegen_native::clif::SlotPlan;
use ap_codegen_native::feedback::{FeedbackCell, FeedbackState, TypeTag, IC_EMPTY, IC_WRITING};
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

// ---------------------------------------------------------------------------
// Cell state machine (pon feedback.rs discipline)
// ---------------------------------------------------------------------------

#[test]
fn cell_starts_empty_and_records_mono() {
    let cell = FeedbackCell::new();
    assert_eq!(cell.read().map(|(s, _, _)| s), Some(FeedbackState::Empty));
    cell.record(TypeTag::IntI64, Some(TypeTag::IntI64));
    let (state, a, b) = cell.read().expect("no writer in flight");
    assert_eq!(state, FeedbackState::Mono);
    assert_eq!(a, TypeTag::IntI64);
    assert_eq!(b, Some(TypeTag::IntI64));
}

#[test]
fn divergent_tags_promote_through_poly_to_mega() {
    let cell = FeedbackCell::new();
    cell.record(TypeTag::IntI64, Some(TypeTag::IntI64));
    // Divergent observation past saturation → POLY.
    for _ in 0..200 {
        cell.record(TypeTag::Float, Some(TypeTag::Float));
    }
    let state = cell.read().expect("readable").0;
    assert!(
        state == FeedbackState::Poly || state == FeedbackState::Mega,
        "divergent tags must leave MONO, got {state:?}"
    );
    // POLY + one more divergent observation → MEGA (saturating end state).
    for _ in 0..10 {
        cell.record(TypeTag::Str, None);
    }
    let state = cell.read().expect("readable").0;
    assert_eq!(state, FeedbackState::Mega, "mega is terminal");
    // MEGA stops tracking (no further state churn).
    cell.record(TypeTag::Bool, None);
    assert_eq!(cell.read().unwrap().0, FeedbackState::Mega);
}

#[test]
fn seqlock_writer_claim_tears_reads() {
    // Simulate the IC_WRITING window: a reader must return None (slow
    // path), never a torn payload.
    let cell = FeedbackCell::new();
    cell.record(TypeTag::IntI64, Some(TypeTag::IntI64));
    // Force the writing sentinel (as a concurrent writer would).
    use std::sync::atomic::Ordering;
    cell.packed.store(IC_WRITING, Ordering::Release);
    assert!(cell.read().is_none(), "writer-in-flight ⇒ torn-read guard");
    cell.packed.store(IC_EMPTY, Ordering::Release);
    assert!(cell.read().is_some());
}

#[test]
fn audit_reset_rewinds_to_empty() {
    // APX_IC_AUDIT=1 cross-check substrate: rewind forces the slow path
    // to re-record; cached-vs-recomputed comparison happens at the hook.
    let cell = FeedbackCell::new();
    cell.record(TypeTag::IntI64, Some(TypeTag::IntI64));
    cell.audit_reset();
    assert_eq!(cell.read().map(|(s, _, _)| s), Some(FeedbackState::Empty));
}

#[test]
fn cell_layout_frozen() {
    // pon offset-assert discipline: 32 bytes, packed first.
    assert_eq!(std::mem::size_of::<FeedbackCell>(), 32);
    assert_eq!(std::mem::align_of::<FeedbackCell>(), 8);
}

// ---------------------------------------------------------------------------
// TypeTag classification (ap-obj surface reads only)
// ---------------------------------------------------------------------------

#[test]
fn type_tag_classifies_boxed_values() {
    use ap_obj::Int;
    use ap_obj::PyString;
    use ap_obj::Value;
    assert_eq!(TypeTag::of(&Value::None), TypeTag::Other);
    assert_eq!(TypeTag::of(&Value::Bool(true)), TypeTag::Bool);
    assert_eq!(TypeTag::of(&Value::Int(Int::from_i64(5))), TypeTag::IntI64);
    let big = ap_obj::Int::from_big(num_bigint::BigInt::from(2u8).pow(200u32).into());
    assert_eq!(TypeTag::of(&Value::Int(big)), TypeTag::IntBig);
    assert_eq!(TypeTag::of(&Value::Float(1.5)), TypeTag::Float);
    assert_eq!(TypeTag::of(&Value::Str(PyString::new("x"))), TypeTag::Str);
    // Only IntI64 is unboxable in this wave (leaf: smi/i64 fast path first).
    assert!(TypeTag::IntI64.unboxable());
    assert!(!TypeTag::Float.unboxable());
    assert!(!TypeTag::IntBig.unboxable());
}

// ---------------------------------------------------------------------------
// Risk-5: slot assignment ONLY at final lowering, deterministic
// ---------------------------------------------------------------------------

#[test]
fn slot_assignment_is_deterministic_and_stable() {
    // Same function lowered twice → identical SlotPlan (count AND kinds
    // in order). Mutation-during-compile safety follows from this +
    // the snapshot-at-enqueue discipline.
    let f = fn_def(vec![ap_ir::item::Stmt::ExprStmt(Expr::BinOp(
        ap_ir::expr::BinOp::Add,
        Box::new(Expr::Attribute(
            Box::new(Expr::Name("x".to_string())),
            "y".to_string(),
        )),
        Box::new(Expr::LitInt(1)),
    ))]);
    let cfg1 = lower_fn(&f).expect("lower");
    let cfg2 = lower_fn(&f).expect("lower");
    let p1 = SlotPlan::assign(&cfg1);
    let p2 = SlotPlan::assign(&cfg2);
    assert_eq!(p1, p2, "re-lowering determinism (Risk-5 probe)");
    // The site kinds: LoadGlobal(x), LoadAttr(y), BinOp(+) — in order.
    assert_eq!(p1.len(), 3);
    assert_eq!(
        p1.sites,
        vec![
            ap_codegen_native::clif::SiteKind::LoadGlobal,
            ap_codegen_native::clif::SiteKind::LoadAttr,
            ap_codegen_native::clif::SiteKind::BinOp,
        ]
    );
}

#[test]
fn non_specializable_ops_get_no_slots() {
    // Truthy/StoreLocal etc. carry no slot: density assert — every slot
    // maps to a specializable site and vice versa.
    let f = fn_def(vec![ap_ir::item::Stmt::Assign(
        vec![ap_ir::item::Target::Name("b".to_string())],
        Expr::LitInt(7),
    )]);
    let cfg = lower_fn(&f).expect("lower");
    let plan = SlotPlan::assign(&cfg);
    assert!(plan.is_empty(), "Const/StoreLocal are not specializable");
}

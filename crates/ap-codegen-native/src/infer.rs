//! 02.05c metadata/infer pass: feedback snapshots + annotations → per-temp
//! inferred types over the internal [`cfg::Func`].
//!
//! pon analogue: pon-codegen/src/infer.rs seeds `Inst::inferred_type` from
//! annotations + feedback before the region finder runs. Adapted from pon
//! (github.com/can1357/pon). Copyright Can Bölük, MIT/Apache-2.0; adapted
//! for antipythony under the granted dual license with attribution.
//!
//! Typing rules (repr decision — PROGRESS-0205c): an unboxed temp is an
//! i64 in the ap-obj smi domain ([`crate::infer::is_smi`], the
//! [-2^59, 2^59) `Int::Smi` arm). Three admission sources, weakest first:
//! - [`Inferred::SmiConst`] — an in-domain integer literal (static fact).
//! - [`Inferred::SmiLocal`] — a LoadLocal result: DYNAMICALLY checked by
//!   the runtime guard (`h_slot_smi_v`, sentinel compare) on every region
//!   entry-to-use path; a Bool/bigint/str local side-exits to the cold
//!   twin. Sound by guard, admission costs nothing statically.
//! - [`Inferred::SmiBin`] — a BinOp result whose operands are unboxable:
//!   exactness (smi-range) is runtime-checked per op, type follows from
//!   the operand guards.
//! - [`Inferred::Bool`] — Compare/Truthy results over unboxable operands
//!   (a 0/1 i64 that boxes back as `Value::Bool`).
//!
//! The FEEDBACK snapshot's role (deliberate divergence from pon, recorded
//! in PROGRESS-0205c): the snapshot does not type temps — the value-level
//! guards do — it gates WHEN tier-1 compiles (jit_hook tiers up a function
//! only once a binop cell reached MONO(IntI64, IntI64); see
//! [`has_int_hotness`]). pon types from feedback because its guards are
//! version-based; our guards are value-based, so feedback is a hotness
//! signal here. The snapshot still rides the enqueueing thread only
//! (never read concurrently with live cells).
//!
//! Discipline (leaf Risk-5 / study problem #5): the site walk here uses
//! the SAME shared predicate ([`crate::clif::specializable`]) and the same
//! deterministic (block, inst) order as `SlotPlan::assign`; the site count
//! is cross-checked against the actual plan at compile time
//! (assert-density analogue of pon's `debug_assert_suspend_numbering`).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::cfg::{ConstValue, Func, Inst, TempId};
use crate::clif::specializable;
use crate::feedback::{CellSnapshot, FeedbackState, TypeTag};

/// What tier-1 may assume about a temp's runtime representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inferred {
    /// No usable information: always boxed, never unboxed.
    Unknown,
    /// In-domain integer literal (statically exact).
    SmiConst,
    /// LoadLocal result: unboxed ONLY through the runtime smi guard.
    SmiLocal,
    /// Typed-arithmetic result (operands unboxable; exactness guarded).
    SmiBin,
    /// Compare/Truthy result over unboxable operands: a 0/1 i64 that
    /// boxes back as `Value::Bool`.
    Bool,
}

impl Inferred {
    /// Whether the temp may hold an unboxed i64 register.
    pub fn unboxable(self) -> bool {
        matches!(
            self,
            Inferred::SmiConst | Inferred::SmiLocal | Inferred::SmiBin
        )
    }

    /// Whether the temp is a 0/1 bit that boxes as Bool.
    pub fn is_bool(self) -> bool {
        self == Inferred::Bool
    }
}

/// The inference result: per-temp types plus the site count the caller
/// cross-checks against the actual SlotPlan.
#[derive(Debug, Clone, Default)]
pub struct InferInfo {
    /// TempId → inferred type (index == TempId.0).
    pub temps: Vec<Inferred>,
    /// Number of feedback slots the deterministic walk counted (must
    /// equal `SlotPlan::len` — checked by the caller).
    pub site_count: usize,
}

/// Run the inference pass.
///
/// `snapshots` is indexed by feedback-slot id (SlotPlan order). Temps are
/// BLOCK-LOCAL in this cfg (statement-tree lowering: every temp is defined
/// and consumed inside one block; locals are the cross-block vehicles), so
/// a single in-order walk per block types everything reachable.
pub fn infer(func: &Func, snapshots: &[CellSnapshot]) -> InferInfo {
    let num_temps = max_temp(func) as usize + 1;
    let mut info = InferInfo {
        temps: vec![Inferred::Unknown; num_temps],
        site_count: 0,
    };
    for block in &func.blocks {
        for inst in &block.insts {
            // Risk-5 lockstep: the site count uses the SAME shared
            // predicate the SlotPlan assign walk uses (one definition).
            if specializable(inst).is_some() {
                info.site_count += 1;
            }
            match inst {
                Inst::Const(ConstValue::Int(v), d) if is_smi(*v) => {
                    info.temps[d.0 as usize] = Inferred::SmiConst;
                }
                Inst::LoadLocal(_, d) => {
                    info.temps[d.0 as usize] = Inferred::SmiLocal;
                }
                Inst::BinOp(_, a, b, d) => {
                    if operand_ok(&info, *a) && operand_ok(&info, *b) {
                        info.temps[d.0 as usize] = Inferred::SmiBin;
                    }
                    // Feedback bookkeeping: the snapshot at this slot is
                    // the hotness signal (see [`has_int_hotness`]); the
                    // walk order here IS the SlotPlan order, so slot ==
                    // the site id assigned by the final lowering.
                    let _ = snapshots;
                }
                Inst::Compare(op, a, b, d) => {
                    if relational(*op) && operand_ok(&info, *a) && operand_ok(&info, *b) {
                        info.temps[d.0 as usize] = Inferred::Bool;
                    }
                }
                Inst::Truthy(a, d) => {
                    if operand_ok(&info, *a) {
                        info.temps[d.0 as usize] = Inferred::Bool;
                    }
                }
                _ => {}
            }
        }
    }
    info
}

/// Whether `t` may flow into a typed op (its guard/definition makes the
/// unboxed read sound).
fn operand_ok(info: &InferInfo, t: TempId) -> bool {
    info.temps
        .get(t.0 as usize)
        .copied()
        .unwrap_or(Inferred::Unknown)
        .unboxable()
}

/// The six relational comparisons with native i64 semantics. Is/IsNot/In/
/// NotIn are identity/membership (boxed protocol ops) — never typed.
pub fn relational(op: ap_ir::expr::CmpOp) -> bool {
    use ap_ir::expr::CmpOp::*;
    matches!(op, Eq | NotEq | Lt | LtE | Gt | GtE)
}

/// Hotness gate: at least one binop site's snapshot says MONO int-int
/// (the steady-state arithmetic loop shape tier-1 exists for). Called on
/// the enqueueing thread with fresh snapshots (pon snapshot discipline).
pub fn has_int_hotness(snapshots: &[CellSnapshot]) -> bool {
    snapshots.iter().any(|s| {
        s.state == FeedbackState::Mono && matches!(s.tags, Some((TypeTag::IntI64, TypeTag::IntI64)))
    })
}

/// Highest TempId the CFG references (slot-space bound; the clif emitter's
/// `temp_slot` offsets temps above locals).
pub fn max_temp(func: &Func) -> u32 {
    let mut hi = 0u32;
    let bump = |t: TempId, hi: &mut u32| {
        if t.0 > *hi {
            *hi = t.0;
        }
    };
    for b in &func.blocks {
        for i in &b.insts {
            for t in temps_of(i) {
                bump(t, &mut hi);
            }
        }
        match &b.term {
            Some(crate::cfg::Terminator::Branch(c, _, _)) => bump(*c, &mut hi),
            Some(crate::cfg::Terminator::Return(v) | crate::cfg::Terminator::Raise(v, _)) => {
                bump(*v, &mut hi)
            }
            Some(crate::cfg::Terminator::Suspend(v, _, _)) => bump(*v, &mut hi),
            _ => {}
        }
    }
    hi
}

/// The temp a specializable inst WRITES (its result), if any.
#[allow(dead_code)] // diagnostic surface (first_disqualifier reporting)
fn writes_of(inst: &Inst) -> Option<TempId> {
    match inst {
        Inst::BinOp(_, _, _, d)
        | Inst::LoadAttr(_, _, d)
        | Inst::LoadGlobal(_, d)
        | Inst::Call(_, _, d)
        | Inst::Const(_, d) => Some(*d),
        _ => None,
    }
}

/// All temps an inst references.
fn temps_of(inst: &Inst) -> Vec<TempId> {
    match inst {
        Inst::LoadLocal(_, d)
        | Inst::Const(_, d)
        | Inst::LoadGlobal(_, d)
        | Inst::LoadAttr(_, _, d)
        | Inst::Subscript(_, _, d)
        | Inst::Truthy(_, d) => vec![*d],
        Inst::StoreLocal(s, _) => vec![*s],
        Inst::UnaryOp(_, a, d) => vec![*a, *d],
        Inst::BinOp(_, a, b, d) | Inst::Compare(_, a, b, d) => vec![*a, *b, *d],
        Inst::Call(f, args, d) => {
            let mut r = vec![*f, *d];
            r.extend(args.iter().copied());
            r
        }
        Inst::StoreAttr(o, _, v) => vec![*o, *v],
        Inst::MatchExc(_, _, d) => vec![*d],
        Inst::Print(v) => vec![*v],
        Inst::PushExcInfo(_)
        | Inst::PopExcInfo(_)
        | Inst::Reraise
        | Inst::SuspendMarker
        | Inst::OsrPoll => vec![],
    }
}

/// Whether an i64 literal lies in the smi fast-path domain.
pub fn is_smi(v: i64) -> bool {
    (ap_obj::int::SMI_MIN..=ap_obj::int::SMI_MAX).contains(&v)
}

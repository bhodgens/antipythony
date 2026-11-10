//! 02.05c region finder: maximal single-entry unboxable regions over the
//! internal [`cfg::Func`].
//!
//! pon analogue: pon-codegen/src/region.rs ("maximal single-entry typed
//! region admitting only blocks whose insts carry unboxable metadata").
//! Adapted from pon (github.com/can1357/pon). Copyright Can Bölük,
//! MIT/Apache-2.0; adapted for antipythony under the granted dual license
//! with attribution.
//!
//! A REGION is a maximal stretch of blocks B_i..B_j such that:
//! 1. every member block is ELIGIBLE (only unboxable traffic: smi
//!    constants, LoadLocal/StoreLocal across unboxable temps, typed BinOp
//!    (add/sub/mul/div/floordiv/mod), relational Compare, Truthy), and
//! 2. the only edge from OUTSIDE the stretch into a member lands on B_i
//!    (single entry — checked over predecessor sets).
//!
//! Calls, global/attr loads, exception machinery — anything raiseable or
//! observationally boxed — never become members: the region body cannot
//! raise (failures are guard side-exits), so the fast path is
//! side-effect-clean until it re-enters the twin.
//!
//! Membership is INDEPENDENTLY runtime-guarded (optimizing.rs + clif.rs):
//! every region load of a local goes through the value-level smi guard and
//! every typed op is exactness-checked, so the finder is a pure
//! optimization hint — a stale or torn feedback read can only cost a
//! side-exit, never a wrong answer. Determinism is pinned by tests (same
//! CFG + inference → identical region bitmap across recompiles).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::cfg::{ConstValue, Func, Inst, Terminator};
use crate::clif::op_code;
use crate::infer::{InferInfo, Inferred};
use crate::runtime as rt;

/// One typed region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedRegion {
    /// The single entry block (region id == its BlockId index).
    pub entry: crate::cfg::BlockId,
    /// Member blocks (indices into Func::blocks), ascending.
    pub blocks: Vec<u32>,
}

impl TypedRegion {
    pub fn contains(&self, b: u32) -> bool {
        self.blocks.contains(&b)
    }
}

/// Whether one BinOp opcode has i64-native typed semantics (the +-*/ plus
/// floordiv/mod family; div-zero and range failures side-exit). Shifts and
/// bitops are excluded from the v0 slice (rhs-range guards are subtler;
/// they land behind a dedicated probe later).
pub fn typed_binop(code: i64) -> bool {
    matches!(
        code,
        rt::OP_ADD | rt::OP_SUB | rt::OP_MUL | rt::OP_DIV | rt::OP_FLOORDIV | rt::OP_MOD
    )
}

/// Whether a block may be a REGION MEMBER given the inferred temp types.
fn block_eligible(func: &Func, bi: usize, info: &InferInfo) -> bool {
    let block = &func.blocks[bi];
    for inst in &block.insts {
        match inst {
            // Local traffic between unboxable temps is free (the LOAD side
            // is runtime-guarded; see infer.rs's SmiLocal admission).
            Inst::LoadLocal(_, d) => {
                if !temp_unboxable(info, *d) {
                    return false;
                }
            }
            Inst::StoreLocal(s, _) => {
                if !temp_unboxable(info, *s) {
                    return false;
                }
            }
            Inst::Const(ConstValue::Int(v), _) if crate::infer::is_smi(*v) => {}
            // Typed arithmetic: both operands must be unboxable.
            Inst::BinOp(op, a, b, _) => {
                if !typed_binop(op_code(*op))
                    || !temp_unboxable(info, *a)
                    || !temp_unboxable(info, *b)
                {
                    return false;
                }
            }
            // Relational compare over unboxable operands: a 0/1 bit.
            Inst::Compare(op, a, b, _) => {
                if !crate::infer::relational(*op)
                    || !temp_unboxable(info, *a)
                    || !temp_unboxable(info, *b)
                {
                    return false;
                }
            }
            // Branch conditions come from region compares or truthiness of
            // unboxable temps (int truthiness is `!= 0`).
            Inst::Truthy(a, _) => {
                if !temp_unboxable(info, *a) {
                    return false;
                }
            }
            // Anything observable or raiseable: out. (Non-smi constants
            // — Str/Float/Bool/None — are boxed protocol values: out.)
            Inst::Call(..)
            | Inst::LoadGlobal(..)
            | Inst::LoadAttr(..)
            | Inst::StoreAttr(..)
            | Inst::Subscript(..)
            | Inst::Print(..)
            | Inst::PushExcInfo(_)
            | Inst::PopExcInfo(_)
            | Inst::MatchExc(..)
            | Inst::Reraise
            | Inst::SuspendMarker
            | Inst::OsrPoll
            | Inst::UnaryOp(..)
            | Inst::Const(..) => return false,
        }
    }
    // Terminator: Goto/Branch/Return are internal edges or typed exits;
    // Raise and Suspend disqualify (exception/generator machinery is
    // boxed). A Return of a BOOL-typed temp must box as `True`/`False`,
    // not 0/1 — return-shape rule: only Int-typed returns are eligible.
    let ret_shape_ok = match &block.term {
        Some(Terminator::Return(v)) => !temp_is_bool(info, *v),
        _ => true,
    };
    ret_shape_ok
        && matches!(
            block.term,
            Some(Terminator::Goto(_)) | Some(Terminator::Branch(..)) | Some(Terminator::Return(_))
        )
}

fn temp_unboxable(info: &InferInfo, t: crate::cfg::TempId) -> bool {
    info.temps
        .get(t.0 as usize)
        .copied()
        .unwrap_or(Inferred::Unknown)
        .unboxable()
}

fn temp_is_bool(info: &InferInfo, t: crate::cfg::TempId) -> bool {
    info.temps
        .get(t.0 as usize)
        .copied()
        .unwrap_or(Inferred::Unknown)
        .is_bool()
}

/// Find the maximal single-entry typed regions (pon `find_regions`
/// analogue). Deterministic: blocks scanned in order, candidates grown
/// maximally, the single-entry rule enforced by shrinking from the right.
pub fn find_regions(func: &Func, info: &InferInfo) -> Vec<TypedRegion> {
    let n = func.blocks.len();
    let eligible: Vec<bool> = (0..n).map(|bi| block_eligible(func, bi, info)).collect();

    // Predecessor sets (for the single-entry check).
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (bi, b) in func.blocks.iter().enumerate() {
        let mut add = |t: u32| {
            if !preds[t as usize].contains(&(bi as u32)) {
                preds[t as usize].push(bi as u32);
            }
        };
        match &b.term {
            Some(Terminator::Goto(t)) => add(t.0),
            Some(Terminator::Branch(_, t, f)) => {
                add(t.0);
                add(f.0);
            }
            _ => {}
        }
    }

    let mut regions = Vec::new();
    let mut i = 0usize;
    while i < n {
        if !eligible[i] {
            i += 1;
            continue;
        }
        // Grow the maximal contiguous eligible stretch.
        let mut j = i;
        while j + 1 < n && eligible[j + 1] {
            j += 1;
        }
        // Single-entry: every predecessor of an INTERIOR member (i+1..=j)
        // must itself be a member; else shrink from the right.
        let mut end = j;
        while end > i {
            let interior_ok = ((i + 1)..=end).all(|m| {
                preds[m]
                    .iter()
                    .all(|p| (*p as usize) >= i && (*p as usize) <= end)
            });
            if interior_ok {
                break;
            }
            end -= 1;
        }
        let members: Vec<u32> = (i..=end).map(|m| m as u32).collect();
        regions.push(TypedRegion {
            entry: crate::cfg::BlockId(i as u32),
            blocks: members,
        });
        i = end + 1;
    }
    regions
}

/// The full region plan: regions + why ineligible blocks were left boxed
/// (report, never silence — leaf Risk discipline).
#[derive(Debug, Clone, Default)]
pub struct RegionPlan {
    pub regions: Vec<TypedRegion>,
    /// Blocks deliberately left boxed, with the reason (first
    /// disqualifier). Diagnostic surface only.
    pub boxed_blocks: Vec<(u32, &'static str)>,
}

pub fn plan_regions(func: &Func, info: &InferInfo) -> RegionPlan {
    let regions = find_regions(func, info);
    let mut boxed_blocks = Vec::new();
    for (bi, block) in func.blocks.iter().enumerate() {
        if regions.iter().any(|r| r.contains(bi as u32)) {
            continue;
        }
        boxed_blocks.push((bi as u32, first_disqualifier(block, info)));
    }
    RegionPlan {
        regions,
        boxed_blocks,
    }
}

fn first_disqualifier(block: &crate::cfg::Block, info: &InferInfo) -> &'static str {
    for inst in &block.insts {
        let why = match inst {
            Inst::Call(..) => "call",
            Inst::LoadGlobal(..) => "global load",
            Inst::LoadAttr(..) => "attr load",
            Inst::StoreAttr(..) => "attr store",
            Inst::Subscript(..) => "subscript",
            Inst::Print(..) => "print",
            Inst::PushExcInfo(_) | Inst::PopExcInfo(_) | Inst::MatchExc(..) | Inst::Reraise => {
                "exception machinery"
            }
            Inst::SuspendMarker => "generator suspend",
            Inst::OsrPoll => "osr poll",
            Inst::Const(..) => "non-smi constant",
            Inst::BinOp(op, ..) if !typed_binop(op_code(*op)) => "untyped binop",
            Inst::BinOp(_, a, ..) if !temp_unboxable(info, *a) => "untyped binop operand",
            _ => continue,
        };
        return why;
    }
    match block.term {
        Some(Terminator::Raise(..)) => "raise edge",
        Some(Terminator::Suspend(..)) => "suspend edge",
        Some(Terminator::Unreachable) | None => "unterminated",
        _ => "mixed eligibility",
    }
}

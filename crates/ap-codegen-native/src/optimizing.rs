//! 02.05c typed codegen: the OptimizingPlan + tier-1 fast-path metadata.
//!
//! pon analogue: pon-codegen/src/optimizing.rs `plan_function` producing
//! `OptimizingPlan {region, entry_guards, fast_path, cold_twin}`, plus the
//! re-boxing exits (`exits_requiring_boxing` / `ensure_boxed_value`
//! analogues). Adapted from pon (github.com/can1357/pon). Copyright
//! Can Bölük, MIT/Apache-2.0; adapted for antipythony under the granted
//! dual license with attribution.
//!
//! THE COLD TWIN (load-bearing invariant, PROGRESS-0205 Risk-3): every
//! tier-1 compilation carries BOTH paths — the typed fast path AND the
//! boxed baseline of the SAME cfg::Func (clif.rs `compile_baseline`). A
//! guard failure at a region load or a typed-op exactness failure branches
//! to the twin AT THE FAILED BLOCK (the twin_start seam, runtime.rs), which
//! recomputes with exact baseline semantics; execution then flows on
//! unchanged. Tier-1 is only ever a SUPPLEMENT: no output depends on a
//! guard passing. (02.05d adds the thrash backoff on top of the same
//! side-exit edge; true state-materialization deopt stays out of scope —
//! study problem #3.)
//!
//! Unboxed repr (the leaf's one genuinely open design call — DECIDED, see
//! PROGRESS-0205c): an i64 CLIF register admitted only within ap-obj's smi
//! domain (`Int::Smi`, [-2^59, 2^59)); guards are VALUE-level type+range
//! checks re-run on every region load (nothing caches across executions —
//! so unlike pon this pipeline consumes no version counters; the 02.05b
//! counters remain for the future IC-consuming codegen). Float/str reprs
//! are explicitly deferred; the pipeline is repr-agnostic by construction.
//! Consequence: no pointer-typed CLIF value exists even in tier-1 — all
//! boxed state stays in ctx.slots write-through — so the 02.05g
//! gc_value_declarations set stays EMPTY and the conservative GC floor
//! remains the sole root story; the ap-gc registry machinery stays ready
//! for a future pointer repr.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::cfg::{Func, TempId};
use crate::infer::{InferInfo, Inferred};
use crate::region::{plan_regions, RegionPlan, TypedRegion};

/// Entry guards for one typed region (pon `entry_guards` analogue): the
/// ctx slots the region LOADS from boxed storage through the runtime smi
/// guard (`h_slot_smi_v`). The guard order is slot-index ascending
/// (deterministic emission); `constants` pins the in-region smi literals
/// (documented; they need no runtime guard — statically exact).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntryGuards {
    /// ctx slot indices loaded through the guard, ascending.
    pub guarded_slots: Vec<u32>,
    /// In-region smi literals (temp id → literal), ascending by temp.
    pub constants: Vec<(u32, i64)>,
}

/// Fast-path metadata the typed emitter consumes.
#[derive(Debug, Clone, Default)]
pub struct FastPathMeta {
    /// Per-region guard plans (parallel to `regions`).
    pub guards: Vec<EntryGuards>,
    /// Every unboxable temp in the region set (the parallel-unboxed-slot
    /// universe; deterministic ascending order).
    pub typed_temps: Vec<u32>,
}

/// One planned tier-1 compilation: typed regions + guards + the cold twin.
pub struct OptimizingPlan {
    /// The single-entry typed regions (region-finder output).
    pub regions: Vec<TypedRegion>,
    /// Emitter metadata (guards + typed-temp universe).
    pub fast_path: FastPathMeta,
    /// The boxed cold twin (always compiled — the load-bearing invariant).
    pub cold_twin: crate::clif::JitFunc,
    /// Diagnostics: blocks deliberately left boxed, with reasons.
    pub boxed_blocks: Vec<(u32, &'static str)>,
    /// The inference input (retained for tests/verification).
    pub infer_info: InferInfo,
}

/// Build the full OptimizingPlan (pon `plan_function` analogue).
/// Infallible: blocks that cannot be typed simply stay on the twin.
pub fn plan_function(
    func: &Func,
    info: InferInfo,
    cold_twin: crate::clif::JitFunc,
) -> OptimizingPlan {
    let RegionPlan {
        mut regions,
        boxed_blocks,
    } = plan_regions(func, &info);
    // FAST-ENTRY SOUNDNESS (02.05c gate fix): the typed fast path enters
    // the region AT ITS ENTRY BLOCK from the function entry — value guards
    // establish TYPES, but they cannot establish the BRANCH CONDITION that
    // guards a mid-function entry (e.g. the `count = 0` then-arm of
    // `if count is None`). A region whose entry is a branch target would
    // execute unconditionally on the fast path, corrupting control flow
    // (live repro: functools/argparse corpus, two-region bodies flipping
    // results on call 2). pon enters typed regions ONLY at function entry
    // (guards on parameters); we adopt the same discipline: keep regions
    // rooted at block 0, demote everything else to the twin. The finder's
    // other regions remain REPORTED (never silently dropped).
    regions.retain(|r| r.entry.0 == 0);
    let mut guards = Vec::with_capacity(regions.len());
    let mut typed_temps_set = std::collections::BTreeSet::new();
    for region in &regions {
        let guarded = guarded_slots(func, region, &info);
        let constants = region_constants(func, region, &info);
        for t in region_temps(func, region) {
            if temp_unboxable(&info, t) {
                typed_temps_set.insert(t.0);
            }
        }
        guards.push(EntryGuards {
            guarded_slots: guarded,
            constants,
        });
    }
    OptimizingPlan {
        regions,
        fast_path: FastPathMeta {
            guards,
            typed_temps: typed_temps_set.into_iter().collect(),
        },
        cold_twin,
        boxed_blocks,
        infer_info: info,
    }
}

/// The ctx slot indices the region loads through the guard: locals read by
/// `LoadLocal` inside the region (the SmiLocal admission is guard-backed).
fn guarded_slots(func: &Func, region: &TypedRegion, _info: &InferInfo) -> Vec<u32> {
    let mut slots = std::collections::BTreeSet::new();
    for &b in &region.blocks {
        for inst in &func.blocks[b as usize].insts {
            if let crate::cfg::Inst::LoadLocal(src, _) = inst {
                slots.insert(src.0);
            }
        }
    }
    slots.into_iter().collect()
}

/// In-region smi literals (temp id → literal), deterministic order.
fn region_constants(func: &Func, region: &TypedRegion, _info: &InferInfo) -> Vec<(u32, i64)> {
    let mut out = Vec::new();
    for &b in &region.blocks {
        for inst in &func.blocks[b as usize].insts {
            if let crate::cfg::Inst::Const(crate::cfg::ConstValue::Int(v), d) = inst {
                if crate::infer::is_smi(*v) {
                    out.push((d.0, *v));
                }
            }
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

/// Every temp referenced by the region's member blocks.
fn region_temps(func: &Func, region: &TypedRegion) -> Vec<TempId> {
    let mut seen = std::collections::BTreeSet::new();
    for &b in &region.blocks {
        for inst in &func.blocks[b as usize].insts {
            for t in temps_of(inst) {
                seen.insert(t);
            }
        }
    }
    seen.into_iter().collect()
}

/// Live-out temps at one region exit: typed temps defined inside the
/// region and referenced by the target block (the twin needs them boxed).
/// The emitter re-boxes this set at the exit landing block.
pub fn live_out_temps(
    func: &Func,
    region: &TypedRegion,
    target: u32,
    info: &InferInfo,
) -> Vec<u32> {
    let defined: std::collections::BTreeSet<u32> = region
        .blocks
        .iter()
        .flat_map(|&b| writes_in_block(&func.blocks[b as usize]))
        .filter(|&t| {
            info.temps
                .get(t as usize)
                .copied()
                .unwrap_or(Inferred::Unknown)
                .unboxable()
        })
        .collect();
    let target_block = &func.blocks[target as usize];
    let mut out = Vec::new();
    for inst in &target_block.insts {
        for t in temps_of(inst) {
            if defined.contains(&t.0) && !out.contains(&t.0) {
                out.push(t.0);
            }
        }
    }
    if let Some(t) = term_temp(target_block) {
        if defined.contains(&t.0) && !out.contains(&t.0) {
            out.push(t.0);
        }
    }
    out.sort_unstable();
    out
}

fn writes_in_block(block: &crate::cfg::Block) -> Vec<u32> {
    let mut out = Vec::new();
    for inst in &block.insts {
        if let Some(t) = writes_of(inst) {
            out.push(t.0);
        }
    }
    out
}

fn term_temp(block: &crate::cfg::Block) -> Option<TempId> {
    match &block.term {
        Some(crate::cfg::Terminator::Branch(c, _, _)) => Some(*c),
        Some(crate::cfg::Terminator::Return(v) | crate::cfg::Terminator::Raise(v, _)) => Some(*v),
        _ => None,
    }
}

fn writes_of(inst: &crate::cfg::Inst) -> Option<TempId> {
    use crate::cfg::Inst;
    match inst {
        Inst::StoreLocal(s, _) => Some(*s),
        Inst::LoadLocal(_, d) => Some(*d),
        Inst::Const(_, d)
        | Inst::LoadGlobal(_, d)
        | Inst::UnaryOp(_, _, d)
        | Inst::LoadAttr(_, _, d)
        | Inst::Subscript(_, _, d)
        | Inst::Truthy(_, d)
        | Inst::BinOp(_, _, _, d)
        | Inst::Compare(_, _, _, d)
        | Inst::Call(_, _, d)
        | Inst::MatchExc(_, _, d) => Some(*d),
        _ => None,
    }
}

fn temps_of(inst: &crate::cfg::Inst) -> Vec<TempId> {
    use crate::cfg::Inst;
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

fn temp_unboxable(info: &InferInfo, t: TempId) -> bool {
    info.temps
        .get(t.0 as usize)
        .copied()
        .unwrap_or(Inferred::Unknown)
        .unboxable()
}

/// Whether the plan has any typed region at all (a plan without regions is
/// equivalent to running the twin alone — the caller skips the install).
pub fn has_typed_regions(plan: &OptimizingPlan) -> bool {
    !plan.regions.is_empty()
}

/// Terminator-shape sanity for the region exits used by the typed emitter
/// (fail-closed at plan time: an unexpected terminator inside a region is
/// a finder bug, surfaced loudly).
pub fn verify_region_terminators(
    func: &Func,
    plan: &OptimizingPlan,
) -> Result<(), crate::CodegenError> {
    for region in &plan.regions {
        for &b in &region.blocks {
            let term = func.blocks[b as usize].term.as_ref().ok_or_else(|| {
                crate::CodegenError::Verify("region block unterminated".to_string())
            })?;
            match term {
                crate::cfg::Terminator::Goto(_)
                | crate::cfg::Terminator::Branch(..)
                | crate::cfg::Terminator::Return(_) => {}
                other => {
                    return Err(crate::CodegenError::Verify(format!(
                        "region {} member block {b} ends in {other:?} — finder bug",
                        region.entry.0
                    )))
                }
            }
        }
    }
    Ok(())
}

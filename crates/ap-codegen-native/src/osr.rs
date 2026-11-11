//! Boxed OSR: buffer + transfer + entry (pon OSR-by-call adaptation from
//! pon-runtime/src/tierup.rs + pon-codegen baseline control.rs).
//! Copyright Can Bölük, MIT/Apache-2.0; adapted for antipythony under the
//! granted dual license with attribution.
//!
//! Design (leaf scope item 6, Risk 2 phase 1 — BOXED OSR ONLY; the
//! typed-region extension is an ordered follow-up, live-set-capped):
//!
//! - Tier-0 emits an OSR poll at every loop back-edge (the cfg stage puts
//!   `Inst::OsrPoll` there). The poll bumps loop hotness and, when a
//!   compiled OSR entry exists for this (function, loop-header) pair,
//!   RETURNS the entry pointer.
//! - Transfer is OSR-BY-CALL: tier-0 builds an [`OsrTransferBuffer`] in an
//!   explicit stack slot, calls the OSR entry indirectly with it, and the
//!   OSR body runs to completion and returns the function's final boxed
//!   result, which tier-0 forwards unchanged. NO frame transplant; the
//!   tier-0 frame is simply abandoned after the call.
//! - Slot order is canonical: all locals in LocalId order (NULL when
//!   unbound), then live-in values at the header in ascending TempId.
//! - Live sets > [`OSR_MAX_LIVE`] stay OSR-INELIGIBLE (documented; the
//!   eligibility gate is checked and reported, never silently dropped).
//! - OSR entry frames are never GC-map-registered; they stay on the
//!   conservative-scan path (pon discipline; pairs with the GC roots shim
//!   decision in docs/gc-mmtk-swap.md §4).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::cfg::{BlockId, Func, Inst, LocalId, TempId};
use ap_obj::value::Value;

/// Fixed-size transfer buffer cap (pon `OSR_MAX_LIVE = 16`). Functions
/// whose (locals + live-in) exceed this at any loop header are
/// OSR-ineligible — an explicit, reported gate, not a silent drop.
pub const OSR_MAX_LIVE: usize = 16;

/// `repr(C)` transfer buffer (pon shape): tier-0 writes, OSR entry reads.
/// Slots carry BOXED values directly ([`Value`] — 16 bytes each; both
/// sides of the transfer are Rust compiled from THIS crate, so the layout
/// is shared by construction — the v0 usize-slot idea died on Value being
/// two words, see the leaf report).
#[repr(C)]
pub struct OsrTransferBuffer {
    /// The loop header this transfer enters at (cfg BlockId raw).
    pub loop_header: u32,
    /// Number of populated slots.
    pub live_count: u32,
    /// Boxed values: locals first (LocalId order), then live-ins.
    pub slots: [Value; OSR_MAX_LIVE],
}

const _BUFFER_SIZE_ASSERT: () = assert!(
    std::mem::size_of::<OsrTransferBuffer>() == 8 + 16 * OSR_MAX_LIVE,
    "OsrTransferBuffer layout: header + OSR_MAX_LIVE boxed Values"
);

impl OsrTransferBuffer {
    /// A zeroed buffer with `live_count = 0` (tier-0 fills before the call).
    pub fn zeroed() -> Self {
        OsrTransferBuffer {
            loop_header: 0,
            live_count: 0,
            slots: std::array::from_fn(|_| Value::None),
        }
    }
}

/// OSR eligibility for one loop header: canonical slot plan (locals +
/// live-in set), or an explicit ineligibility report.
#[derive(Debug, Clone, PartialEq)]
pub enum OsrEligibility {
    Eligible {
        /// Locals transferred, in LocalId order.
        locals: Vec<LocalId>,
        /// Live-in temps at the header, ascending TempId order.
        live_in: Vec<TempId>,
    },
    Ineligible {
        /// Why (reported, never silent — leaf Risk-2 gate).
        reason: &'static str,
        live_count: usize,
    },
}

/// Compute the OSR slot plan for `header` from CFG liveness.
///
/// Liveness is plain backward dataflow over the CFG (pon
/// `osr_live_values` analogue). The v0 liveness is CONSERVATIVE: a temp is
/// live-in at the header if any path from the header reads it before
/// definition (computed by a simple forward scan; refined to true
/// backward dataflow when the typed-OSR extension lands).
pub fn osr_plan(func: &Func, header: BlockId) -> OsrEligibility {
    let locals: Vec<LocalId> = (0..func.num_locals).map(LocalId).collect();
    let live_in = live_in_at(func, header);
    let total = locals.len() + live_in.len();
    if total > OSR_MAX_LIVE {
        return OsrEligibility::Ineligible {
            reason: "live set exceeds OSR_MAX_LIVE (documented cap; loop stays tier-0)",
            live_count: total,
        };
    }
    OsrEligibility::Eligible { locals, live_in }
}

/// Temps read on some path from `header` before being redefined (v0
/// conservative forward scan; fixed-point over the reachable subset).
fn live_in_at(func: &Func, header: BlockId) -> Vec<TempId> {
    // defined-then-read tracking: a temp is live-in if read on a path
    // where the last def before the read is outside the region.
    let mut live = std::collections::BTreeSet::new();
    let mut defined = std::collections::BTreeSet::new();
    let mut stack = vec![header];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(b) = stack.pop() {
        if !visited.insert(b) {
            continue;
        }
        let block = &func.blocks[b.0 as usize];
        for inst in &block.insts {
            for read in reads_of(inst) {
                if !defined.contains(&read) {
                    live.insert(read);
                }
            }
            for w in writes_of(inst) {
                defined.insert(w);
            }
        }
        match &block.term {
            Some(crate::cfg::Terminator::Goto(t)) => stack.push(*t),
            Some(crate::cfg::Terminator::Branch(_, t, f)) => {
                stack.push(*t);
                stack.push(*f);
            }
            _ => {}
        }
    }
    live.into_iter().collect()
}

/// A canonical OSR slot plan (the shared constant table the split's Risk
/// note demands): ONE mapping consumed by BOTH directions —
/// - transfer emit (clif.rs's `emit_cfg` OsrPoll arm reads
///   [`OsrPlan::slot_of_value`] to fill the buffer), and
/// - entry load (`compile_osr_entry` reads the SAME plan to read the
///   buffer back into slot space).
///
/// The mapping is a pure function of (cfg, header); tests pin both
/// directions against it (tests/osr_tests.rs).
#[derive(Debug, Clone, PartialEq)]
pub struct OsrPlan {
    /// Locals transferred, in LocalId order — buffer slots [0..locals.len()).
    pub locals: Vec<LocalId>,
    /// Live-in temps at the header, ascending TempId — buffer slots
    /// [locals.len()..locals.len()+live_in.len()).
    pub live_in: Vec<TempId>,
    /// The header this plan enters (informational; the buffer header word).
    pub header: BlockId,
}

impl OsrPlan {
    /// Buffer slot index of a boxed value (LocalId or TempId), or None
    /// when the value is not part of the transfer set (a def'd temp).
    /// ONE definition — transfer emit and entry load cannot drift.
    pub fn slot_of_local(&self, l: LocalId) -> Option<usize> {
        self.locals.iter().position(|&x| x == l)
    }

    pub fn slot_of_temp(&self, t: TempId) -> Option<usize> {
        self.live_in.iter().position(|&x| x == t)
    }

    pub fn live_count(&self) -> usize {
        self.locals.len() + self.live_in.len()
    }
}

/// Resolve an eligibility into a plan (the Eligible arm's payload, kept
/// as one struct so the emitter and the entry compiler share the table).
pub fn osr_plan_resolved(func: &Func, header: BlockId) -> Option<OsrPlan> {
    match osr_plan(func, header) {
        OsrEligibility::Eligible { locals, live_in } => Some(OsrPlan {
            locals,
            live_in,
            header,
        }),
        OsrEligibility::Ineligible { .. } => None,
    }
}

/// Every loop-header BlockId the cfg lowers an OsrPoll for (the OSR-entry
/// compile's work list). Polls are back-edge markers placed by the cfg
/// stage inside loop-body blocks; a poll's header is the block whose
/// (back) edge targets it — v0 shape: while-loops branch back to the
/// CONDITION block that branches into the poll's body block, so the
/// header of a poll in block `p` is the unique in-loop predecessor of
/// `p` (single-back-edge loops only; multi-back-edge bodies stay
/// ineligible and are reported by the caller's compile path).
pub fn loop_headers(func: &Func) -> Vec<BlockId> {
    let mut headers = Vec::new();
    for (pi, block) in func.blocks.iter().enumerate() {
        let has_poll = block.insts.iter().any(|i| matches!(i, Inst::OsrPoll));
        if !has_poll {
            continue;
        }
        // Predecessors of the poll's block.
        let mut preds: Vec<u32> = Vec::new();
        for b in func.blocks.iter() {
            let mut add = |t: u32| {
                if !preds.contains(&t) {
                    preds.push(t);
                }
            };
            match &b.term {
                Some(crate::cfg::Terminator::Goto(t)) => add(t.0),
                Some(crate::cfg::Terminator::Branch(_, t, f)) => {
                    add(t.0);
                    add(f.0);
                }
                _ => {}
            }
        }
        // The header is the predecessor that itself reaches the poll block
        // again through the loop (the back-edge source). For the while
        // shape the cfg emits, the body block's in-loop predecessor is the
        // condition re-evaluation edge from the body's end — the header is
        // the CONDITION block: the unique predecessor reachable FROM the
        // poll block. Walk successors of the poll block; whichever
        // predecessor lies on that forward path is the header.
        let header = preds.iter().copied().find(|&p| {
            // p must be able to reach `pi` forward (it re-enters the loop)
            reaches(func, p, pi as u32)
        });
        if let Some(h) = header {
            headers.push(BlockId(h));
        }
    }
    headers.sort_unstable();
    headers.dedup();
    headers
}

/// Forward reachability (small CFGs; a plain DFS per query is fine).
fn reaches(func: &Func, from: u32, to: u32) -> bool {
    if from == to {
        return true;
    }
    let mut stack = vec![from];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(b) = stack.pop() {
        if !seen.insert(b) {
            continue;
        }
        let block = &func.blocks[b as usize];
        let mut succs: Vec<u32> = Vec::new();
        match &block.term {
            Some(crate::cfg::Terminator::Goto(t)) => succs.push(t.0),
            Some(crate::cfg::Terminator::Branch(_, t, f)) => {
                succs.push(t.0);
                succs.push(f.0);
            }
            _ => {}
        }
        for s in succs {
            if s == to {
                return true;
            }
            stack.push(s);
        }
    }
    false
}

fn reads_of(i: &Inst) -> Vec<TempId> {
    match i {
        Inst::LoadLocal(_, d)
        | Inst::Const(_, d)
        | Inst::LoadGlobal(_, d)
        | Inst::LoadAttr(_, _, d)
        | Inst::Subscript(_, _, d)
        | Inst::Truthy(_, d)
        | Inst::UnaryOp(_, _, d) => vec![*d],
        Inst::StoreLocal(s, _) => vec![*s],
        Inst::StoreAttr(o, _, v) => vec![*o, *v],
        Inst::BinOp(_, a, b, d) | Inst::Compare(_, a, b, d) => vec![*a, *b, *d],
        Inst::Call(f, args, d) => {
            let mut r = vec![*f, *d];
            r.extend(args.iter().copied());
            r
        }
        Inst::MatchExc(_, _, d) => vec![*d],
        Inst::Print(s) => vec![*s],
        Inst::PushExcInfo(_)
        | Inst::PopExcInfo(_)
        | Inst::Reraise
        | Inst::SuspendMarker
        | Inst::OsrPoll => vec![],
    }
}

fn writes_of(i: &Inst) -> Vec<TempId> {
    match i {
        Inst::LoadLocal(_, d)
        | Inst::Const(_, d)
        | Inst::LoadGlobal(_, d)
        | Inst::LoadAttr(_, _, d)
        | Inst::Subscript(_, _, d)
        | Inst::Truthy(_, d)
        | Inst::UnaryOp(_, _, d)
        | Inst::BinOp(_, _, _, d)
        | Inst::Compare(_, _, _, d)
        | Inst::Call(_, _, d)
        | Inst::MatchExc(_, _, d) => vec![*d],
        _ => vec![],
    }
}

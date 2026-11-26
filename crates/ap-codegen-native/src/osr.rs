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
/// hp-forloop: the v0 CONSERVATIVE forward scan is replaced with TRUE
/// backward dataflow (pon `osr_live_values` discipline — iterate to the
/// fixed point over the CFG's use/def sets). A temp is live-in at `header`
/// iff some path from `header` READS it before any redefinition; the
/// forward scan over-approximated this (every read-anywhere temp counted),
/// which pushed for-loop bodies (iterator handle + exhausted-flag temps)
/// past OSR_MAX_LIVE and silently killed OSR eligibility for exactly the
/// shapes this leaf gates. Locals-only refinement stays as-is: cells are
/// excluded by construction (the v0 slice has no cell locals).
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

/// True backward liveness over temps at one block entry (the worklist
/// fixed-point of `live_out(b) = ∪ live_in(succ)` / `live_in(b) =
/// use(b) ∪ (live_out(b) − def(b))`). Temps are block-local in the
/// statement-tree lowering EXCEPT values carried across back-edges (the
/// iterator handle, loop-carried phis), which is exactly what this pass
/// recovers: the fixed point propagates the reads at the loop's exit/join
/// blocks back around the back-edge to the header.
fn live_in_at(func: &Func, header: BlockId) -> Vec<TempId> {
    let n = func.blocks.len();
    // Per-block use (read before any def in-block) and def sets. NOTE: the
    // shared reads_of() lists an inst's DESTINATION as a read (deliberately
    // conservative for the old forward scan); true liveness needs the
    // precise form — an inst's dst is a DEF, its operands are USES.
    fn true_reads_writes(i: &Inst) -> (Vec<TempId>, Vec<TempId>) {
        match i {
            Inst::LoadLocal(_, d)
            | Inst::Const(_, d)
            | Inst::LoadGlobal(_, d)
            | Inst::Truthy(_, d)
            | Inst::UnaryOp(_, _, d)
            | Inst::MatchExc(_, _, d) => (vec![], vec![*d]),
            Inst::LoadAttr(o, _, d) | Inst::Subscript(o, _, d) => (vec![*o], vec![*d]),
            Inst::StoreLocal(s, _) => (vec![*s], vec![]),
            Inst::StoreAttr(o, _, v) => (vec![*o, *v], vec![]),
            Inst::StoreSubscript(o, k, v) => (vec![*o, *k, *v], vec![]),
            Inst::BinOp(_, a, b, d) | Inst::Compare(_, a, b, d) => (vec![*a, *b], vec![*d]),
            Inst::Call(f, args, d) => {
                let mut r = vec![*f];
                r.extend(args.iter().copied());
                (r, vec![*d])
            }
            Inst::Print(s) => (vec![*s], vec![]),
            Inst::IterNew(src, d) => (vec![*src], vec![*d]),
            Inst::IterNext(it, d, st) => (vec![*it], vec![*d, *st]),
            Inst::PushExcInfo(_)
            | Inst::PopExcInfo(_)
            | Inst::Reraise
            | Inst::SuspendMarker
            | Inst::OsrPoll => (vec![], vec![]),
        }
    }
    let mut use_sets: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); n];
    let mut def_sets: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); n];
    for (bi, block) in func.blocks.iter().enumerate() {
        let (u, d) = (&mut use_sets[bi], &mut def_sets[bi]);
        for inst in &block.insts {
            let (reads, writes) = true_reads_writes(inst);
            for r in reads {
                if !d.contains(&r.0) {
                    u.insert(r.0);
                }
            }
            for w in writes {
                d.insert(w.0);
            }
        }
        // A branch condition / return / raise operand is a READ at the
        // block's end (after every inst's defs).
        let term_read = |t: &TempId, u: &mut std::collections::BTreeSet<u32>| {
            if !d.contains(&t.0) {
                u.insert(t.0);
            }
        };
        match &block.term {
            Some(crate::cfg::Terminator::Branch(c, _, _)) => term_read(c, u),
            Some(crate::cfg::Terminator::Return(v) | crate::cfg::Terminator::Raise(v, _)) => {
                term_read(v, u)
            }
            Some(crate::cfg::Terminator::Suspend(v, _, _)) => term_read(v, u),
            _ => {}
        }
    }
    // Successor lists.
    let mut succs: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (bi, b) in func.blocks.iter().enumerate() {
        let push = |t: u32, v: &mut Vec<u32>| {
            if !v.contains(&t) {
                v.push(t);
            }
        };
        match &b.term {
            Some(crate::cfg::Terminator::Goto(t)) => push(t.0, &mut succs[bi]),
            Some(crate::cfg::Terminator::Branch(_, t, f)) => {
                push(t.0, &mut succs[bi]);
                push(f.0, &mut succs[bi]);
            }
            _ => {}
        }
    }
    // Backward fixed point: live_in(b) = use(b) ∪ (live_out(b) − def(b)).
    let mut live_in: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); n];
    let mut live_out: Vec<std::collections::BTreeSet<u32>> =
        vec![std::collections::BTreeSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for bi in (0..n).rev() {
            let mut out = std::collections::BTreeSet::new();
            for s in &succs[bi] {
                out.extend(live_in[*s as usize].iter().copied());
            }
            if out != live_out[bi] {
                live_out[bi] = out;
                changed = true;
            }
            let mut inn = use_sets[bi].clone();
            for t in &live_out[bi] {
                if !def_sets[bi].contains(t) {
                    inn.insert(*t);
                }
            }
            if inn != live_in[bi] {
                live_in[bi] = inn;
                changed = true;
            }
        }
    }
    live_in[header.0 as usize]
        .iter()
        .map(|&t| TempId(t))
        .collect()
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
        Inst::StoreSubscript(o, k, v) => vec![*o, *k, *v],
        Inst::BinOp(_, a, b, d) | Inst::Compare(_, a, b, d) => vec![*a, *b, *d],
        Inst::Call(f, args, d) => {
            let mut r = vec![*f, *d];
            r.extend(args.iter().copied());
            r
        }
        // hp-forloop: IterNew reads the iterable (dst is the fresh handle);
        // IterNext reads the iterator handle and WRITES both dst (the
        // value) and st (the exhausted flag) — the st temp is the branch
        // condition of the loop exit, so it is a WRITE here (reads_of
        // only reports values defined BEFORE this inst).
        Inst::IterNew(src, _) => vec![*src],
        Inst::IterNext(it, _, _) => vec![*it],
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
        // hp-forloop: both results are fresh defs (value + exhausted flag).
        Inst::IterNew(_, d) => vec![*d],
        Inst::IterNext(_, d, st) => vec![*d, *st],
        _ => vec![],
    }
}

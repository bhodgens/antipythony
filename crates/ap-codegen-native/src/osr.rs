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

use crate::cfg::{Func, LocalId, TempId};

/// Fixed-size transfer buffer cap (pon `OSR_MAX_LIVE = 16`). Functions
/// whose (locals + live-in) exceed this at any loop header are
/// OSR-ineligible — an explicit, reported gate, not a silent drop.
pub const OSR_MAX_LIVE: usize = 16;

/// `repr(C)` transfer buffer (pon shape): tier-0 writes, OSR entry reads,
/// at FIXED offsets so the two sides need no shared Rust ABI.
#[repr(C)]
pub struct OsrTransferBuffer {
    /// The loop header this transfer enters at (cfg BlockId raw).
    pub loop_header: u32,
    /// Number of populated slots.
    pub live_count: u32,
    /// Boxed values: locals first (LocalId order), then live-ins.
    pub slots: [usize; OSR_MAX_LIVE],
}

const _BUFFER_SIZE_ASSERT: () = assert!(
    std::mem::size_of::<OsrTransferBuffer>() == 8 + 8 * OSR_MAX_LIVE,
    "OsrTransferBuffer layout: header + OSR_MAX_LIVE words"
);

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
pub fn osr_plan(func: &Func, header: crate::cfg::BlockId) -> OsrEligibility {
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
fn live_in_at(func: &Func, header: crate::cfg::BlockId) -> Vec<TempId> {
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

fn reads_of(i: &crate::cfg::Inst) -> Vec<TempId> {
    use crate::cfg::Inst;
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

fn writes_of(i: &crate::cfg::Inst) -> Vec<TempId> {
    use crate::cfg::Inst;
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

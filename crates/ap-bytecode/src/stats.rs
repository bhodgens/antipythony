//! Opt-in opcode-frequency histogram (fast-path 02-perf/02-superinstructions).
//!
//! Pure counting structure: records single opcodes and adjacent PAIR/TRIPLE
//! sequences at the dispatch boundary. No semantics, no `Value`, no deps.
//! The interpreter (ap-run) owns the one global instance and is the ONLY
//! caller that writes it — every write is gated on `APX_BP_STATS=1`, read
//! once per process. When the knob is off, NOTHING in this module is
//! touched: the dispatch loop's `if bp_on` branch is the only cost (one
//! perfectly-predicted branch, no atomic/alloc/lock).
//!
//! Jump targets are deliberately ignored: `prev` is the dynamic predecessor
//! at the dispatch boundary, so a loop back-edge produces the pair
//! (last-body-op, loop-head-op) exactly as a real profile sees it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::instr::OPCODE_NAMES;

/// Sentinel "no predecessor yet" (frame entry / just after a jump chain).
pub const NO_OP: u16 = u16::MAX;
/// Dispatch-counter capacity; must exceed the opcode count. Extra slots are
/// harmless (never recorded), and keep this module independent of future
/// opcode additions.
pub const N_OPCODES: usize = 64;

/// Cumulative opcode histogram for one process (global, relaxed counters).
pub struct OpHistogram {
    singles: Vec<AtomicU64>,
    pairs: Mutex<HashMap<(u16, u16), u64>>,
    triples: Mutex<HashMap<(u16, u16, u16), u64>>,
    total: AtomicU64,
}

impl Default for OpHistogram {
    fn default() -> Self {
        Self {
            singles: (0..N_OPCODES).map(|_| AtomicU64::new(0)).collect(),
            pairs: Mutex::new(HashMap::new()),
            triples: Mutex::new(HashMap::new()),
            total: AtomicU64::new(0),
        }
    }
}

impl OpHistogram {
    /// Record one dispatch of `cur` with dynamic predecessors `p1`/`p2`.
    #[inline]
    pub fn record(&self, cur: u16, p1: u16, p2: u16) {
        if (cur as usize) < self.singles.len() {
            self.singles[cur as usize].fetch_add(1, Ordering::Relaxed);
        }
        self.total.fetch_add(1, Ordering::Relaxed);
        if p1 != NO_OP && p2 != NO_OP {
            let mut t = self.triples.lock().unwrap_or_else(|e| e.into_inner());
            *t.entry((p2, p1, cur)).or_insert(0) += 1;
        }
        if p1 != NO_OP {
            let mut p = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
            *p.entry((p1, cur)).or_insert(0) += 1;
        }
    }

    /// Top-N lines for one sequence length, rendered like
    /// `(LoadFast, LoadFast)=1234`.
    fn top(map: &HashMap<Vec<u16>, u64>, n: usize) -> Vec<String> {
        let mut rows: Vec<_> = map.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        rows.into_iter()
            .take(n)
            .map(|(seq, count)| {
                let names: Vec<&str> = seq
                    .iter()
                    .map(|&i| OPCODE_NAMES.get(i as usize).copied().unwrap_or("<unknown>"))
                    .collect();
                format!("({})={}", names.join(", "), count)
            })
            .collect()
    }

    /// Emit a multi-line top-20 pair/triple table (plus top-10 singles).
    /// Only called from the opt-in exit hook.
    pub fn report(&self, top_n: usize) -> String {
        let total = self.total.load(Ordering::Relaxed);
        let pairs = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
        let triples = self.triples.lock().unwrap_or_else(|e| e.into_inner());
        let pair_map: HashMap<Vec<u16>, u64> =
            pairs.iter().map(|(&(a, b), &c)| (vec![a, b], c)).collect();
        let triple_map: HashMap<Vec<u16>, u64> = triples
            .iter()
            .map(|(&(a, b, c), &d)| (vec![a, b, c], d))
            .collect();
        let mut singles: Vec<_> = self
            .singles
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let v = c.load(Ordering::Relaxed);
                (v > 0).then_some((i, v))
            })
            .collect();
        singles.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let single_rows: Vec<String> = singles
            .iter()
            .take(10)
            .map(|(i, c)| {
                format!(
                    "{}={}",
                    OPCODE_NAMES.get(*i).copied().unwrap_or("<unknown>"),
                    c
                )
            })
            .collect();
        format!(
            "[BP-HIST] total_dispatches={total}\n\
             [BP-HIST] singles(top10): {}\n\
             [BP-HIST] pairs(top{}): {}\n\
             [BP-HIST] triples(top{}): {}",
            single_rows.join(" "),
            top_n,
            Self::top(&pair_map, top_n).join(" "),
            top_n,
            Self::top(&triple_map, top_n).join(" "),
        )
    }
}

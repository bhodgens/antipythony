//! The exception table (CPython 3.11+ zero-cost try ranges) — leaf
//! fp-exceptions (plans/fast-path/01-core/03-exceptions.md).
//!
//! One `ExcEntry` per lowered handler arc: the bytecode range
//! `[start, end)` it guards, the handler's entry pc, and the operand-
//! stack depth to restore on unwind (zero-cost non-exception path —
//! the table is consulted ONLY when an exception is in flight).
//!
//! Lowering shape (lower.rs): the table is filled IN EMISSION ORDER by
//! `begin_try`/`end_range` bracket calls, then post-sorted by start for
//! lookup (entries recorded in nesting order sort correctly — inner
//! nested ranges have distinct spans; equal starts keep the DEEPER
//! handler first via the `depth` tiebreak, which `find_handler` relies
//! on for nested-try ordering).
//!
//! No-ops (Pass-only try bodies) still emit a range with start == end —
//! `find_handler` treats empty ranges as guarded (a raise AT the
//! boundary pc dispatches to the handler, CPython's inclusive-range
//! behavior).

/// One guarded range -> handler arc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcEntry {
    /// First instruction index of the guarded range (inclusive).
    pub start: u32,
    /// One past the last guarded instruction (exclusive).
    pub end: u32,
    /// Handler arc entry pc.
    pub handler: u32,
    /// Operand-stack depth to restore on unwind.
    pub stack_depth: u32,
    /// Nesting depth at lowering (outer = 0); the tiebreak for equal
    /// spans (find_handler picks the DEEPEST matching range first,
    /// matching CPython's innermost-handler-first dispatch).
    pub depth: u16,
}

/// The per-function exception table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExcTable {
    entries: Vec<ExcEntry>,
}

impl ExcTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one entry (emission order; sorted lazily on first lookup
    /// — the lowerer brackets emission, so the table arrives unsorted).
    pub fn push(&mut self, e: ExcEntry) {
        self.entries.push(e);
    }

    /// Total entries (validate + tests).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[ExcEntry] {
        &self.entries
    }

    /// Sort by (start ASC, depth DESC) — find_handler's scan order.
    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then(b.depth.cmp(&a.depth))
                .then(a.handler.cmp(&b.handler))
        });
    }

    /// The handler arc for an exception raised at `pc`: the DEEPEST
    /// range containing pc wins (CPython's innermost-handler-first
    /// dispatch). None = no handler in this frame (propagate).
    pub fn find_handler(&self, pc: u32) -> Option<&ExcEntry> {
        let mut best: Option<&ExcEntry> = None;
        for e in &self.entries {
            let in_range = pc >= e.start && pc < e.end || (e.start == e.end && pc == e.start);
            if in_range && best.is_none_or(|b| b.depth < e.depth) {
                best = Some(e);
            }
        }
        best
    }

    /// Lowerer invariant check (validate): every handler pc and range
    /// bound is in bounds and well-formed (start <= end, pc != the
    /// unpatched sentinel).
    pub fn validate(&self, n_instr: usize) -> Result<(), String> {
        for e in &self.entries {
            if e.start as usize > n_instr
                || e.end as usize > n_instr
                || e.handler as usize >= n_instr
            {
                return Err(format!(
                    "exc entry out of bounds: start={} end={} handler={} n={}",
                    e.start, e.end, e.handler, n_instr
                ));
            }
            if e.start > e.end {
                return Err(format!("exc entry inverted range: {} > {}", e.start, e.end));
            }
        }
        Ok(())
    }
}

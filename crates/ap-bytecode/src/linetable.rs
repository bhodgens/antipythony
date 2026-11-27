//! Sparse line table: `(instr_idx -> line)`, entry only where the line
//! CHANGES.
//!
//! Replaces the tree-walker's per-node `last_line.set()`: the dispatch
//! loop does NOT touch lines; exception/warning paths consult
//! [`LineTable::line_for`] (which does the binary search the walker did
//! per-step, but only on the cold path).
//!
//! Invariant: `entries` strictly sorted by `instr_idx` (monotonic), each
//! line >= 1 (0 = unknown never recorded).

/// One sparse entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEntry {
    /// First instruction index at which `line` is current.
    pub instr_idx: u32,
    /// 1-based source line.
    pub line: u32,
}

/// Sparse sorted map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineTable {
    entries: Vec<LineEntry>,
}

impl LineTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that instruction `instr_idx` is at `line`. Only records when
    /// the line differs from the current tail entry (sparse discipline).
    /// `instr_idx` must be monotonic non-decreasing across calls (the
    /// lowerer appends in emission order); `line` must be >= 1.
    pub fn record(&mut self, instr_idx: u32, line: usize) {
        debug_assert!(line >= 1, "line 0 = unknown; never record it");
        if let Some(last) = self.entries.last_mut() {
            debug_assert!(
                instr_idx >= last.instr_idx,
                "line-table instr indices must be monotonic"
            );
            if last.instr_idx == instr_idx {
                // Same instruction re-recorded (a later subexpression on
                // the same instr): overwrite.
                last.line = line as u32;
                return;
            }
            if last.line == line as u32 {
                return; // line unchanged — stay sparse
            }
        }
        self.entries.push(LineEntry {
            instr_idx,
            line: line as u32,
        });
    }

    /// The line current at `instr_idx` (binary search). Instructions
    /// before the first entry report the FIRST entry's line; an empty
    /// table reports 0.
    pub fn line_for(&self, instr_idx: u32) -> u32 {
        match self.entries.partition_point(|e| e.instr_idx <= instr_idx) {
            0 => self.entries.first().map(|e| e.line).unwrap_or(0),
            i => self.entries[i - 1].line,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LineEntry> {
        self.entries.iter()
    }

    /// fp-superinstructions: rebuild from remapped instruction indices
    /// (peephole rewrite). Caller supplies entries in monotonic order.
    pub fn from_remapped<'a, I, F>(entries: I, mut map: F) -> Self
    where
        I: IntoIterator<Item = &'a LineEntry>,
        F: FnMut(u32) -> u32,
    {
        let mut t = Self::new();
        for e in entries {
            t.record(map(e.instr_idx), e.line as usize);
        }
        t
    }
}

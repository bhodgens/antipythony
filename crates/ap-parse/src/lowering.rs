//! AST→IR lowering (leaf 01.03).
//!
//! Converts the ruff AST held by [`crate::ModuleAst`] into [`ap_ir::Module`].
//! ap-parse is the single ruff-tainted crate (master vendor-clean rule), so
//! the conversion lives here and its public surface speaks only ap-ir.
//!
//! # Totality
//!
//! Lowering is total over programs `parse` accepts: there are no panic paths.
//! Internal invariant violations (the parser produced a node shape the
//! lowering cannot represent) return [`LoweringBug`], a distinct error from
//! [`crate::ParseError`] — expected to be unreachable on valid input.
//!
//! # Semantics preservation
//!
//! Every lowering step preserves observable semantics; the comment on each arm
//! cites the spec-pack clause (S-xx-yy) or compat-contract clause (C-xx) it
//! preserves. Structural choices that carry no observable semantics (e.g.
//! merging implicitly concatenated literals) cite the clause that makes the
//! choice unobservable.
//!
//! # Scope classification
//!
//! Alongside the IR, [`lower`] emits a [`ScopeInfoModule`]: for every
//! function-like scope (function, lambda, comprehension) the sets of
//! `global`-declared, `nonlocal`-bound, cell (shared with a nested scope) and
//! free (read from an enclosing function) names, per S-02-1..S-02-5,
//! S-02-12/S-02-13.

use std::collections::BTreeMap;
use std::fmt;

use ap_ir::expr::{CompClause, DictItem};
use ap_ir::{
    Arg, BinOp, BoolOp, Builder, CmpOp, Conversion, Expr, FStrPart, FnBody, ImportAlias, Item,
    MatchCase, Param, ParamKind, Pattern, Stmt, Target, TypeLattice, WithItem,
};
use ruff_python_ast as ast;
use ruff_text_size::Ranged;

use crate::ModuleAst;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// An internal invariant violation during lowering. Unreachable on programs
/// `parse` accepts; distinct from [`crate::ParseError`] so callers can tell a
/// source-text problem from a lowering bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweringBug {
    /// 1-based line of the violation.
    pub line: usize,
    /// 1-based column (Unicode scalar values) of the violation.
    pub col: usize,
    /// What invariant was violated.
    pub message: String,
}

impl fmt::Display for LoweringBug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "lowering bug at line {}, column {}: {}",
            self.line, self.col, self.message
        )
    }
}

impl std::error::Error for LoweringBug {}

/// Error type of [`lower`] / [`lower_source`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowerError {
    /// The source failed to parse (contract diagnostic, T-01 format).
    Parse(crate::ParseError),
    /// Internal invariant violation (should be unreachable; see [`LoweringBug`]).
    Bug(LoweringBug),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LowerError::Parse(e) => write!(f, "{e}"),
            LowerError::Bug(b) => write!(f, "{b}"),
        }
    }
}

impl std::error::Error for LowerError {}

impl From<crate::ParseError> for LowerError {
    fn from(e: crate::ParseError) -> Self {
        LowerError::Parse(e)
    }
}

// ---------------------------------------------------------------------------
// Scope classification (S-02-1..S-02-5)
// ---------------------------------------------------------------------------

/// Scope classification for one function-like scope (function, lambda, or
/// comprehension scope). Sets are sorted and deduplicated for byte-stable
/// output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScopeInfo {
    /// Dotted scope path from the module, e.g. `make_accumulator.<locals>.add`.
    /// Top-level functions are just `["f"]`; nested functions use the CPython
    /// qualname convention with `<locals>` segments.
    pub path: Vec<String>,
    /// Names declared `global` in this scope (S-02-3).
    pub globals: Vec<String>,
    /// Names bound to an enclosing function scope via `nonlocal` (S-02-4).
    pub nonlocals: Vec<String>,
    /// Names this scope reads from an enclosing function's locals — the free
    /// variables it captures (S-02-12).
    pub frees: Vec<String>,
    /// Local names of this scope that a nested scope captures — the closure
    /// cells this scope owns (S-02-13).
    pub cells: Vec<String>,
}

/// Module-level scope classification: one entry per function-like scope, in
/// deterministic (path-sorted) order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScopeInfoModule {
    /// Per-scope classification. Function-like scopes only: class bodies are
    /// their own namespace but do NOT enclose anything (S-02-5), so they never
    /// carry cell/free traffic and are not listed.
    pub scopes: Vec<ScopeInfo>,
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Lower a parsed module to IR plus its scope classification.
///
/// Fail-closed: parse errors propagate as [`LowerError::Parse`]; an internal
/// invariant violation (unreachable on accepted programs) is
/// [`LowerError::Bug`].
pub fn lower(ast: &ModuleAst) -> Result<(ap_ir::Module, ScopeInfoModule), LowerError> {
    let mut l = Lowerer::new(ast.source());
    let items = l.lower_stmt_list(&ast.module().body)?;
    let module = l.b.module(items);
    let scopes = l.finish_scopes();
    Ok((module, scopes))
}

/// Parse and lower in one step.
pub fn lower_source(src: &str) -> Result<(ap_ir::Module, ScopeInfoModule), LowerError> {
    let parsed = crate::parse(src)?;
    lower(&parsed)
}

// ---------------------------------------------------------------------------
// Scope tracking machinery
// ---------------------------------------------------------------------------

/// What kind of scope a body runs in. Drives name classification
/// (S-02-2, S-02-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    /// Module top level.
    Module,
    /// Function/lambda/comprehension body — locals apply (S-02-2).
    Function,
    /// Class body: its own namespace but NOT an enclosing scope for nested
    /// functions (S-02-5) — free lookups skip it.
    Class,
}

/// Per-scope name-binding data collected during lowering.
///
/// B7 (memperf wave 3): each insertion-ordered `Vec` has a `HashSet`
/// sidecar for the dedupe probe. The old form scanned the whole Vec per
/// name occurrence — O(names^2) per scope for hot local scopes (large
/// function bodies bind/read dozens of names, each re-scanned on every
/// occurrence). `seen_*` sets accept exactly what the Vec scan accepted;
/// insertion ORDER (the only externally-visible shape) still comes from
/// the Vecs, so all outputs are unchanged.
#[derive(Debug, Default)]
struct ScopeData {
    /// All names bound in this scope (assignment, def/class, import, for
    /// target, with-as, except-as, parameter, walrus, match capture).
    bound: Vec<String>,
    /// Names declared `global` (S-02-3).
    declared_global: Vec<String>,
    /// Names declared `nonlocal` (S-02-4).
    declared_nonlocal: Vec<String>,
    /// Names shared with a nested scope (closure cells; S-02-13).
    cells: Vec<String>,
    /// Names read but not bound in this scope (candidates for global/free).
    loads: Vec<String>,
    seen_bound: std::collections::HashSet<String>,
    seen_global: std::collections::HashSet<String>,
    seen_nonlocal: std::collections::HashSet<String>,
    seen_loads: std::collections::HashSet<String>,
}

impl ScopeData {
    fn bind(&mut self, name: &str) {
        // F5/B7 pattern: contains-first — repeated names (the hot case)
        // cost one hash probe and zero allocations.
        if !self.seen_bound.contains(name) {
            self.seen_bound.insert(name.to_string());
            self.bound.push(name.to_string());
        }
    }
    fn declare_global(&mut self, name: &str) {
        if !self.seen_global.contains(name) {
            self.seen_global.insert(name.to_string());
            self.declared_global.push(name.to_string());
        }
    }
    fn declare_nonlocal(&mut self, name: &str) {
        if !self.seen_nonlocal.contains(name) {
            self.seen_nonlocal.insert(name.to_string());
            self.declared_nonlocal.push(name.to_string());
        }
    }
    fn load(&mut self, name: &str) {
        if !self.seen_loads.contains(name) {
            self.seen_loads.insert(name.to_string());
            self.loads.push(name.to_string());
        }
    }
    /// Free-propagation push (close_scope): appends WITHOUT the dedupe
    /// `load()` applies — preserving the original unconditional push (dupes
    /// are collapsed by the sort+dedupe at the scope's close). Keeps the
    /// `seen_loads` sidecar in sync.
    fn load_propagate(&mut self, name: &str) {
        self.seen_loads.insert(name.to_string());
        self.loads.push(name.to_string());
    }
}

// ---------------------------------------------------------------------------
// The lowerer
// ---------------------------------------------------------------------------

struct Lowerer<'s> {
    src: &'s str,
    b: Builder,
    /// Scope stack; last entry is the innermost open scope.
    scopes: Vec<(ScopeKind, ScopeData, Vec<String>)>,
    /// Completed classifications keyed by dotted path (sorted on output).
    finished: BTreeMap<Vec<String>, ScopeInfo>,
    /// Wave-60: loads whose enclosing bind had not happened yet at scope
    /// close (self-recursive nested defs: the def's name binds after its
    /// body lowers). Retried once in finish_scopes.
    deferred_frees: Vec<(Vec<String>, String, Vec<String>)>,
    /// Wave-60: bound names per finished scope path, for the deferred-free
    /// retry to verify the load was truly bound (vs a builtin/global).
    finished_bounds: BTreeMap<Vec<String>, Vec<String>>,
    /// Byte offsets of every line start, built once in new(). line_of used
    /// to rescan src from byte 0 per node — O(file) per call, O(file^2)
    /// per module (profiled: dominated the unittest-import probe's samples
    /// at 3M+ newline-count iterations).
    line_starts: Vec<usize>,
}

impl<'s> Lowerer<'s> {
    fn new(src: &'s str) -> Self {
        let mut line_starts = Vec::with_capacity(64);
        line_starts.push(0usize);
        for (i, byte) in src.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Lowerer {
            src,
            b: Builder::new(),
            scopes: vec![(ScopeKind::Module, ScopeData::default(), Vec::new())],
            finished: BTreeMap::new(),
            deferred_frees: Vec::new(),
            finished_bounds: BTreeMap::new(),
            line_starts,
        }
    }

    fn bug(&self, offset: ruff_text_size::TextSize, message: impl Into<String>) -> LowerError {
        let off = usize::try_from(u32::from(offset)).unwrap_or(0);
        let (line, col) = crate::line_col_of(self.src, off);
        LowerError::Bug(LoweringBug {
            line,
            col,
            message: message.into(),
        })
    }

    fn cur(&mut self) -> &mut ScopeData {
        &mut self.scopes.last_mut().expect("scope stack never empty").1
    }

    fn finish_scopes(mut self) -> ScopeInfoModule {
        // Close any scopes still open (only the module itself should remain;
        // it is not classified).
        while self.scopes.len() > 1 {
            self.close_scope();
        }
        // Wave-60: resolve deferred frees now that every bind has run.
        // For each deferred (path, load): find the nearest ENCLOSING
        // function scope (path is a prefix-chain: enclosing scopes'
        // paths are strict prefixes with "<locals>." separators) that
        // bound the load; mark it as a cell there and add the load to
        // the deferred scope's frees.
        // W111 fix: a SELF-RECURSIVE def's binding lives in the def's
        // PARENT scope (def reg inside reg's own body reads its own
        // binding from make2). The recorded `enclosing` (nearest function
        // scope at def-time) is the def ITSELF — walk the enclosing chain
        // outward until a scope that truly binds the load is found.
        // B7 (memperf wave 3): the old code CLONED the whole finished_bounds
        // map (every finished scope's full bound Vec) even when there was
        // nothing deferred. `finish_scopes` consumes self and nothing reads
        // finished_bounds afterwards, so take ownership instead of cloning,
        // and skip entirely when the deferred list is empty.
        let deferred = std::mem::take(&mut self.deferred_frees);
        if deferred.is_empty() {
            return ScopeInfoModule {
                scopes: self.finished.into_values().collect(),
            };
        }
        let bounds = std::mem::take(&mut self.finished_bounds);
        for (path, load, enclosing) in deferred {
            let original_enclosing = enclosing.clone();
            // Walk the prefix chain from the recorded enclosing scope
            // outward: first scope that truly binds the load wins.
            // W110: enclosing is a SCOPE-PATH VECTOR; the prefix chain is
            // its own prefixes. Scope paths interleave "<locals>" marker
            // segments (["make2","<locals>","reg"]) — climb by dropping
            // trailing segments, skipping bare "<locals>" tails (a real
            // scope never ENDS on the marker).
            let mut chain: Vec<Vec<String>> = Vec::new();
            let mut cur: Vec<String> = enclosing.clone();
            chain.push(cur.clone());
            while cur.len() > 1 {
                cur.pop();
                if cur.last().map(|s| s.as_str()) == Some("<locals>") {
                    continue; // marker tail: not a real scope path
                }
                chain.push(cur.clone());
            }
            let mut resolved: Option<Vec<String>> = None;
            for scope_path in &chain {
                let truly_bound = bounds
                    .get(scope_path)
                    .map(|b| b.iter().any(|n| n == &load))
                    .unwrap_or(false);
                if truly_bound {
                    resolved = Some(scope_path.clone());
                    break;
                }
            }
            let Some(enclosing_hit) = resolved else {
                if std::env::var("APX_W111DBG").is_ok() {
                    eprintln!(
                        "[W111-DEFER] UNRESOLVED load={load} path={path:?} enclosing={enclosing:?}"
                    );
                }
                continue;
            };
            if std::env::var("APX_W111DBG").is_ok() {
                eprintln!("[W111-DEFER] load={load} path={path:?} hit={enclosing_hit:?}");
            }
            let enclosing = enclosing_hit.clone();
            // W111: propagate the free through INTERMEDIATE scopes
            // (CPython transitive co_freevars chain): every scope between
            // the def and the binding scope carries the name as a free so
            // the runtime cell chain (defining.my_cells -> free_cells)
            // passes through. The binding scope itself gets the cell.
            // Chain: original enclosing scope ... resolved binding scope.
            {
                // climb from the ORIGINAL enclosing to `resolved`, marking
                // intermediates
                let mut cur = original_enclosing.clone();
                let mut intermediate: Vec<Vec<String>> = Vec::new();
                while cur != enclosing {
                    if cur != path {
                        intermediate.push(cur.clone());
                    }
                    if cur.len() > 1 {
                        cur.pop();
                        if cur.last().map(|s| s.as_str()) == Some("<locals>") && cur.len() > 1 {
                            cur.pop();
                        }
                    } else {
                        break;
                    }
                }
                for scope_path in intermediate {
                    if let Some(si) = self.finished.get_mut(&scope_path) {
                        if !si.frees.iter().any(|f| f == &load) {
                            si.frees.push(load.clone());
                        }
                    }
                }
            }
            // The binding now exists in the enclosing scope's bound set —
            // but ScopeInfo doesn't carry `bound`; the runtime closure
            // wiring keys on cells, so mark the load as a cell in the
            // enclosing scope and a free in the deferred scope.
            if let Some(si) = self.finished.get_mut(&enclosing) {
                if !si.cells.iter().any(|c| c == &load) {
                    si.cells.push(load.clone());
                }
            }
            if let Some(si) = self.finished.get_mut(&path) {
                if !si.frees.iter().any(|f| f == &load) {
                    si.frees.push(load.clone());
                }
            }
        }
        ScopeInfoModule {
            scopes: self.finished.into_values().collect(),
        }
    }

    /// Resolve and record the classification for the scope being popped.
    fn close_scope(&mut self) {
        let (kind, data, path) = self.scopes.pop().expect("scope stack never empty");
        if kind == ScopeKind::Module {
            // The module scope is never classified.
            self.scopes.push((kind, data, path));
            return;
        }

        // `nonlocal x` binds to the nearest enclosing FUNCTION scope
        // (S-02-4); that shared binding is a cell in the enclosing scope
        // (S-02-13).
        for nl in &data.declared_nonlocal {
            for idx in (0..self.scopes.len()).rev() {
                if self.scopes[idx].0 == ScopeKind::Function {
                    if !self.scopes[idx].1.cells.iter().any(|c| c == nl) {
                        self.scopes[idx].1.cells.push(nl.clone());
                    }
                    break;
                }
            }
        }

        // Free variables: loads not bound locally, not global-declared, not
        // nonlocal-declared — resolved through enclosing FUNCTION scopes,
        // skipping class bodies (S-02-5). A hit marks the enclosing scope's
        // name as a cell (S-02-12/S-02-13); reaching the module scope means
        // the name is a plain global read, not free.
        let mut frees: Vec<String> = Vec::new();
        for load in &data.loads {
            if data.seen_bound.contains(load)
                || data.seen_global.contains(load)
                || data.seen_nonlocal.contains(load)
            {
                continue;
            }
            let mut found = false;
            for idx in (0..self.scopes.len()).rev() {
                let (ekind, edata, _) = &self.scopes[idx];
                match ekind {
                    ScopeKind::Module => break,   // module binding => global read
                    ScopeKind::Class => continue, // not enclosing (S-02-5)
                    ScopeKind::Function => {
                        if edata.seen_bound.contains(load) {
                            let cells = &mut self.scopes[idx].1.cells;
                            if !cells.iter().any(|c| c == load) {
                                cells.push(load.clone());
                            }
                            found = true;
                            break;
                        }
                    }
                }
            }
            if found {
                frees.push(load.clone());
            } else {
                // Wave-60: the enclosing def's bind() may not have run
                // yet (a def's name binds AFTER its body lowers, so a
                // self-recursive nested def reads its own future
                // binding). Defer with the IMMEDIATELY enclosing
                // function scope path — that is where the binding will
                // appear once the def statement lowers.
                let enclosing = self
                    .scopes
                    .iter()
                    .rev()
                    .find(|(k, _, _)| *k == ScopeKind::Function)
                    .map(|(_, _, p)| p.clone());
                if let Some(enc) = enclosing {
                    self.deferred_frees.push((path.clone(), load.clone(), enc));
                }
            }
        }

        // Propagate frees through intermediate scopes (CPython symtable
        // semantics): if this scope's free variable cannot be bound in an
        // IMMEDIATELY enclosing function scope, that enclosing scope itself
        // gains the name as a free so the cell chain passes through it.
        // (Matches CPython: co_cellvars/co_freevars form a transitive chain.)
        if !frees.is_empty() {
            for idx in (0..self.scopes.len()).rev() {
                match self.scopes[idx].0 {
                    ScopeKind::Module => break,
                    ScopeKind::Class => continue,
                    ScopeKind::Function => {
                        let edata = &mut self.scopes[idx].1;
                        // B7: sidecar-set probes; restructured to compute the
                        // pushes first (the old closure held an immutable
                        // borrow across the mutation).
                        let to_push: Vec<String> = frees
                            .iter()
                            .filter(|f| {
                                !(edata.seen_bound.contains(f.as_str())
                                    || edata.seen_global.contains(f.as_str())
                                    || edata.cells.iter().any(|c| c == *f)
                                    || edata.seen_nonlocal.contains(f.as_str()))
                            })
                            .cloned()
                            .collect();
                        for f in to_push {
                            edata.load_propagate(&f); // re-classified at this scope's close
                        }
                        break;
                    }
                }
            }
        }

        let sorted = |mut v: Vec<String>| {
            v.sort();
            v.dedup();
            v
        };
        let new_info = ScopeInfo {
            path: path.clone(),
            globals: sorted(data.declared_global),
            nonlocals: sorted(data.declared_nonlocal),
            frees: sorted(frees),
            cells: sorted(data.cells),
        };
        if std::env::var("APX_SCOPE_DEBUG").is_ok() {
            eprintln!(
                "[SCOPE] {:?} loads={:?} bound={:?} frees={:?} cells={:?}",
                new_info.path, data.loads, data.bound, new_info.frees, new_info.cells
            );
        }
        // T1-d fix: two scopes can finish with the SAME path when a nested
        // function's parent path was mis-prefixed (twisted deprecate.py
        // produced two "deprecated" entries — one real, one empty). The
        // BTreeMap insert CLOBBERED the real classification, breaking
        // closure wiring ('version' NameError). Merge on collision.
        match self.finished.get_mut(&path) {
            Some(existing) => {
                for c in &new_info.cells {
                    if !existing.cells.iter().any(|x| x == c) {
                        existing.cells.push(c.clone());
                    }
                }
                for f in &new_info.frees {
                    if !existing.frees.iter().any(|x| x == f) {
                        existing.frees.push(f.clone());
                    }
                }
                for g in &new_info.globals {
                    if !existing.globals.iter().any(|x| x == g) {
                        existing.globals.push(g.clone());
                    }
                }
                for nl in &new_info.nonlocals {
                    if !existing.nonlocals.iter().any(|x| x == nl) {
                        existing.nonlocals.push(nl.clone());
                    }
                }
            }
            None => {
                self.finished_bounds
                    .insert(path.clone(), data.bound.clone());
                self.finished.insert(path, new_info);
            }
        }
    }

    /// Open a new function-like scope named `name` inside the current path.
    fn open_scope(&mut self, kind: ScopeKind, name: &str) {
        let mut path = self.scopes.last().expect("scope").2.clone();
        if !path.is_empty() {
            // qualname convention: parent.<locals>.child for non-class parents
            path.push("<locals>".to_string());
        }
        path.push(name.to_string());
        self.scopes.push((kind, ScopeData::default(), path));
    }

    // -----------------------------------------------------------------------
    // Statements
    // -----------------------------------------------------------------------

    fn lower_stmt_list(&mut self, body: &[ast::Stmt]) -> Result<Vec<Item>, LowerError> {
        let mut items = Vec::with_capacity(body.len());
        for stmt in body {
            // T2: line marker before each top-level statement so tracebacks
            // carry true linenos.
            items.push(Item::Stmt(Stmt::Line(self.line_of(stmt))));
            items.extend(self.lower_stmt(stmt)?);
        }
        Ok(items)
    }

    /// Lower statements into a function `FnBody`. Nested defs/classes stay in
    /// source position via [`Stmt::LocalDef`]/[`Stmt::LocalClass`] — hoisting
    /// them to module items would reorder observable def-time effects
    /// (S-02-7: `def` executes and binds at its statement position).
    /// T2: source line of a statement (ruff span).
    fn line_of(&self, stmt: &ast::Stmt) -> usize {
        use ruff_text_size::Ranged;
        let off = stmt.start().to_u32() as usize;
        self.line_at(off)
    }

    /// W110: line of any AST node's start (used for the def keyword line;
    /// ruff's FunctionDef range begins at the first decorator, matching
    /// CPython's co_firstlineno semantics).
    fn line_of_stmt_start<T: ruff_text_size::Ranged>(&self, node: &T) -> usize {
        use ruff_text_size::Ranged;
        let off = node.start().to_u32() as usize;
        self.line_at(off)
    }

    /// Binary search over the one-shot line-start index: O(log lines)
    /// instead of the old O(offset) newline count per node.
    fn line_at(&self, off: usize) -> usize {
        let off = off.min(self.src.len());
        match self.line_starts.binary_search(&off) {
            Ok(i) => i + 1,
            Err(i) => i, // off is inside line i-1 (line_starts[i-1] < off)
        }
    }

    fn lower_fn_body(&mut self, body: &[ast::Stmt]) -> Result<FnBody, LowerError> {
        let mut blocks = Vec::with_capacity(body.len());
        for stmt in body {
            blocks.push(Stmt::Line(self.line_of(stmt)));
            for item in self.lower_stmt(stmt)? {
                match item {
                    Item::Stmt(s) => blocks.push(s),
                    Item::FnDef(f) => blocks.push(Stmt::LocalDef(Box::new(f))),
                    Item::ClassDef(c) => blocks.push(Stmt::LocalClass(Box::new(c))),
                }
            }
        }
        Ok(FnBody { blocks })
    }

    /// Lower statements into a class body (all results are statements; nested
    /// defs/classes inside a class body stay in position the same way).
    fn lower_class_body(&mut self, body: &[ast::Stmt]) -> Result<Vec<Stmt>, LowerError> {
        let mut blocks = Vec::with_capacity(body.len());
        for stmt in body {
            blocks.push(Stmt::Line(self.line_of(stmt)));
            for item in self.lower_stmt(stmt)? {
                match item {
                    Item::Stmt(s) => blocks.push(s),
                    Item::FnDef(f) => blocks.push(Stmt::LocalDef(Box::new(f))),
                    Item::ClassDef(c) => blocks.push(Stmt::LocalClass(Box::new(c))),
                }
            }
        }
        Ok(blocks)
    }

    fn lower_stmt(&mut self, s: &ast::Stmt) -> Result<Vec<Item>, LowerError> {
        match s {
            ast::Stmt::FunctionDef(f) => {
                // Def-time evaluation order: decorators (bottom-up at binding
                // time), defaults, then the function object binds its name
                // (S-02-7).
                let mut decorators = Vec::with_capacity(f.decorator_list.len());
                for d in &f.decorator_list {
                    decorators.push(self.lower_expr(&d.expression)?);
                }
                self.open_scope(ScopeKind::Function, f.name.id.as_str());
                let params = self.lower_params(&f.parameters)?;
                let return_ann = match &f.returns {
                    Some(r) => Some(self.lower_ann(r)?),
                    None => None,
                };
                let body = self.lower_fn_body(&f.body)?;
                self.close_scope();
                let mut fndef =
                    self.b
                        .fn_def(f.name.id.as_str(), params, return_ann, decorators, body);
                // W110: the def keyword's 1-based line (CPython
                // co_firstlineno). ruff's FunctionDef range starts at the
                // DECORATOR when present; CPython's co_firstlineno is the
                // same (first decorator line) — ruff start() matches.
                fndef.def_line = self.line_of_stmt_start(&f);
                if f.is_async {
                    fndef = Builder::fn_async(fndef);
                }
                self.cur().bind(f.name.id.as_str());
                Ok(vec![Item::FnDef(fndef)])
            }
            ast::Stmt::ClassDef(c) => {
                // Decorators, then bases/keywords in source order
                // (S-03-1/S-03-2).
                let mut decorators = Vec::with_capacity(c.decorator_list.len());
                for d in &c.decorator_list {
                    decorators.push(self.lower_expr(&d.expression)?);
                }
                let mut bases = Vec::new();
                let mut metaclass = None;
                if let Some(args) = &c.arguments {
                    for a in &args.args {
                        bases.push(self.lower_expr(a)?);
                    }
                    for kw in &args.keywords {
                        let v = self.lower_expr(&kw.value)?;
                        match &kw.arg {
                            Some(n) if n.id.as_str() == "metaclass" => {
                                // Explicit metaclass resolution (S-03-2).
                                metaclass = Some(v);
                            }
                            // Non-metaclass class keywords keep evaluation
                            // order (S-03-1); namespace delivery is Plan 03.
                            Some(n) => bases.push(
                                self.b.call(
                                    self.b
                                        .attribute(self.b.name("__ap_classkw__"), n.id.as_str()),
                                    vec![Arg::Pos(v)],
                                ),
                            ),
                            None => {
                                return Err(
                                    self.bug(kw.range().start(), "class keyword without a name")
                                )
                            }
                        }
                    }
                }
                self.open_scope(ScopeKind::Class, c.name.id.as_str());
                let body = self.lower_class_body(&c.body)?;
                self.close_scope();
                let class =
                    self.b
                        .class_def(c.name.id.as_str(), bases, metaclass, decorators, body);
                self.cur().bind(c.name.id.as_str());
                Ok(vec![Item::ClassDef(class)])
            }
            ast::Stmt::Return(r) => {
                // `return` with no expr returns None (S-02-11).
                let v = r.value.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(vec![Item::Stmt(self.b.return_(v))])
            }
            ast::Stmt::Delete(d) => {
                // del unbinds names / clears subscripts / attrs
                // (S-02-6, S-03-11).
                let mut targets = Vec::with_capacity(d.targets.len());
                for t in &d.targets {
                    targets.push(self.lower_del_target(t)?);
                }
                Ok(vec![Item::Stmt(self.b.del_(targets))])
            }
            ast::Stmt::TypeAlias(t) => {
                // PEP 695 `type X = ...` — matrix: recorded as module alias
                // metadata; no runtime effect in v1 (S-04 area). The name
                // binds like an assignment target; the statement itself
                // becomes a no-op Pass in the IR (annotation-only value).
                let name = alias_target_name(&t.name);
                if !name.is_empty() {
                    self.cur().bind(&name);
                }
                Ok(vec![Item::Stmt(self.b.pass_())])
            }
            ast::Stmt::Assign(a) => {
                // `x = y = v`: value evaluated once, then targets bound
                // left-to-right (S-02-19).
                let value = self.lower_expr(&a.value)?;
                let mut targets = Vec::with_capacity(a.targets.len());
                for t in &a.targets {
                    targets.push(self.lower_target(t, true)?);
                }
                Ok(vec![Item::Stmt(self.b.assign(targets, value))])
            }
            ast::Stmt::AugAssign(a) => {
                // In-place protocol first, binop fallback (S-03-26); name
                // targets rebind (S-02-19).
                let target = self.lower_target(&a.target, true)?;
                let value = self.lower_expr(&a.value)?;
                Ok(vec![Item::Stmt(self.b.aug_assign(
                    target,
                    lower_binop(a.op),
                    value,
                ))])
            }
            ast::Stmt::AnnAssign(a) => {
                let target = self.lower_target(&a.target, true)?;
                let ann = self.lower_ann(&a.annotation)?;
                let value = a.value.as_ref().map(|v| self.lower_expr(v)).transpose()?;
                Ok(vec![Item::Stmt(self.b.ann_assign(target, ann, value))])
            }
            ast::Stmt::For(f) => {
                // iter() obtained once, next() until StopIteration, else on
                // normal exhaustion (S-02-17). Loop target binds in the
                // enclosing function scope and LEAKS after the loop
                // (S-02-17/S-02-2). `async for` differs only in the iterator
                // protocol (S-05-13 area) — structural IR shape identical.
                let iter = self.lower_expr(&f.iter)?;
                let target = self.lower_target(&f.target, true)?;
                let body = self.lower_block(&f.body)?;
                let orelse = self.lower_block(&f.orelse)?;
                Ok(vec![Item::Stmt(self.b.for_(target, iter, body, orelse))])
            }
            ast::Stmt::While(w) => {
                // Pre-test loop; else on normal exit (S-02-16).
                let cond = self.lower_expr(&w.test)?;
                let body = self.lower_block(&w.body)?;
                let orelse = self.lower_block(&w.orelse)?;
                Ok(vec![Item::Stmt(self.b.while_(cond, body, orelse))])
            }
            ast::Stmt::If(i) => {
                // First true branch runs (S-02-15); elif chains nest as Ifs
                // in the orelse position.
                let cond = self.lower_expr(&i.test)?;
                let then = self.lower_block(&i.body)?;
                let els = self.lower_elif_chain(&i.elif_else_clauses)?;
                Ok(vec![Item::Stmt(self.b.if_(cond, then, els))])
            }
            ast::Stmt::With(w) => {
                // Items evaluated in order, entered in order, exits reverse
                // (S-02-40..42). `async with` differs only in protocol
                // (S-05-13 area) — same structural shape.
                let mut items = Vec::with_capacity(w.items.len());
                for item in &w.items {
                    let expr = self.lower_expr(&item.context_expr)?;
                    let target = match &item.optional_vars {
                        Some(t) => Some(self.lower_target(t, true)?),
                        None => None,
                    };
                    items.push(WithItem { expr, target });
                }
                let body = self.lower_block(&w.body)?;
                Ok(vec![Item::Stmt(self.b.with_(items, body))])
            }
            ast::Stmt::Match(m) => {
                // Structural pattern matching per PEP 634; captures bind in
                // the enclosing scope (S-02-2 store rules).
                let subject = self.lower_expr(&m.subject)?;
                let mut cases = Vec::with_capacity(m.cases.len());
                for case in &m.cases {
                    let pattern = self.lower_pattern(&case.pattern)?;
                    let guard = case
                        .guard
                        .as_ref()
                        .map(|g| self.lower_expr(g))
                        .transpose()?;
                    let body = self.lower_block(&case.body)?;
                    bind_pattern_names(self, &case.pattern);
                    cases.push(MatchCase {
                        pattern,
                        guard,
                        body,
                    });
                }
                Ok(vec![Item::Stmt(self.b.match_(subject, cases))])
            }
            ast::Stmt::Raise(r) => {
                // raise EXPR [from CAUSE]; bare raise re-raises in flight
                // (S-02-27/S-02-28); `from` sets __cause__ (S-02-34).
                let exc = r.exc.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let cause = r.cause.as_ref().map(|c| self.lower_expr(c)).transpose()?;
                Ok(vec![Item::Stmt(self.b.raise_(exc, cause))])
            }
            ast::Stmt::Try(t) => {
                // Source-order handler dispatch (S-02-30), bare except
                // (S-02-31), else (S-02-32), finally on every path (S-02-33).
                // `except*` (is_star) has the same IR shape; ExceptionGroup
                // split semantics live in the runtime (spec 02/05).
                let body = self.lower_block(&t.body)?;
                let mut handlers = Vec::with_capacity(t.handlers.len());
                for h in &t.handlers {
                    let h = match h {
                        ast::ExceptHandler::ExceptHandler(inner) => inner,
                    };
                    let ty = h.type_.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                    let name = h.name.as_ref().map(|n| n.id.as_str().to_string());
                    if let Some(n) = &h.name {
                        // except-as binds, then unbinds after the handler
                        // (S-02-30); it is still a static local binding.
                        self.cur().bind(n.id.as_str());
                    }
                    let hbody = self.lower_block(&h.body)?;
                    handlers.push(ap_ir::ExceptHandler {
                        ty,
                        name,
                        body: hbody,
                    });
                }
                let orelse = self.lower_block(&t.orelse)?;
                let finalbody = self.lower_block(&t.finalbody)?;
                Ok(vec![Item::Stmt(
                    self.b.try_(body, handlers, orelse, finalbody),
                )])
            }
            ast::Stmt::Assert(a) => {
                // AssertionError on falsy test (S-02-39).
                let test = self.lower_expr(&a.test)?;
                let msg = a.msg.as_ref().map(|m| self.lower_expr(m)).transpose()?;
                Ok(vec![Item::Stmt(self.b.assert_(test, msg))])
            }
            ast::Stmt::Import(imp) => {
                // `import a.b.c` binds top name `a`; `as d` binds `d`
                // (S-04-6/S-04-8; local binding inside functions S-04-9).
                let mut aliases = Vec::with_capacity(imp.names.len());
                for a in &imp.names {
                    aliases.push(lower_alias(a));
                    let bind = match &a.asname {
                        Some(n) => n.id.as_str(),
                        None => a.name.id.as_str().split('.').next().unwrap_or_default(),
                    };
                    self.cur().bind(bind);
                }
                Ok(vec![Item::Stmt(self.b.import_(aliases))])
            }
            ast::Stmt::ImportFrom(imp) => {
                // `from m import n` binds n (S-04-7); `*` is module-level
                // only (S-04-10); missing attr error at import (S-04-15).
                // Relative imports keep their dots in the module path.
                let module = match &imp.module {
                    Some(m) => m.id.as_str().to_string(),
                    None => String::new(),
                };
                let level_dots = ".".repeat(imp.level as usize);
                let full_module = format!("{level_dots}{module}");
                let star = imp.names.iter().any(|a| a.name.id.as_str() == "*");
                let mut aliases = Vec::new();
                for a in &imp.names {
                    if a.name.id.as_str() == "*" {
                        continue;
                    }
                    aliases.push(lower_alias(a));
                    self.cur().bind(
                        a.asname
                            .as_ref()
                            .map(|n| n.id.as_str())
                            .unwrap_or_else(|| a.name.id.as_str()),
                    );
                }
                Ok(vec![Item::Stmt(self.b.import_from(
                    full_module,
                    aliases,
                    star,
                ))])
            }
            ast::Stmt::Global(g) => {
                // global names (S-02-3).
                let names: Vec<String> =
                    g.names.iter().map(|n| n.id.as_str().to_string()).collect();
                for n in &names {
                    self.cur().declare_global(n);
                }
                Ok(vec![Item::Stmt(self.b.global_(names))])
            }
            ast::Stmt::Nonlocal(nl) => {
                // nonlocal names (S-02-4).
                let names: Vec<String> =
                    nl.names.iter().map(|n| n.id.as_str().to_string()).collect();
                for n in &names {
                    self.cur().declare_nonlocal(n);
                }
                Ok(vec![Item::Stmt(self.b.nonlocal(names))])
            }
            ast::Stmt::Expr(e) => {
                let v = self.lower_expr(&e.value)?;
                Ok(vec![Item::Stmt(self.b.expr_stmt(v))])
            }
            ast::Stmt::Pass(_) => Ok(vec![Item::Stmt(self.b.pass_())]),
            ast::Stmt::Break(_) => Ok(vec![Item::Stmt(self.b.break_())]),
            ast::Stmt::Continue(_) => Ok(vec![Item::Stmt(self.b.continue_())]),
            ast::Stmt::IpyEscapeCommand(_) => Err(self.bug(
                s.range().start(),
                "IPython escape commands never parse in module mode",
            )),
        }
    }

    /// Lower a statement block into IR statements.
    fn lower_block(&mut self, body: &[ast::Stmt]) -> Result<Vec<Stmt>, LowerError> {
        let mut blocks = Vec::with_capacity(body.len());
        for stmt in body {
            // W112: line marker before each nested-block statement too.
            // lower_fn_body/lower_stmt_list emit these, but try/if/while
            // bodies (lower_block) did not — frame.last_line then lagged
            // one or more statements, and tb entries for exceptions
            // raised inside nested blocks showed stale lines (probe27/28:
            // raise at line 5 reported as 3; pluggy entries 1-low across
            // the board, crashing the pytest repr formatter's source
            // lookup).
            blocks.push(Stmt::Line(self.line_of(stmt)));
            for item in self.lower_stmt(stmt)? {
                match item {
                    Item::Stmt(s) => blocks.push(s),
                    Item::FnDef(f) => blocks.push(Stmt::LocalDef(Box::new(f))),
                    Item::ClassDef(c) => blocks.push(Stmt::LocalClass(Box::new(c))),
                }
            }
        }
        Ok(blocks)
    }

    /// Lower elif/else clauses (S-02-15) into the else position of an If:
    /// nested Ifs, first-true-branch semantics preserved.
    fn lower_elif_chain(
        &mut self,
        clauses: &[ast::ElifElseClause],
    ) -> Result<Vec<Stmt>, LowerError> {
        let mut it = clauses.iter();
        let Some(first) = it.next() else {
            return Ok(Vec::new());
        };
        match &first.test {
            Some(t) => {
                let c = self.lower_expr(t)?;
                let then = self.lower_block(&first.body)?;
                let els = self.lower_elif_chain(&clauses[1..])?;
                Ok(vec![self.b.if_(c, then, els)])
            }
            None => self.lower_block(&first.body),
        }
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    fn lower_expr(&mut self, e: &ast::Expr) -> Result<Expr, LowerError> {
        match e {
            ast::Expr::NumberLiteral(n) => Ok(self.lower_number(&n.value)),
            ast::Expr::StringLiteral(s) => {
                // Implicit concatenation is resolved by ruff into one value;
                // lowering joins the parts. Unobservable: concatenated
                // literals are a single object at runtime (C-18).
                Ok(self.b.str_lit(s.value.to_str().to_string()))
            }
            ast::Expr::BytesLiteral(bs) => {
                let bytes: Vec<u8> = bs.value.bytes().collect();
                Ok(self.b.bytes_lit(bytes))
            }
            ast::Expr::BooleanLiteral(b) => Ok(self.b.bool_lit(b.value)),
            ast::Expr::NoneLiteral(_) => Ok(self.b.none_lit()),
            ast::Expr::EllipsisLiteral(_) => Ok(self.b.ellipsis_lit()),
            ast::Expr::Name(n) => {
                self.cur().load(n.id.as_str());
                Ok(self.b.name(n.id.as_str()))
            }
            ast::Expr::BinOp(b_) => {
                let lhs = self.lower_expr(&b_.left)?;
                let rhs = self.lower_expr(&b_.right)?;
                Ok(self.b.bin_op(lower_binop(b_.op), lhs, rhs))
            }
            ast::Expr::UnaryOp(u) => {
                let x = self.lower_expr(&u.operand)?;
                Ok(self.b.unary_op(lower_unaryop(u.op), x))
            }
            ast::Expr::BoolOp(b_) => {
                let mut vals = Vec::with_capacity(b_.values.len());
                for v in &b_.values {
                    vals.push(self.lower_expr(v)?);
                }
                let op = match b_.op {
                    ast::BoolOp::And => BoolOp::And,
                    ast::BoolOp::Or => BoolOp::Or,
                };
                Ok(self.b.bool_op(op, vals))
            }
            ast::Expr::Compare(c) => {
                // Chained comparison kept as a chain: each middle term
                // evaluated once, short-circuits on first false (S-01-31).
                let first = self.lower_expr(&c.left)?;
                let mut operands = vec![first];
                for cmp in &c.comparators {
                    operands.push(self.lower_expr(cmp)?);
                }
                let ops: Vec<CmpOp> = c.ops.iter().map(|op| lower_cmpop(*op)).collect();
                Ok(self.b.compare(ops, operands))
            }
            ast::Expr::Call(c) => {
                let f = self.lower_expr(&c.func)?;
                // Call-argument evaluation order: left-to-right in source
                // (S-02-9). ruff splits positionals (with inline stars) from
                // keywords; in valid Python a bare positional cannot follow a
                // keyword (parse error), so positionals-then-keywords IS
                // source order. Stars stay marked (S-02-9 unpacking).
                let mut args =
                    Vec::with_capacity(c.arguments.args.len() + c.arguments.keywords.len());
                for a in &c.arguments.args {
                    args.push(match a {
                        ast::Expr::Starred(st) => Arg::Star(self.lower_expr(&st.value)?),
                        other => Arg::Pos(self.lower_expr(other)?),
                    });
                }
                for kw in &c.arguments.keywords {
                    let v = self.lower_expr(&kw.value)?;
                    args.push(match &kw.arg {
                        Some(name) => Arg::Kw(name.id.as_str().to_string(), v),
                        None => Arg::KwStar(v),
                    });
                }
                Ok(self.b.call(f, args))
            }
            ast::Expr::Attribute(a) => {
                let obj = self.lower_expr(&a.value)?;
                Ok(self.b.attribute(obj, a.attr.id.as_str()))
            }
            ast::Expr::Subscript(s) => {
                let obj = self.lower_expr(&s.value)?;
                let key = self.lower_expr(&s.slice)?;
                Ok(self.b.subscript(obj, key))
            }
            ast::Expr::Slice(sl) => {
                let lo = sl.lower.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let hi = sl.upper.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                let st = sl.step.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(self.b.slice(lo, hi, st))
            }
            ast::Expr::List(l) => {
                let mut items = Vec::with_capacity(l.elts.len());
                for it in &l.elts {
                    items.push(self.lower_expr(it)?);
                }
                Ok(self.b.list(items))
            }
            ast::Expr::Tuple(t) => {
                let mut items = Vec::with_capacity(t.elts.len());
                for it in &t.elts {
                    items.push(self.lower_expr(it)?);
                }
                Ok(self.b.tuple(items))
            }
            ast::Expr::Set(s) => {
                let mut items = Vec::with_capacity(s.elts.len());
                for it in &s.elts {
                    items.push(self.lower_expr(it)?);
                }
                Ok(self.b.set(items))
            }
            ast::Expr::Dict(d) => {
                // `**spam` entries (key None) become DictExpand items,
                // preserving left-to-right evaluation order (S-01-21).
                let mut items = Vec::with_capacity(d.items.len());
                for item in &d.items {
                    match &item.key {
                        Some(k) => {
                            let key = self.lower_expr(k)?;
                            let value = self.lower_expr(&item.value)?;
                            items.push(DictItem { key, value });
                        }
                        None => {
                            let expanded = self.lower_expr(&item.value)?;
                            items.push(DictItem {
                                key: self.b.dict_expand(expanded),
                                value: self.b.none_lit(),
                            });
                        }
                    }
                }
                Ok(self.b.dict(items))
            }
            ast::Expr::Lambda(l) => {
                // Lambda: own scope, frame name `<lambda>` (S-02-38);
                // params/body lower like a function's (S-02-8/S-02-12).
                self.open_scope(ScopeKind::Function, "<lambda>");
                let params = match &l.parameters {
                    Some(p) => self.lower_params(p)?,
                    None => Vec::new(),
                };
                let body = self.lower_expr(&l.body)?;
                self.close_scope();
                Ok(self.b.lambda(params, body))
            }
            ast::Expr::If(i) => {
                // Conditional expression: condition by truthiness, only the
                // taken branch evaluates (S-01-27).
                let cond = self.lower_expr(&i.test)?;
                let then = self.lower_expr(&i.body)?;
                let els = self.lower_expr(&i.orelse)?;
                Ok(self.b.if_exp(cond, then, els))
            }
            ast::Expr::Named(n) => {
                // Walrus binds in the enclosing scope, never a comprehension's
                // nested clause scope (S-02-2). ruff already resolved the
                // binding scope at parse time; the Name target + value keep
                // evaluation order (target := value).
                let tname = match &*n.target {
                    ast::Expr::Name(nm) => nm.id.as_str().to_string(),
                    _ => {
                        return Err(
                            self.bug(n.range().start(), "walrus target is not a simple name")
                        )
                    }
                };
                self.cur().bind(&tname);
                let value = self.lower_expr(&n.value)?;
                Ok(self.b.named(self.b.name(tname), value))
            }
            ast::Expr::Starred(st) => {
                // Star unpacking in displays/args (S-02-9).
                let x = self.lower_expr(&st.value)?;
                Ok(self.b.starred(x))
            }
            ast::Expr::Yield(y) => {
                let v = y.value.as_ref().map(|e| self.lower_expr(e)).transpose()?;
                Ok(self.b.yield_(v))
            }
            ast::Expr::YieldFrom(y) => {
                let v = self.lower_expr(&y.value)?;
                Ok(self.b.yield_from(v))
            }
            ast::Expr::Await(a) => {
                let v = self.lower_expr(&a.value)?;
                Ok(self.b.await_(v))
            }
            ast::Expr::FString(f) => self.lower_fstring_value(&f.value),
            ast::Expr::TString(_) => {
                // Matrix: t-strings rejected at the 3.14 v1 subset pin.
                Err(self.bug(e.range().start(), "t-strings are not in the v1 subset"))
            }
            ast::Expr::ListComp(c) => {
                // Comprehensions own an implicit scope (S-02-2); the first
                // iterable evaluates in the ENCLOSING scope (S-02-5) —
                // captured because lowering of `iter` happens before the
                // scope opens below.
                let head_iter = self.lower_comp_clauses_head(&c.generators)?;
                self.open_scope(ScopeKind::Function, "<listcomp>");
                let clauses = self.lower_comp_clauses_tail(head_iter, &c.generators)?;
                let elt = self.lower_expr(&c.elt)?;
                self.close_scope();
                Ok(self.b.list_comp(elt, clauses))
            }
            ast::Expr::SetComp(c) => {
                let head_iter = self.lower_comp_clauses_head(&c.generators)?;
                self.open_scope(ScopeKind::Function, "<setcomp>");
                let clauses = self.lower_comp_clauses_tail(head_iter, &c.generators)?;
                let elt = self.lower_expr(&c.elt)?;
                self.close_scope();
                Ok(self.b.set_comp(elt, clauses))
            }
            ast::Expr::DictComp(c) => {
                let head_iter = self.lower_comp_clauses_head(&c.generators)?;
                self.open_scope(ScopeKind::Function, "<dictcomp>");
                let clauses = self.lower_comp_clauses_tail(head_iter, &c.generators)?;
                let key = c
                    .key
                    .as_ref()
                    .map(|k| self.lower_expr(k))
                    .transpose()?
                    .ok_or_else(|| self.bug(c.range().start(), "dict comprehension without key"))?;
                let val = self.lower_expr(&c.value)?;
                self.close_scope();
                Ok(self.b.dict_comp(key, val, clauses))
            }
            ast::Expr::Generator(g) => {
                let head_iter = self.lower_comp_clauses_head(&g.generators)?;
                self.open_scope(ScopeKind::Function, "<genexpr>");
                let clauses = self.lower_comp_clauses_tail(head_iter, &g.generators)?;
                let elt = self.lower_expr(&g.elt)?;
                self.close_scope();
                Ok(self.b.generator(elt, clauses))
            }
            ast::Expr::IpyEscapeCommand(_) => Err(self.bug(
                e.range().start(),
                "IPython escape commands never parse in module mode",
            )),
        }
    }

    /// Lower the FIRST comprehension clause's iterable in the *enclosing*
    /// scope (S-02-5: the first iterable is evaluated outside the implicit
    /// scope). Returns the lowered clause-0 iterable, or None when there are
    /// no clauses (later clauses are lowered by `lower_comp_clauses_tail`).
    /// B7 (memperf wave 3): the old shape returned a Vec wrapping the whole
    /// `gens.to_vec()` deep clone of every comprehension clause; the tail
    /// consumed only the lowered iterable and dropped the clone unconditionally.
    fn lower_comp_clauses_head(
        &mut self,
        gens: &[ast::Comprehension],
    ) -> Result<Option<Expr>, LowerError> {
        if let Some(first) = gens.first() {
            let iter = self.lower_expr(&first.iter)?;
            return Ok(Some(iter));
        }
        Ok(None)
    }

    /// Finish comprehension lowering inside the fresh scope: rebuild all
    /// clauses, binding targets in the comprehension scope; clause 0's
    /// iterable reuses the already-lowered enclosing-scope expression.
    fn lower_comp_clauses_tail(
        &mut self,
        head_iter: Option<Expr>,
        gens: &[ast::Comprehension],
    ) -> Result<Vec<CompClause>, LowerError> {
        let mut clauses = Vec::with_capacity(gens.len());
        for (i, g) in gens.iter().enumerate() {
            let target = self.lower_comp_target(&g.target)?;
            let iter = if i == 0 {
                head_iter
                    .clone()
                    .expect("head lowered for the first clause")
            } else {
                self.lower_expr(&g.iter)?
            };
            let mut ifs = Vec::with_capacity(g.ifs.len());
            for f in &g.ifs {
                ifs.push(self.lower_expr(f)?);
            }
            clauses.push(self.b.comp_clause(target, iter, ifs));
        }
        Ok(clauses)
    }

    fn lower_number(&mut self, n: &ast::Number) -> Expr {
        match n {
            ast::Number::Int(i) => {
                // Literal ints are non-negative in source (unary minus is a
                // separate expr). Small path: u64 -> i128. Big path: digits
                // verbatim (C-12/C-13 arbitrary precision).
                match i.as_u64() {
                    Some(v) => self.b.int_lit(v as i128),
                    None => self.b.int_big_lit(i.to_string()),
                }
            }
            ast::Number::Float(f) => self.b.float_lit(*f),
            ast::Number::Complex { real, imag } => self.b.complex_lit(*real, *imag),
        }
    }

    fn lower_fstring_value(&mut self, v: &ast::FStringValue) -> Result<Expr, LowerError> {
        // S-01-16/C-18: literal chunks (post-escape) + interpolations with
        // optional conversion and (possibly nested) format spec. Implicitly
        // concatenated f-string parts flatten in order. Debug forms (`{x=}`)
        // keep their literal source text on the Interp part (PEP 701: the
        // debug text is real emitted output); the implied-`!r` conversion is
        // applied in `lower_interp_element` per CPython rules.
        let mut parts = Vec::new();
        for part in v.iter() {
            match part {
                ast::FStringPart::Literal(s) => {
                    let text = s.as_str();
                    if !text.is_empty() {
                        parts.push(FStrPart::Lit(text.to_string()));
                    }
                }
                ast::FStringPart::FString(f) => {
                    for el in f.elements.iter() {
                        self.lower_interp_element(el, &mut parts)?;
                    }
                }
            }
        }
        Ok(self.b.fstring(parts))
    }

    fn lower_interp_element(
        &mut self,
        el: &ast::InterpolatedStringElement,
        parts: &mut Vec<FStrPart>,
    ) -> Result<(), LowerError> {
        match el {
            ast::InterpolatedStringElement::Literal(lit) => {
                let text = lit.value.as_ref();
                if !text.is_empty() {
                    parts.push(FStrPart::Lit(text.to_string()));
                }
            }
            ast::InterpolatedStringElement::Interpolation(interp) => {
                let expr = self.lower_expr(&interp.expression)?;
                let debug_text = interp.debug_text.as_ref().map(|d| d.as_str().to_string());
                let mut conv = match interp.conversion.to_char() {
                    Some('s') => Some(Conversion::S),
                    Some('r') => Some(Conversion::R),
                    Some('a') => Some(Conversion::A),
                    Some(other) => {
                        return Err(self.bug(
                            interp.range().start(),
                            format!("unknown f-string conversion flag {other:?}"),
                        ))
                    }
                    None => None,
                };
                let spec = match &interp.format_spec {
                    Some(spec) => {
                        // Format specs are themselves f-string parts
                        // (S-01-16 nesting).
                        let mut spec_parts = Vec::new();
                        for el in spec.elements.iter() {
                            self.lower_interp_element(el, &mut spec_parts)?;
                        }
                        Some(spec_parts)
                    }
                    None => None,
                };
                // Debug-form implied conversion (PEP 701 / CPython): a bare
                // debug form renders with repr(); ruff leaves the conversion
                // flag at None, so derive it here. An explicit conversion
                // (f20 `{x=!r}`) or a format spec (f19 `{x=:>10}`) suppresses
                // the implication — CPython emits neither an implicit `!r`
                // when a spec is present.
                if debug_text.is_some() && conv.is_none() && spec.is_none() {
                    conv = Some(Conversion::R);
                }
                parts.push(FStrPart::Interp {
                    expr,
                    conv,
                    spec,
                    debug_text,
                });
            }
        }
        Ok(())
    }

    /// Comprehension clause targets bind in the comprehension scope
    /// (S-02-2): register bindings and build the target expression.
    fn lower_comp_target(&mut self, t: &ast::Expr) -> Result<Expr, LowerError> {
        let expr = self.lower_expr(t)?;
        self.bind_target_names(t);
        Ok(expr)
    }

    /// Register every name a target pattern binds (S-02-2).
    fn bind_target_names(&mut self, t: &ast::Expr) {
        match t {
            ast::Expr::Name(n) => self.cur().bind(n.id.as_str()),
            ast::Expr::Starred(st) => self.bind_target_names(&st.value),
            ast::Expr::Tuple(tup) => {
                for el in &tup.elts {
                    self.bind_target_names(el);
                }
            }
            ast::Expr::List(list) => {
                for el in &list.elts {
                    self.bind_target_names(el);
                }
            }
            _ => {} // attribute/subscript targets bind nothing
        }
    }

    /// Lower an assignment/for/with/del target into the IR `Target` shape.
    fn lower_target(&mut self, t: &ast::Expr, bind: bool) -> Result<Target, LowerError> {
        let target = match t {
            ast::Expr::Name(n) => {
                if bind {
                    self.cur().bind(n.id.as_str());
                }
                Target::Name(n.id.as_str().to_string())
            }
            ast::Expr::Subscript(sub) => {
                let obj = self.lower_expr(&sub.value)?;
                let key = self.lower_expr(&sub.slice)?;
                Target::Subscript(Box::new(obj), Box::new(key))
            }
            ast::Expr::Attribute(a) => {
                let obj = self.lower_expr(&a.value)?;
                Target::Attribute(Box::new(obj), a.attr.id.as_str().to_string())
            }
            ast::Expr::Tuple(tup) => {
                let mut items = Vec::with_capacity(tup.elts.len());
                for el in &tup.elts {
                    items.push(self.lower_target(el, bind)?);
                }
                Target::Tuple(items)
            }
            ast::Expr::List(list) => {
                let mut items = Vec::with_capacity(list.elts.len());
                for el in &list.elts {
                    items.push(self.lower_target(el, bind)?);
                }
                Target::Tuple(items)
            }
            ast::Expr::Starred(st) => {
                let inner = self.lower_target(&st.value, bind)?;
                Target::Starred(Box::new(target_to_expr(inner)))
            }
            other => {
                return Err(self.bug(
                    other.range().start(),
                    "unsupported assignment target expression",
                ))
            }
        };
        Ok(target)
    }

    /// Lower a `del` target (S-02-6): name targets are statically-bound
    /// locals for classification even though del unbinds them.
    fn lower_del_target(&mut self, t: &ast::Expr) -> Result<Target, LowerError> {
        match t {
            ast::Expr::Name(n) => {
                self.cur().bind(n.id.as_str());
                Ok(Target::Name(n.id.as_str().to_string()))
            }
            other => self.lower_target(other, false),
        }
    }

    // -----------------------------------------------------------------------
    // Parameters and annotations
    // -----------------------------------------------------------------------

    fn lower_params(&mut self, p: &ast::Parameters) -> Result<Vec<Param>, LowerError> {
        let mut out = Vec::new();
        // Kinds and order per S-02-8: posonly `/`, pos-or-kw, *vararg,
        // kwonly, **kwarg — source order preserved.
        for pwd in &p.posonlyargs {
            out.push(self.lower_param_with_default(pwd, ParamKind::PosOnly)?);
        }
        for pwd in &p.args {
            out.push(self.lower_param_with_default(pwd, ParamKind::PosOrKw)?);
        }
        if let Some(v) = &p.vararg {
            out.push(self.lower_variadic_param(v, ParamKind::VarArg)?);
        }
        for pwd in &p.kwonlyargs {
            out.push(self.lower_param_with_default(pwd, ParamKind::KwOnly)?);
        }
        if let Some(k) = &p.kwarg {
            out.push(self.lower_variadic_param(k, ParamKind::KwArg)?);
        }
        Ok(out)
    }

    fn lower_param_with_default(
        &mut self,
        pwd: &ast::ParameterWithDefault,
        kind: ParamKind,
    ) -> Result<Param, LowerError> {
        let name = pwd.parameter.name.id.as_str().to_string();
        self.cur().bind(&name);
        let mut param = self.b.param(name, kind);
        // Defaults evaluate at def time, left-to-right (S-02-7).
        if let Some(d) = &pwd.default {
            param = Builder::with_default(param, self.lower_expr(d)?);
        }
        if let Some(ann) = &pwd.parameter.annotation {
            param = Builder::with_ann(param, self.lower_ann(ann)?);
        }
        Ok(param)
    }

    /// `*args` / `**kwargs` (never a default; S-02-8).
    fn lower_variadic_param(
        &mut self,
        p: &ast::Parameter,
        kind: ParamKind,
    ) -> Result<Param, LowerError> {
        let name = p.name.id.as_str().to_string();
        self.cur().bind(&name);
        let mut param = self.b.param(name, kind);
        if let Some(ann) = &p.annotation {
            param = Builder::with_ann(param, self.lower_ann(ann)?);
        }
        Ok(param)
    }

    /// Lower an annotation expression and seed the lattice when recognizable
    /// (S-04 area: annotations never evaluate for runtime effect in v1).
    fn lower_ann(&mut self, e: &ast::Expr) -> Result<ap_ir::Ann, LowerError> {
        let expr = self.lower_expr(e)?;
        let ty = lattice_of_ann(&expr);
        Ok(self.b.ann(expr, ty))
    }

    // -----------------------------------------------------------------------
    // Patterns
    // -----------------------------------------------------------------------

    fn lower_pattern(&mut self, p: &ast::Pattern) -> Result<Pattern, LowerError> {
        match p {
            ast::Pattern::MatchValue(v) => Ok(Pattern::Literal(self.lower_expr(&v.value)?)),
            ast::Pattern::MatchSingleton(sg) => Ok(Pattern::Literal(match sg.value {
                ast::Singleton::None => self.b.none_lit(),
                ast::Singleton::True => self.b.bool_lit(true),
                ast::Singleton::False => self.b.bool_lit(false),
            })),
            ast::Pattern::MatchSequence(seq) => {
                // [p, *rest, q] / (p, q): the starred element (at most one,
                // PEP 634) becomes the sequence's `rest`, position preserved
                // as rest-follows-items.
                let mut items = Vec::with_capacity(seq.patterns.len());
                let mut rest = None;
                for sub in &seq.patterns {
                    if let ast::Pattern::MatchStar(star) = sub {
                        rest = Some(Box::new(match &star.name {
                            Some(n) => Pattern::Capture(n.id.as_str().to_string()),
                            None => Pattern::Wildcard,
                        }));
                    } else {
                        items.push(self.lower_pattern(sub)?);
                    }
                }
                Ok(Pattern::Sequence(items, rest))
            }
            ast::Pattern::MatchMapping(mp) => {
                // {key: p, **rest}: keys are literals/values (PEP 634);
                // lowered as literal-key patterns in source order.
                let mut items = Vec::with_capacity(mp.keys.len());
                for (k, v) in mp.keys.iter().zip(mp.patterns.iter()) {
                    let key = self.lower_expr(k)?;
                    let pat = self.lower_pattern(v)?;
                    items.push((Pattern::Literal(key), pat));
                }
                let rest = mp.rest.as_ref().map(|r| r.id.as_str().to_string());
                Ok(Pattern::Mapping(items, rest))
            }
            ast::Pattern::MatchClass(cls) => {
                // C(pos..., kw=p...): keywords must follow positionals
                // (PEP 634); shape mirrors that order.
                let c = self.lower_expr(&cls.cls)?;
                let mut pos = Vec::with_capacity(cls.arguments.patterns.len());
                for sub in &cls.arguments.patterns {
                    pos.push(self.lower_pattern(sub)?);
                }
                let mut kws = Vec::with_capacity(cls.arguments.keywords.len());
                for kw in &cls.arguments.keywords {
                    kws.push((
                        kw.attr.id.as_str().to_string(),
                        self.lower_pattern(&kw.pattern)?,
                    ));
                }
                Ok(Pattern::Class(Box::new(c), pos, kws))
            }
            ast::Pattern::MatchStar(star) => Err(self.bug(
                star.range().start(),
                "bare MatchStar outside a sequence pattern",
            )),
            ast::Pattern::MatchAs(ma) => match (&ma.pattern, &ma.name) {
                // `p as x`: inner shape kept under a Group wrapper; the
                // capture itself is registered by bind_pattern_names (the IR
                // Pattern set has no AS node; S-02-2 binding still recorded).
                (Some(inner), Some(_)) => Ok(Pattern::Group(Box::new(self.lower_pattern(inner)?))),
                (Some(inner), None) => Ok(Pattern::Group(Box::new(self.lower_pattern(inner)?))),
                (None, Some(n)) => Ok(Pattern::Capture(n.id.as_str().to_string())),
                (None, None) => Ok(Pattern::Wildcard), // `case _`
            },
            ast::Pattern::MatchOr(or) => {
                // p | q (PEP 634: alternatives bind the same names).
                let mut alts = Vec::with_capacity(or.patterns.len());
                for sub in &or.patterns {
                    alts.push(self.lower_pattern(sub)?);
                }
                Ok(Pattern::Or(alts))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// PEP 634 pattern capture names bind in the enclosing scope (S-02-2).
fn bind_pattern_names(l: &mut Lowerer<'_>, p: &ast::Pattern) {
    match p {
        ast::Pattern::MatchAs(ma) => {
            if let Some(inner) = &ma.pattern {
                bind_pattern_names(l, inner);
            }
            if let Some(n) = &ma.name {
                l.cur().bind(n.id.as_str());
            }
        }
        ast::Pattern::MatchStar(star) => {
            if let Some(n) = &star.name {
                l.cur().bind(n.id.as_str());
            }
        }
        ast::Pattern::MatchSequence(seq) => {
            for sub in &seq.patterns {
                bind_pattern_names(l, sub);
            }
        }
        ast::Pattern::MatchMapping(mp) => {
            for v in &mp.patterns {
                bind_pattern_names(l, v);
            }
            if let Some(r) = &mp.rest {
                l.cur().bind(r.id.as_str());
            }
        }
        ast::Pattern::MatchClass(cls) => {
            for sub in &cls.arguments.patterns {
                bind_pattern_names(l, sub);
            }
            for kw in &cls.arguments.keywords {
                bind_pattern_names(l, &kw.pattern);
            }
        }
        ast::Pattern::MatchOr(or) => {
            for sub in &or.patterns {
                bind_pattern_names(l, sub);
            }
        }
        ast::Pattern::MatchValue(_) | ast::Pattern::MatchSingleton(_) => {}
    }
}

fn lower_alias(a: &ast::Alias) -> ImportAlias {
    // B7 (memperf wave 3): split the borrowed dotted name directly — the
    // old form allocated the whole dotted String only to re-segment it.
    let path: Vec<String> = a.name.id.as_str().split('.').map(str::to_string).collect();
    let alias = a.asname.as_ref().map(|n| n.id.as_str().to_string());
    ImportAlias { path, alias }
}

/// `type X = ...` target is a name (possibly subscripted generic alias);
/// v1 uses the name form.
fn alias_target_name(t: &ast::Expr) -> String {
    match t {
        ast::Expr::Name(n) => n.id.as_str().to_string(),
        ast::Expr::Subscript(sub) => match &*sub.value {
            ast::Expr::Name(n) => n.id.as_str().to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn lower_binop(op: ast::Operator) -> BinOp {
    match op {
        ast::Operator::Add => BinOp::Add,
        ast::Operator::Sub => BinOp::Sub,
        ast::Operator::Mult => BinOp::Mul,
        ast::Operator::MatMult => BinOp::MatMul,
        ast::Operator::Div => BinOp::Div,
        ast::Operator::Mod => BinOp::Mod,
        ast::Operator::Pow => BinOp::Pow,
        ast::Operator::LShift => BinOp::LShift,
        ast::Operator::RShift => BinOp::RShift,
        ast::Operator::BitOr => BinOp::BitOr,
        ast::Operator::BitXor => BinOp::BitXor,
        ast::Operator::BitAnd => BinOp::BitAnd,
        ast::Operator::FloorDiv => BinOp::FloorDiv,
    }
}

fn lower_unaryop(op: ast::UnaryOp) -> ap_ir::UnaryOp {
    match op {
        ast::UnaryOp::Invert => ap_ir::UnaryOp::Invert,
        ast::UnaryOp::Not => ap_ir::UnaryOp::Not,
        ast::UnaryOp::UAdd => ap_ir::UnaryOp::Plus,
        ast::UnaryOp::USub => ap_ir::UnaryOp::Minus,
    }
}

fn lower_cmpop(op: ast::CmpOp) -> CmpOp {
    match op {
        ast::CmpOp::Eq => CmpOp::Eq,
        ast::CmpOp::NotEq => CmpOp::NotEq,
        ast::CmpOp::Lt => CmpOp::Lt,
        ast::CmpOp::LtE => CmpOp::LtE,
        ast::CmpOp::Gt => CmpOp::Gt,
        ast::CmpOp::GtE => CmpOp::GtE,
        ast::CmpOp::Is => CmpOp::Is,
        ast::CmpOp::IsNot => CmpOp::IsNot,
        ast::CmpOp::In => CmpOp::In,
        ast::CmpOp::NotIn => CmpOp::NotIn,
    }
}

/// Seed the type lattice from a lowered annotation naming a recognizable
/// built-in (orchestrator lattice note; S-04 area).
fn lattice_of_ann(e: &Expr) -> TypeLattice {
    match e {
        Expr::Name(n) => match n.as_str() {
            "int" => TypeLattice::Int,
            "float" => TypeLattice::Float,
            "str" => TypeLattice::Str,
            "bool" => TypeLattice::Bool,
            "None" => TypeLattice::None,
            _ => TypeLattice::Unknown,
        },
        _ => TypeLattice::Unknown,
    }
}

/// Extract a Target-shaped expression back out of an inner target (for
/// `Target::Starred`, which wraps an Expr).
fn target_to_expr(t: Target) -> Expr {
    match t {
        Target::Name(n) => Expr::Name(n),
        Target::Subscript(o, k) => Expr::Subscript(o, k),
        Target::Attribute(o, a) => Expr::Attribute(o, a),
        Target::Tuple(items) => Expr::Tuple(items.into_iter().map(target_to_expr).collect()),
        Target::Starred(x) => Expr::Starred(x),
    }
}

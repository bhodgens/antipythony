//! The lowerer: ap-ir Expr/Stmt -> flat `Vec<Instr>` + tables.
//!
//! CONTRACT (master.md + leaf spec 01):
//! - Unsupported shapes raise [`Unsupported`] out of the recursive
//!   walkers; `lower_fn` CATCHES it and collapses the WHOLE function to
//!   one `Instr::FallbackToTreeWalk` — function granularity, never a
//!   partially-lowered body.
//! - Slot assignment: locals = every name STORED anywhere in the body
//!   (StoreFast names scan), plus parameters. Everything else is a
//!   global read (`LoadGlobal` — the name table is shared with
//!   StoreGlobal; LEGB-minus-closure resolution is the interpreter's job,
//!   mirroring the tree-walker's global-or-builtin fallback for
//!   module-level execution).
//! - Const pool dedupes; line table records only on line change; site
//!   ids are allocated for LoadAttr/Call/LoadGlobal in emission order.
//! - Control flow (leaf 01-core/01): If/While/For lower to Jump/GetIter/
//!   ForNext; BoolOp to short-circuit Jump chains (value rule preserved);
//!   Break/Continue/loop-else via block-target tracking (loops stack).
//!   Try/With/Match/Raise/assert/import/del/nested-def still raise
//!   Unsupported — a Try ANYWHERE in a loop body rejects the whole
//!   function (function granularity, 01-core/03 owns exceptions).

use std::collections::HashMap;

use ap_ir::{Arg, Expr, FnDef, Item, Stmt, Target};

use crate::code::CodeObject;
use crate::instr::{BinKind, CompareOp, Instr, UnaryKind};
use crate::linetable::LineTable;
use crate::pool::{Const, ConstPool};
use crate::sites::{SiteKind, SiteTable};

/// A shape the start-set lowerer does not support. Carries what was hit,
/// for the fallback WARN (`APX_BP_DEBUG=1` per master.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

/// Result of lowering one function.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // CodeObject moves out once at entry-build time
pub enum LowerOutcome {
    /// Fully lowered to bytecode.
    Code(CodeObject),
    /// Function-granularity fallback: run the tree-walker on this def.
    Fallback(Unsupported),
}

/// Lower one `FnDef` to bytecode or a function-level fallback.
pub fn lower_fn(fn_def: &FnDef) -> LowerOutcome {
    match Lowerer::new().lower_fn_body(fn_def) {
        Ok(code) => LowerOutcome::Code(code),
        Err(u) => LowerOutcome::Fallback(u),
    }
}

/// Convenience: true iff `f` fully lowered.
pub fn lowers_cleanly(f: &FnDef) -> bool {
    matches!(lower_fn(f), LowerOutcome::Code(_))
}

/// Lower a module-level function item (same thing; kept as a named entry
/// point for the module walker).
pub fn lower_module_fn(item: &Item) -> Option<LowerOutcome> {
    match item {
        Item::FnDef(f) => Some(lower_fn(f)),
        _ => None,
    }
}

struct Lowerer {
    instrs: Vec<Instr>,
    consts: ConstPool,
    names: Vec<String>,
    name_idx: HashMap<String, u32>,
    lines: LineTable,
    sites: SiteTable,
    slots: HashMap<String, u16>,
    n_locals: usize,
    cur_line: usize,
    /// Break/continue routing (block-target tracking). One frame per
    /// enclosing loop: patches applied when the loop's exit is fixed up.
    loop_stack: Vec<LoopFrame>,
    /// Comprehension-scope emulation (leaf 02-calls): per-clause target
    /// bindings live OUTSIDE the function's compile-time slot table —
    /// CPython scopes them to the comprehension's implicit function. Each
    /// entry maps name -> hidden slot; `slot_of` consults the stack TOP
    /// DOWN before the plain table, so a comprehension's `for x in ...`
    /// never leaks `x` into the enclosing frame (S-02-2). Hidden slots
    /// allocate past `n_locals` visible slots... no: they ARE locals vec
    /// entries (the dispatch loop's locals vec is slot-indexed), but they
    /// are EXCLUDED from local_names/frame sync (see hidden_slots below).
    comp_scopes: Vec<HashMap<String, u16>>,
    /// Hidden slot count (comprehension targets live at
    /// n_locals..n_locals+hidden). sync_slots_into_frame iterates
    /// local_names (visible only), so hidden slots never sync back to the
    /// frame — the comprehension's scope stays private.
    hidden_slots: u16,
    /// Exception table under construction (leaf fp-exceptions). Range
    /// brackets append; sorted at finalize.
    excs: crate::exctable::ExcTable,
    /// Current try-nesting depth (the ExcEntry tiebreak).
    try_depth: u16,
    /// FINALLY-PENDING (leaf fp-exceptions): the open try-finally bodies
    /// (innermost last). A Return/Break/Continue fired while this stack
    /// is non-empty INLINES every open finalbody at the exit site
    /// (CPython 3.10's duplicated-block discipline — no runtime pending
    /// slot needed; return-in-finally / break-in-finally override falls
    /// out of the copy's own direct emission).
    fin_bodies: Vec<Vec<Stmt>>,
    /// True while lowering a finally body (inline copy OR the exception-
    /// unwind epilogue copy): inner control flow emits DIRECTLY (its
    /// return/break wins; no re-inlining — also bounds code growth).
    inside_finally: bool,
    /// Compile-time operand-stack depth at statement boundaries (the
    /// residue loop iterators / comprehension handles leave across
    /// statements). begin_range records it as the ExcEntry stack_depth —
    /// the dispatch loop's unwind truncates the operand stack back to
    /// this depth, preserving enclosing for-loop iterator handles.
    operand_depth: u32,
}

/// One enclosing loop's pending break/continue jump sites (instruction
/// indices whose u32 target gets backpatched).
struct LoopFrame {
    /// `break` jumps -> the loop's AFTER-ORELSE exit point.
    breaks: Vec<u32>,
    /// `continue` jumps -> the loop's CONTINUE point (For: the ForNext;
    /// While: the condition re-eval).
    continues: Vec<u32>,
    /// True for For loops: the iterator handle sits on the operand stack
    /// for the whole body, so `break` must PopTop it before jumping to the
    /// exit arc (exhaustion pops it via ForNext; break must match, or the
    /// enclosing loop peeks a stale handle). CPython's compiler synthesizes
    /// the same stack unwind on its break arcs.
    is_for: bool,
    /// The operand-stack depth WITH this loop's iterator handle on it
    /// (statement boundaries inside the loop sit one deeper than outside
    /// for For loops). Restored on loop exit paths.
    opdepth: u32,
}

impl Lowerer {
    fn new() -> Self {
        Self {
            instrs: Vec::new(),
            consts: ConstPool::new(),
            names: Vec::new(),
            name_idx: HashMap::new(),
            lines: LineTable::new(),
            sites: SiteTable::new(),
            slots: HashMap::new(),
            n_locals: 0,
            cur_line: 0,
            loop_stack: Vec::new(),
            comp_scopes: Vec::new(),
            hidden_slots: 0,
            excs: crate::exctable::ExcTable::new(),
            try_depth: 0,
            fin_bodies: Vec::new(),
            inside_finally: false,
            operand_depth: 0,
        }
    }

    // --- emission helpers -------------------------------------------------

    fn emit(&mut self, i: Instr) -> u32 {
        let idx = self.instrs.len() as u32;
        self.instrs.push(i);
        idx
    }

    /// Index the NEXT emit would use (the instruction just emitted + 1...
    /// i.e. after emit_at_line, `instrs.len()-1` is the emitted one).
    fn cur_instr_idx(&self) -> u32 {
        (self.instrs.len() - 1) as u32
    }

    fn emit_at_line(&mut self, i: Instr, line: usize) {
        let idx = self.emit(i);
        self.lines.record(idx, line);
        self.cur_line = line;
    }

    fn intern_name(&mut self, name: &str) -> u32 {
        if let Some(&idx) = self.name_idx.get(name) {
            return idx;
        }
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.name_idx.insert(name.to_string(), idx);
        idx
    }

    fn slot_of(&mut self, name: &str) -> Option<u16> {
        // Comprehension scopes shadow: innermost first (the implicit
        // function scope sits INSIDE the enclosing function's scope for
        // reads — a comprehension body referencing an enclosing local
        // falls through to the visible slot).
        for scope in self.comp_scopes.iter().rev() {
            if let Some(&s) = scope.get(name) {
                return Some(s);
            }
        }
        self.slots.get(name).copied()
    }

    fn alloc_slot(&mut self, name: &str) -> u16 {
        if let Some(&s) = self.slots.get(name) {
            return s;
        }
        let s = self.n_locals as u16;
        self.n_locals += 1;
        self.slots.insert(name.to_string(), s);
        s
    }

    /// Allocate a HIDDEN slot (comprehension clause target / accumulator).
    /// Sits past every visible slot; excluded from local_names (frame sync
    /// iterates local_names, so hidden slots never leak into the enclosing
    /// frame's namespace — the comprehension's implicit function scope).
    fn alloc_hidden_slot(&mut self, name: &str) -> u16 {
        let s = self.n_locals as u16 + self.hidden_slots;
        self.hidden_slots += 1;
        s
    }

    /// Resolve (allocating on first store) the hidden slot of a
    /// comp-scope NAME target in the innermost scope. Non-name targets are
    /// a loud fallback this leaf.
    fn comp_target_slot(&mut self, target: &Expr) -> Result<u16, Unsupported> {
        let Expr::Name(name) = target else {
            return Err(Unsupported("comprehension target (non-name)"));
        };
        if let Some(scope) = self.comp_scopes.last() {
            if let Some(&s) = scope.get(name) {
                return Ok(s);
            }
        }
        let s = self.alloc_hidden_slot(name);
        self.comp_scopes
            .last_mut()
            .expect("comp scope pushed")
            .insert(name.clone(), s);
        Ok(s)
    }

    /// Emit a jump with a placeholder target; returns the instruction idx
    /// for backpatching.
    fn emit_jump(&mut self, i: Instr) -> u32 {
        debug_assert!(i.is_jump());
        self.emit(i)
    }

    fn patch_jump(&mut self, at: u32, target: u32) {
        let ins = &mut self.instrs[at as usize];
        match ins {
            Instr::Jump(t)
            | Instr::JumpIfFalse(t)
            | Instr::JumpIfTrue(t)
            | Instr::JumpIfFalseOrPop(t)
            | Instr::JumpIfTrueOrPop(t)
            | Instr::ForNext(t)
            | Instr::CheckExcept(t) => *t = target,
            _ => unreachable!("patch_jump on non-jump"),
        }
    }

    fn patch_all(&mut self, sites: &[u32], target: u32) {
        for &at in sites {
            self.patch_jump(at, target);
        }
    }

    // --- entry ------------------------------------------------------------

    fn lower_fn_body(mut self, fn_def: &FnDef) -> Result<CodeObject, Unsupported> {
        // Generators/async stay on the tree-walker (master.md out-of-scope
        // list: fiber machinery).
        if fn_def.is_generator {
            return Err(Unsupported("generator"));
        }
        if fn_def.is_async {
            return Err(Unsupported("async def"));
        }
        // VarArg/KwArg/KwOnly params bind inside the call boundary (the SAME
        // bind_args machinery the tree-walker uses fills the frame's locals
        // map by NAME — defaults via eval_default, vararg as a tuple, kwarg
        // as a dict, kw-only from kwargs — and run_frame's prefill reads
        // every param slot by name afterwards). Leaf 02-calls lifts the
        // old refusals: params of ALL kinds + defaults lower fine. *args/
        // **kwargs CALL-SITE shapes (runtime unpack) stay lowerer-rejected
        // at the call arm — exotic, loud, per-function.
        // (No per-param checks remain: prefill is name-keyed and total.)

        // Pre-assign parameter slots in source order.
        for p in &fn_def.params {
            self.alloc_slot(&p.name);
        }

        // Locals scan: every name ASSIGNED anywhere in the body gets a
        // slot up-front (Python locals are compile-time known). Run BEFORE
        // emission so reads resolve to LoadFast even before the store.
        let mut assigned = Vec::new();
        scan_assigned_names(&fn_def.body.blocks, &mut assigned);
        for name in assigned {
            self.alloc_slot(&name);
        }

        // Emission. First entry gets the def line (traceback prologue).
        let start_line = fn_def.def_line;
        self.cur_line = start_line;
        self.lines.record(0, start_line);
        self.lower_stmts(&fn_def.body.blocks)?;
        // Implicit `return None` on fall-off-the-end (S-02-11).
        self.emit(Instr::ReturnNone);
        // fp-superinstructions (02-perf/02): data-driven peephole. Rewrites
        // the histogram's top (load,load,BinOp) triples into ONE dispatch
        // boundary. Must run AFTER all jumps are backpatched (it rewrites
        // instruction indices in place, fixing up every jump target).
        self.peephole_fuse_binop_triples();

        Ok(CodeObject {
            name: fn_def.name.clone(),
            def_line: fn_def.def_line,
            // Visible + hidden (comprehension-scope) slots; local_names
            // carries VISIBLE names only, so frame sync never sees the
            // comprehension's private scope.
            n_locals: self.n_locals + self.hidden_slots as usize,
            local_names: self.local_names_sorted(),
            instrs: self.instrs,
            consts: self.consts,
            names: self.names,
            lines: self.lines,
            sites: self.sites,
            excs: {
                self.excs.sort();
                self.excs
            },
        })
    }

    fn local_names_sorted(&self) -> Vec<String> {
        let mut v: Vec<(u16, String)> = self.slots.iter().map(|(n, s)| (*s, n.clone())).collect();
        v.sort_by_key(|(s, _)| *s);
        v.into_iter().map(|(_, n)| n).collect()
    }

    /// fp-superinstructions (02-perf/02): fuse the HISTOGRAM-SUPPORTED
    /// `(load a, load b, BinOp)` triples into one instruction. The three
    /// shapes come straight from `APX_BP_STATS`:
    ///   LoadConst,LoadConst,BinOp (9.24M) -> LoadConst2BinOp
    ///   LoadFast,LoadConst,BinOp  (2.35M) -> LoadFastConstBinOp
    ///   LoadFast,LoadFast,BinOp   (0.42M) -> LoadFast2BinOp
    ///
    /// Correctness invariants:
    /// - Each fused instruction is semantically the unfused triple (the
    ///   dispatcher uses the SAME fast_smi_binop + eval_binop fallback).
    /// - Rewriting shrinks the stream; jump targets, line-table indices,
    ///   site instr_idx and exception ranges are ALL remapped through a
    ///   single `old -> new` table so control flow / tracebacks stay exact.
    /// - Only ADJACENT instructions with no intervening jump target are
    ///   fused. A block boundary (any jump targeting instr i) pins i, so
    ///   a fused op never spans a control-flow join.
    /// - No site/carry on LoadConst/LoadFast/BinOp — the site table is
    ///   untouched by the rewrite (sites carry their own instr_idx, which
    ///   the remap fixes up).
    #[allow(clippy::too_many_lines)]
    fn peephole_fuse_binop_triples(&mut self) {
        let n = self.instrs.len();
        if n < 3 {
            return;
        }
        // Pinned = any jump target (a control-flow entry point). A fused
        // op must not swallow a pinned instruction into its middle.
        let mut pinned = vec![false; n];
        for ins in &self.instrs {
            if let Some(t) = ins.jump_target() {
                if (t as usize) < n {
                    pinned[t as usize] = true;
                }
            }
        }
        // old index -> new index (identity until a fusion removes slots).
        let mut remap: Vec<u32> = (0..n as u32).collect();
        let mut out: Vec<Instr> = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            let fused = if i + 2 < n && !pinned[i + 1] && !pinned[i + 2] {
                match (self.instrs[i], self.instrs[i + 1], self.instrs[i + 2]) {
                    (Instr::LoadConst(a), Instr::LoadConst(b), Instr::BinOp(op)) => {
                        Some(Instr::LoadConst2BinOp { a, b, op })
                    }
                    (Instr::LoadFast(slot), Instr::LoadConst(b), Instr::BinOp(op)) => {
                        Some(Instr::LoadFastConstBinOp { slot, b, op })
                    }
                    (Instr::LoadFast(a), Instr::LoadFast(b), Instr::BinOp(op)) => {
                        Some(Instr::LoadFast2BinOp { a, b, op })
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(f) = fused {
                // The fused instruction lands at `out.len()`; the three
                // old instructions map there (targets INTO the middle of
                // the triple would have been pinned, so none land at i+1
                // or i+2).
                let dst = out.len() as u32;
                remap[i] = dst;
                remap[i + 1] = dst;
                remap[i + 2] = dst;
                out.push(f);
                i += 3;
            } else {
                remap[i] = out.len() as u32;
                out.push(self.instrs[i]);
                i += 1;
            }
        }
        let new_n = out.len();
        if new_n == n {
            return; // no fusion matched — leave every table untouched
        }
        // Remap jump targets.
        for ins in &mut out {
            if let Some(t) = ins.jump_target() {
                let nt = remap.get(t as usize).copied().unwrap_or(t);
                let replaced = match *ins {
                    Instr::Jump(_) => Some(Instr::Jump(nt)),
                    Instr::JumpIfFalse(_) => Some(Instr::JumpIfFalse(nt)),
                    Instr::JumpIfTrue(_) => Some(Instr::JumpIfTrue(nt)),
                    Instr::JumpIfFalseOrPop(_) => Some(Instr::JumpIfFalseOrPop(nt)),
                    Instr::JumpIfTrueOrPop(_) => Some(Instr::JumpIfTrueOrPop(nt)),
                    Instr::ForNext(_) => Some(Instr::ForNext(nt)),
                    Instr::CheckExcept(_) => Some(Instr::CheckExcept(nt)),
                    _ => None,
                };
                if let Some(x) = replaced {
                    *ins = x;
                }
            }
        }
        // Remap the sparse line table (rebuild through the public API to
        // keep its monotonicity invariant).
        let old_lines = self.lines.clone();
        self.lines = crate::linetable::LineTable::from_remapped(old_lines.iter(), |i| {
            remap.get(i as usize).copied().unwrap_or(i)
        });
        // Remap site instr_idx (fused ops carry no sites, so no site is
        // dropped; sites on OTHER instructions move with their op).
        self.sites.remap_instr(&remap);
        // Remap exception ranges (start/ inclusive, end exclusive).
        for e in self.excs.entries_mut() {
            e.start = remap.get(e.start as usize).copied().unwrap_or(e.start);
            e.end = remap.get(e.end as usize).copied().unwrap_or(e.end);
            e.handler = remap.get(e.handler as usize).copied().unwrap_or(e.handler);
        }
        self.instrs = out;
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<(), Unsupported> {
        for s in stmts {
            self.lower_stmt(s)?;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, s: &Stmt) -> Result<(), Unsupported> {
        match s {
            Stmt::Assign(targets, value) => {
                self.lower_expr(value)?;
                // Multiple targets: `a = b = v` — value duplicated by
                // the interpreter is NOT in the start set (no DupTop);
                // single-target only.
                if targets.len() != 1 {
                    return Err(Unsupported("multi-target assign"));
                }
                self.lower_store_target(&targets[0])
            }
            Stmt::AugAssign(target, op, value) => {
                // `x op= v`: read-target, AUG (in-place protocol), store-
                // target. The AugOp opcode (leaf 01-core/01) executes the
                // __iX__-aware aug — list `+=` aliasing mutation preserved.
                self.lower_load_target(target)?;
                self.lower_expr(value)?;
                self.emit_at_line(
                    Instr::AugOp {
                        kind: BinKind::from_ir(*op),
                        sym: op_symbol(*op),
                    },
                    self.cur_line,
                );
                self.lower_store_target(target)
            }
            Stmt::AnnAssign(target, _ann, value) => {
                // `x: T = v` — value assigns normally; a bare annotation
                // (`x: T`) has no runtime effect in function scope (v1:
                // `__annotations__` binding is a module/class-scope
                // effect, matching the tree-walker's function-scope arm).
                if let Some(v) = value {
                    self.lower_expr(v)?;
                    self.lower_store_target(target)?;
                }
                Ok(())
            }
            Stmt::Return(Some(e)) => {
                self.lower_expr(e)?;
                // FINALLY-PENDING (inline-copy): a return through open
                // try-finallys inlines their bodies at the exit site
                // (CPython's return-through-finally). A return INSIDE a
                // finally copy emits directly (return-in-finally
                // overrides — CPython 3.14 emits a SyntaxWarning but the
                // return wins).
                let fin_bodies = if self.inside_finally {
                    Vec::new()
                } else {
                    self.fin_bodies.clone()
                };
                // Inline copies are finally bodies: their inner control
                // flow emits DIRECTLY (no re-inline — bounds recursion).
                let saved = self.inside_finally;
                self.inside_finally = true;
                for fb in &fin_bodies {
                    self.lower_stmts(fb)?;
                }
                self.inside_finally = saved;
                self.emit_at_line(Instr::ReturnValue, self.cur_line);
                Ok(())
            }
            Stmt::Return(None) => {
                let fin_bodies = if self.inside_finally {
                    Vec::new()
                } else {
                    self.fin_bodies.clone()
                };
                let saved = self.inside_finally;
                self.inside_finally = true;
                for fb in &fin_bodies {
                    self.lower_stmts(fb)?;
                }
                self.inside_finally = saved;
                self.emit_at_line(Instr::ReturnNone, self.cur_line);
                Ok(())
            }
            Stmt::If(cond, then, orelse) => self.lower_if(cond, then, orelse),
            Stmt::While(cond, body, orelse) => self.lower_while(cond, body, orelse),
            Stmt::For(target, iter, body, orelse) => self.lower_for(target, iter, body, orelse),
            Stmt::With(..) => Err(Unsupported("with statement")),
            Stmt::Try(t) => self.lower_try(t),
            Stmt::Raise(exc_expr, cause_expr) => {
                // raise [e [from cause]] (leaf fp-exceptions). The
                // DISPATCH loop's Raise arm carries the full Stmt::Raise
                // semantics via interp's shared helpers (raise_from_value
                // / inflight re-raise / __cause__ chaining); the opcode
                // carries only the operand shape.
                let has_expr = exc_expr.is_some();
                if !has_expr {
                    // Bare raise: re-raise the in-flight exception.
                    self.emit_at_line(Instr::Raise { mode: 2 }, self.cur_line);
                    return Ok(());
                }
                let e = exc_expr.as_ref().expect("checked above");
                self.lower_expr(e)?;
                if let Some(c) = cause_expr {
                    self.lower_expr(c)?;
                    self.emit_at_line(Instr::Raise { mode: 1 }, self.cur_line);
                } else {
                    self.emit_at_line(Instr::Raise { mode: 0 }, self.cur_line);
                }
                Ok(())
            }
            Stmt::Match(..) => Err(Unsupported("match statement")),
            Stmt::Assert(..) => Err(Unsupported("assert statement")),
            Stmt::Global(..) => Err(Unsupported("global declaration")),
            Stmt::Nonlocal(..) => Err(Unsupported("nonlocal declaration")),
            Stmt::Import(..) => Err(Unsupported("import statement")),
            Stmt::ImportFrom(..) => Err(Unsupported("import-from statement")),
            Stmt::Del(..) => Err(Unsupported("del statement")),
            Stmt::Break => {
                let Some(_) = self.loop_stack.last() else {
                    return Err(Unsupported("break outside loop"));
                };
                // FINALLY-PENDING (inline-copy): every open try-finally's
                // body is INLINED at this exit site (innermost LAST — the
                // unwind order is inner-to-outer, and a break inside an
                // inlined copy emits DIRECTLY, overriding any outer
                // pending unwind — CPython break-in-finally semantics).
                // After the copies, the For-iterator handle pops and the
                // loop-exit Jump fires.
                let fin_bodies = if self.inside_finally {
                    Vec::new() // inside a finally copy: break wins directly
                } else {
                    self.fin_bodies.clone()
                };
                // Inline copies are finally bodies: their inner control
                // flow emits DIRECTLY (no re-inline — bounds recursion).
                let saved = self.inside_finally;
                self.inside_finally = true;
                for fb in &fin_bodies {
                    self.lower_stmts(fb)?;
                }
                self.inside_finally = saved;
                if self.loop_stack.last().is_some_and(|f| f.is_for) {
                    self.emit_at_line(Instr::PopTop, self.cur_line);
                }
                let j = self.emit_jump(Instr::Jump(u32::MAX));
                if let Some(frame) = self.loop_stack.last_mut() {
                    frame.breaks.push(j);
                }
                Ok(())
            }
            Stmt::Continue => {
                let Some(_) = self.loop_stack.last() else {
                    return Err(Unsupported("continue outside loop"));
                };
                let fin_bodies = if self.inside_finally {
                    Vec::new()
                } else {
                    self.fin_bodies.clone()
                };
                let saved = self.inside_finally;
                self.inside_finally = true;
                for fb in &fin_bodies {
                    self.lower_stmts(fb)?;
                }
                self.inside_finally = saved;
                let j = self.emit_jump(Instr::Jump(u32::MAX));
                if let Some(frame) = self.loop_stack.last_mut() {
                    frame.continues.push(j);
                }
                Ok(())
            }
            Stmt::LocalDef(_) => Err(Unsupported("nested def")),
            Stmt::LocalClass(_) => Err(Unsupported("nested class")),
            Stmt::Pass => Ok(()),
            Stmt::ExprStmt(e) => {
                self.lower_expr(e)?;
                self.emit_at_line(Instr::PopTop, self.cur_line);
                Ok(())
            }
            Stmt::Line(line) => {
                // Line markers from the builder update the current line
                // for subsequent emission without emitting an instr.
                if *line >= 1 {
                    self.cur_line = *line;
                }
                Ok(())
            }
        }
    }

    // --- control flow (leaf 01-core/01) ------------------------------------

    /// `if c: T else: E` — also covers elif chains (orelse holds the next
    /// If). CPython 3.12+ shape: evaluate cond, JumpIfFalse over the then.
    fn lower_if(&mut self, cond: &Expr, then: &[Stmt], orelse: &[Stmt]) -> Result<(), Unsupported> {
        self.lower_expr(cond)?;
        let j_end = self.emit_jump(Instr::JumpIfFalse(u32::MAX));
        self.lower_stmts(then)?;
        if orelse.is_empty() {
            self.patch_jump(j_end, self.instrs.len() as u32);
        } else {
            // Jump over the else after the then-block.
            let j_over = self.emit_jump(Instr::Jump(u32::MAX));
            self.patch_jump(j_end, self.instrs.len() as u32);
            self.lower_stmts(orelse)?;
            self.patch_jump(j_over, self.instrs.len() as u32);
        }
        Ok(())
    }

    /// `while c: BODY else: ELSE` —
    /// ```text
    /// LOOP:  eval c; JumpIfFalse -> EXIT
    ///        BODY (breaks/continues collect into the frame)
    ///        Jump -> LOOP            (back edge)
    /// EXIT:  ELSE; exit point
    /// ```
    fn lower_while(
        &mut self,
        cond: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), Unsupported> {
        let loop_start = self.instrs.len() as u32;
        self.lower_expr(cond)?;
        let j_exit = self.emit_jump(Instr::JumpIfFalse(u32::MAX));
        self.loop_stack.push(LoopFrame {
            breaks: Vec::new(),
            continues: Vec::new(),
            is_for: false,
            opdepth: self.operand_depth,
        });
        self.lower_stmts(body)?;
        let frame = self.loop_stack.pop().expect("while frame pushed");
        // Back edge: `continue` and fall-through both re-test the cond.
        self.patch_all(&frame.continues, loop_start);
        self.emit_jump(Instr::Jump(loop_start));
        // Exit arc: false-cond lands here; breaks jump past the else.
        let exit_here = self.instrs.len() as u32;
        self.patch_jump(j_exit, exit_here);
        self.lower_stmts(orelse)?;
        self.patch_all(&frame.breaks, self.instrs.len() as u32);
        Ok(())
    }

    /// `for T in ITER: BODY else: ELSE` —
    /// ```text
    ///        eval ITER; GetIter
    /// LOOP:  ForNext -> EXIT   (exhaustion; also the `continue` target)
    ///        store T
    ///        BODY
    ///        Jump -> LOOP         (back edge)
    /// EXIT:  ELSE; exit point
    /// ```
    /// ForNext IS the continue point: CPython re-enters FOR_ITER directly
    /// on continue (next pull, same store-arc skip).
    fn lower_for(
        &mut self,
        target: &Target,
        iter: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
    ) -> Result<(), Unsupported> {
        self.lower_expr(iter)?;
        self.emit_at_line(Instr::GetIter, self.cur_line);
        let loop_start = self.instrs.len() as u32;
        let j_exit = self.emit_jump(Instr::ForNext(u32::MAX));
        self.lower_store_target(target)?;
        // The iterator handle sits on the operand stack for the whole
        // loop: statement boundaries inside are ONE deeper (the dispatch
        // loop's exc unwind preserves it — try-in-loop keeps its handle).
        self.operand_depth += 1;
        self.loop_stack.push(LoopFrame {
            breaks: Vec::new(),
            continues: Vec::new(),
            is_for: true,
            opdepth: self.operand_depth,
        });
        self.lower_stmts(body)?;
        let frame = self.loop_stack.pop().expect("for frame pushed");
        self.patch_all(&frame.continues, loop_start);
        self.emit_jump(Instr::Jump(loop_start));
        let exit_here = self.instrs.len() as u32;
        self.patch_jump(j_exit, exit_here);
        self.lower_stmts(orelse)?;
        self.patch_all(&frame.breaks, self.instrs.len() as u32);
        self.operand_depth -= 1;
        Ok(())
    }

    // --- exceptions (leaf fp-exceptions) ------------------------------------
    //
    // CPython-3.11-style zero-cost try ranges: guarded bytecode gets an
    // (start, end, handler, stack_depth) arc; the dispatch loop consults
    // the table ONLY when an exception is in flight. Finally bodies are
    // DUPLICATED epilogues (CPython 3.11's thread-local finally style):
    // one copy per exit arc (normal / in-flight-propagate), with the
    // in-flight copy ending in ResumeUnwind. Finally-pending is the
    // interp's in-flight machinery itself (current_exception / parked
    // Flow) — no extra runtime slot.

    /// `try: BODY [handlers] [else: ELSE] [finally: FIN]`.
    fn lower_try(&mut self, t: &ap_ir::item::TryStmt) -> Result<(), Unsupported> {
        let has_handlers = !t.handlers.is_empty();
        let has_fin = !t.finalbody.is_empty();
        if !has_handlers && !has_fin {
            return Err(Unsupported("try without except or finally"));
        }
        match (has_handlers, has_fin) {
            (true, false) => self.lower_try_handlers(t),
            (false, true) => self.lower_try_finally_only(t),
            (true, true) => self.lower_try_handlers_with_finally(t),
            (false, false) => unreachable!("rejected above"),
        }
    }

    /// try + finally (no except handlers):
    /// ```text
    ///   [range: body]                       (table arc -> unwinding epilogue)
    ///   epilogue-normal (duplicated copy)
    ///   Jump done
    /// handler:
    ///   epilogue-unwind (duplicated copy); ResumeUnwind
    /// done:
    /// ```
    /// The finally body lowers TWICE (bytecode pays code size; the
    /// non-exception path stays zero-cost). Statements inside the
    /// finally that are loop breaks/continues/returns route through the
    /// ordinary lower_stmt machinery, which emits the winning
    /// jump/return — CPython's break-in-finally-override falls out.
    fn lower_try_finally_only(&mut self, t: &ap_ir::item::TryStmt) -> Result<(), Unsupported> {
        let (start, depth, opdepth) = self.begin_range();
        // FINALLY-PENDING: the finalbody is open for everything lowered
        // inside the guarded body (returns/breaks/continues inline it).
        self.fin_bodies.push(t.finalbody.clone());
        self.lower_stmts(&t.body)?;
        self.fin_bodies.pop();
        let end = self.instrs.len() as u32;
        // Normal path: run the epilogue copy (direct emission — a return
        // inside it emits ReturnValue, CPython's override), then jump
        // over the unwinding copy.
        let saved_inside = self.inside_finally;
        self.inside_finally = true;
        self.lower_stmts(&t.finalbody)?;
        self.inside_finally = saved_inside;
        let j_done = self.emit_jump(Instr::Jump(u32::MAX));
        let handler = self.instrs.len() as u32;
        self.end_range(start, end, handler, depth, opdepth);
        // Unwinding path: same body, then resume the parked unwind.
        self.lower_stmts(&t.finalbody)?;
        self.emit_at_line(Instr::ResumeUnwind, self.cur_line);
        let done = self.instrs.len() as u32;
        self.patch_jump(j_done, done);
        Ok(())
    }

    /// try + except handlers (+ else), no finally:
    /// ```text
    ///   [range: body+orelse]
    ///   Jump done
    /// matcher-1: <ty-expr>; CheckExcept ->next | CheckExceptBare
    ///            [`as n`: StoreFast(n) | PopTop]
    ///            handler-body; PopExc; Jump done
    /// matcher-2: ...
    /// nomatch:   ResumeUnwind
    /// done:
    /// ```
    /// One table arc per handler matcher (same guarded span, handlers in
    /// source order — find_handler's scan order preserves matcher order
    /// for equal spans because arcs are recorded in source order and the
    /// CheckExcept fall-through chains the matchers).
    fn lower_try_handlers(&mut self, t: &ap_ir::item::TryStmt) -> Result<(), Unsupported> {
        let body_start = self.instrs.len() as u32;
        let depth = self.try_depth;
        let opdepth = self.operand_depth;
        self.try_depth += 1;
        self.lower_stmts(&t.body)?;
        self.lower_stmts(&t.orelse)?;
        self.try_depth -= 1;
        let body_end = self.instrs.len() as u32;
        let mut done_jumps: Vec<u32> = vec![self.emit_jump(Instr::Jump(u32::MAX))];
        let mut checkexcepts: Vec<u32> = Vec::new();
        for h in &t.handlers {
            let matcher_at = self.instrs.len() as u32;
            self.excs.push(crate::exctable::ExcEntry {
                start: body_start,
                end: body_end,
                handler: matcher_at,
                stack_depth: opdepth,
                depth,
            });
            self.cur_line = body_start.max(1) as usize;
            if let Some(ty) = &h.ty {
                self.lower_expr(ty)?;
                // CheckExcept target = the NO-MATCH continuation (next
                // matcher / no-match arc); the MATCH path falls through
                // to the bind + handler body. Patched after all matchers
                // are emitted.
                let j = self.emit_jump(Instr::CheckExcept(u32::MAX));
                checkexcepts.push(j);
            } else {
                self.emit_at_line(Instr::CheckExceptBare, self.cur_line);
            }
            if let Some(n) = &h.name {
                let slot = self.slot_of(n).ok_or(Unsupported("except-as target"))?;
                self.emit_at_line(Instr::StoreFast(slot), self.cur_line);
            } else {
                self.emit_at_line(Instr::PopTop, self.cur_line);
            }
            self.lower_stmts(&h.body)?;
            // Handler completion: pop the saved-exception latch; an
            // `as n` binding unbinds here (S-02-30). u16::MAX = no bind.
            let unbind = h
                .name
                .as_ref()
                .and_then(|n| self.slot_of(n))
                .unwrap_or(u16::MAX);
            self.emit_at_line(
                Instr::PopExc {
                    unbind_slot: unbind,
                },
                self.cur_line,
            );
            done_jumps.push(self.emit_jump(Instr::Jump(u32::MAX)));
        }
        // No matcher matched: the in-flight exception continues outward.
        let nomatch = self.instrs.len() as u32;
        self.emit_at_line(Instr::ResumeUnwind, self.cur_line);
        // CheckExcept's NO-MATCH arcs land at the NEXT matcher's start —
        // each entry i patches to matcher i+1; the last lands at nomatch.
        let matcher_starts: Vec<u32> = {
            self.excs
                .entries()
                .iter()
                .filter(|e| e.depth == depth && e.end == body_end && e.start == body_start)
                .map(|e| e.handler)
                .collect()
        };
        for (i, &j) in checkexcepts.iter().enumerate() {
            let target = matcher_starts.get(i + 1).copied().unwrap_or(nomatch);
            self.patch_jump(j, target);
        }
        let done = self.instrs.len() as u32;
        self.patch_all(&done_jumps, done);
        Ok(())
    }

    /// try/except/else + finally: the handlers shape lowers exactly like
    /// lower_try_handlers, with its normal exit and no-match exit routed
    /// through the (duplicated) finally epilogues of an OUTER range.
    fn lower_try_handlers_with_finally(
        &mut self,
        t: &ap_ir::item::TryStmt,
    ) -> Result<(), Unsupported> {
        //   [outer range: body+orelse+handler shapes]
        //   <handled exits land here> epilogue-normal; Jump done
        // outer-handler: epilogue-unwind; ResumeUnwind
        // done:
        // Inner arcs: one per matcher (span = body+orelse) AND the outer
        // arc (span = everything up to the outer handler — catches
        // exceptions raised inside handler bodies / matchers, which
        // CPython routes through finally too).
        let outer_start = self.instrs.len() as u32;
        let outer_depth = self.try_depth;
        let outer_opdepth = self.operand_depth;
        self.try_depth += 1;

        // FINALLY-PENDING: open for the body/orelse AND all handler
        // bodies (a return inside any of them inlines the finalbody).
        self.fin_bodies.push(t.finalbody.clone());

        // --- inner handlers shape (exit routing deferred) ---
        let body_start = self.instrs.len() as u32;
        let inner_depth = self.try_depth;
        let inner_opdepth = self.operand_depth;
        self.try_depth += 1;
        self.lower_stmts(&t.body)?;
        self.lower_stmts(&t.orelse)?;
        self.try_depth -= 1;
        let body_end = self.instrs.len() as u32;
        let mut normal_jumps: Vec<u32> = vec![self.emit_jump(Instr::Jump(u32::MAX))];
        let mut unwind_jumps: Vec<u32> = Vec::new();
        let mut checkexcepts: Vec<u32> = Vec::new();
        for h in &t.handlers {
            let matcher_at = self.instrs.len() as u32;
            self.excs.push(crate::exctable::ExcEntry {
                start: body_start,
                end: body_end,
                handler: matcher_at,
                stack_depth: inner_opdepth,
                depth: inner_depth,
            });
            self.cur_line = body_start.max(1) as usize;
            if let Some(ty) = &h.ty {
                self.lower_expr(ty)?;
                // CheckExcept target = NO-MATCH continuation (patched
                // after emission, same discipline as lower_try_handlers).
                let j = self.emit_jump(Instr::CheckExcept(u32::MAX));
                checkexcepts.push(j);
            } else {
                self.emit_at_line(Instr::CheckExceptBare, self.cur_line);
            }
            if let Some(n) = &h.name {
                let slot = self.slot_of(n).ok_or(Unsupported("except-as target"))?;
                self.emit_at_line(Instr::StoreFast(slot), self.cur_line);
            } else {
                self.emit_at_line(Instr::PopTop, self.cur_line);
            }
            self.lower_stmts(&h.body)?;
            // Handler completion: pop the saved-exception latch; an
            // `as n` binding unbinds here (S-02-30). u16::MAX = no bind.
            let unbind = h
                .name
                .as_ref()
                .and_then(|n| self.slot_of(n))
                .unwrap_or(u16::MAX);
            self.emit_at_line(
                Instr::PopExc {
                    unbind_slot: unbind,
                },
                self.cur_line,
            );
            normal_jumps.push(self.emit_jump(Instr::Jump(u32::MAX)));
        }
        self.fin_bodies.pop();
        // No matcher matched: in-flight continues through the finally
        // epilogue (unwinding copy), then re-raises.
        unwind_jumps.push(self.emit_jump(Instr::Jump(u32::MAX)));

        // --- normal-path epilogue (duplicated copy 1) ---
        let normal_target = self.instrs.len() as u32;
        let saved_inside = self.inside_finally;
        self.inside_finally = true;
        self.lower_stmts(&t.finalbody)?;
        let j_done = self.emit_jump(Instr::Jump(u32::MAX));

        // --- outer handler arc: in-flight unwinding epilogue (copy 2) ---
        let outer_handler = self.instrs.len() as u32;
        self.excs.push(crate::exctable::ExcEntry {
            start: outer_start,
            end: outer_handler,
            handler: outer_handler,
            stack_depth: outer_opdepth,
            depth: outer_depth,
        });
        self.try_depth -= 1;
        self.lower_stmts(&t.finalbody)?;
        self.inside_finally = saved_inside;
        self.emit_at_line(Instr::ResumeUnwind, self.cur_line);
        let done = self.instrs.len() as u32;
        self.patch_jump(j_done, done);
        self.patch_all(&normal_jumps, normal_target);
        self.patch_all(&unwind_jumps, outer_handler);
        // CheckExcept NO-MATCH arcs -> the next matcher (or the no-match
        // arc, which routes to the unwinding epilogue = outer_handler).
        let matcher_starts: Vec<u32> = self
            .excs
            .entries()
            .iter()
            .filter(|e| e.depth == inner_depth && e.end == body_end && e.start == body_start)
            .map(|e| e.handler)
            .collect();
        for (i, &j) in checkexcepts.iter().enumerate() {
            let target = matcher_starts.get(i + 1).copied().unwrap_or(outer_handler);
            self.patch_jump(j, target);
        }
        Ok(())
    }

    /// Open a guarded range at the current pc — returns (start, depth,
    /// operand_depth) for the matching end_range. operand_depth records
    /// the compile-time operand-stack residue at statement boundaries
    /// (loop iterators the dispatch loop must preserve across unwinds).
    fn begin_range(&mut self) -> (u32, u16, u32) {
        let start = self.instrs.len() as u32;
        let depth = self.try_depth;
        let opdepth = self.operand_depth;
        self.try_depth += 1;
        (start, depth, opdepth)
    }

    /// Close the guarded range (record the table entry; the handler arc
    /// is `handler`, emitted AFTER the normal-path Jump over it).
    fn end_range(&mut self, start: u32, end: u32, handler: u32, depth: u16, opdepth: u32) {
        self.try_depth -= 1;
        self.excs.push(crate::exctable::ExcEntry {
            start,
            end,
            handler,
            stack_depth: opdepth,
            depth,
        });
    }

    fn lower_store_target(&mut self, t: &Target) -> Result<(), Unsupported> {
        match t {
            Target::Name(name) => {
                match self.slot_of(name) {
                    Some(slot) => {
                        self.emit_at_line(Instr::StoreFast(slot), self.cur_line);
                    }
                    None => {
                        // Not scanned as local => global store (module
                        // namespace). Function bodies always scan, so a
                        // miss here means module-level code.
                        let idx = self.intern_name(name);
                        self.emit_at_line(Instr::StoreGlobal(idx), self.cur_line);
                    }
                }
                Ok(())
            }
            Target::Subscript(obj, key) => {
                // Stack: value (TOS2) obj (TOS1) key (TOS0).
                self.lower_expr(obj)?;
                self.lower_expr(key)?;
                self.emit_at_line(Instr::StoreSubscript, self.cur_line);
                Ok(())
            }
            Target::Attribute(obj, name) => {
                // Leaf 02-calls: `obj.attr = v` lowers to StoreAttr — the
                // SAME classes::attr_set_protocol route the tree-walker's
                // Target::Attribute arm takes (user __setattr__ included).
                // Stack: value (TOS1) obj (TOS0) — Assign pushed the value
                // first; the interpreter pops obj then value.
                self.lower_expr(obj)?;
                let idx = self.intern_name(name);
                self.emit_at_line(Instr::StoreAttr { idx }, self.cur_line);
                Ok(())
            }
            Target::Tuple(..) => Err(Unsupported("tuple unpack assign")),
            Target::Starred(_) => Err(Unsupported("starred assign")),
        }
    }

    fn lower_load_target(&mut self, t: &Target) -> Result<(), Unsupported> {
        match t {
            Target::Name(_) => self.lower_expr(&Expr::Name(match t {
                Target::Name(n) => n.clone(),
                _ => unreachable!(),
            })),
            Target::Subscript(obj, key) => {
                self.lower_expr(&Expr::Subscript(obj.clone(), key.clone(), self.cur_line))
            }
            Target::Attribute(obj, name) => {
                // Aug-target read: plain LoadAttr (same route as Expr::Attribute).
                self.lower_expr(obj)?;
                let idx = self.intern_name(name);
                let site = self.sites.alloc(SiteKind::AttrLoad, 0);
                let ins_idx = self.emit(Instr::LoadAttr { idx, site });
                self.sites.retarget(ins_idx, site);
                self.lines.record(ins_idx, self.cur_line);
                Ok(())
            }
            Target::Tuple(..) => Err(Unsupported("tuple aug-target")),
            Target::Starred(_) => Err(Unsupported("starred aug-target")),
        }
    }

    // --- expressions --------------------------------------------------------

    fn lower_expr(&mut self, e: &Expr) -> Result<(), Unsupported> {
        match e {
            Expr::LitInt(v) => {
                let c = match i64::try_from(*v) {
                    Ok(small) => Const::Int(small),
                    Err(_) => Const::BigInt(v.to_string()),
                };
                let idx = self.consts.intern(c);
                self.emit_at_line(Instr::LoadConst(idx), self.cur_line);
                Ok(())
            }
            Expr::LitBigInt(digits) => {
                let idx = self.consts.intern(Const::BigInt(digits.clone()));
                self.emit_at_line(Instr::LoadConst(idx), self.cur_line);
                Ok(())
            }
            Expr::LitFloat(f) => {
                let idx = self.consts.intern(Const::Float(*f));
                self.emit_at_line(Instr::LoadConst(idx), self.cur_line);
                Ok(())
            }
            Expr::LitComplex(..) => Err(Unsupported("complex literal")),
            Expr::LitStr(s) => {
                let idx = self.consts.intern(Const::Str(s.clone()));
                self.emit_at_line(Instr::LoadConst(idx), self.cur_line);
                Ok(())
            }
            Expr::LitBytes(b) => {
                let idx = self.consts.intern(Const::Bytes(b.clone()));
                self.emit_at_line(Instr::LoadConst(idx), self.cur_line);
                Ok(())
            }
            Expr::LitBool(true) => {
                self.emit_at_line(Instr::LoadConstTrue, self.cur_line);
                Ok(())
            }
            Expr::LitBool(false) => {
                self.emit_at_line(Instr::LoadConstFalse, self.cur_line);
                Ok(())
            }
            Expr::LitNone => {
                self.emit_at_line(Instr::LoadConstNone, self.cur_line);
                Ok(())
            }
            Expr::LitEllipsis => Err(Unsupported("ellipsis literal")),
            Expr::Name(name) => {
                match self.slot_of(name) {
                    Some(slot) => {
                        self.emit_at_line(Instr::LoadFast(slot), self.cur_line);
                    }
                    None => {
                        let idx = self.intern_name(name);
                        let site = self.sites.alloc(SiteKind::GlobalLoad, {
                            // site's instr_idx fills after emit; alloc
                            // BEFORE emit and backpatch via a second
                            // alloc-time index is avoided by emitting
                            // first — see below.
                            0
                        });
                        let ins_idx = self.emit(Instr::LoadGlobal(idx));
                        // Fix the recorded instr_idx (allocated before we
                        // knew it).
                        self.sites.retarget(ins_idx, site);
                        self.lines.record(ins_idx, self.cur_line);
                    }
                }
                Ok(())
            }
            Expr::BinOp(op, lhs, rhs, line) => {
                self.lower_expr(lhs)?;
                self.lower_expr(rhs)?;
                if *line >= 1 {
                    self.cur_line = *line;
                }
                self.emit_at_line(Instr::BinOp(BinKind::from_ir(*op)), self.cur_line);
                Ok(())
            }
            Expr::UnaryOp(op, operand, line) => {
                self.lower_expr(operand)?;
                if *line >= 1 {
                    self.cur_line = *line;
                }
                self.emit_at_line(Instr::UnaryOp(UnaryKind::from_ir(*op)), self.cur_line);
                Ok(())
            }
            Expr::BoolOp(op, xs) => {
                // Short-circuit Jump chains preserving the VALUE rule:
                // `a or b` yields a's VALUE when truthy (not a bool). CPython
                // 3.12+ emits JUMP_IF_FALSE/JUMP_IF_TRUE **or-pop** style;
                // the TOS stays on the stack through each undecided
                // operand, and the last operand's value is the result.
                //
                // Layout for `x0 and x1 and x2`:
                //   eval x0; JumpIfFalseOrPop -> DONE   (x0 falsy = result)
                //   eval x1; JumpIfFalseOrPop -> DONE
                //   eval x2                             (last: no jump)
                // DONE:
                // `or` mirrors with JumpIfTrueOrPop.
                let is_and = matches!(op, ap_ir::BoolOp::And);
                let kind = if is_and {
                    Instr::JumpIfFalseOrPop(u32::MAX)
                } else {
                    Instr::JumpIfTrueOrPop(u32::MAX)
                };
                let mut jumps: Vec<u32> = Vec::new();
                for (i, x) in xs.iter().enumerate() {
                    self.lower_expr(x)?;
                    if i + 1 < xs.len() {
                        jumps.push(self.emit_jump(kind));
                    }
                }
                let done = self.instrs.len() as u32;
                self.patch_all(&jumps, done);
                Ok(())
            }
            Expr::Compare(ops, operands, line) => {
                // Pairwise single comparison lowers; chains (2+ ops) need
                // short-circuit jumps (01-core) — the IR evaluates each
                // middle once, and emitting both Compares would
                // double-evaluate. Byte-exactness forbids that.
                if ops.len() != 1 || operands.len() != 2 {
                    return Err(Unsupported("chained comparison"));
                }
                self.lower_expr(&operands[0])?;
                self.lower_expr(&operands[1])?;
                if *line >= 1 {
                    self.cur_line = *line;
                }
                self.emit_at_line(Instr::Compare(CompareOp::from_ir(ops[0])), self.cur_line);
                Ok(())
            }
            Expr::Call(callee, args, line) => self.lower_call(callee, args, *line),
            Expr::Attribute(obj, attr, line) => {
                self.lower_expr(obj)?;
                if *line >= 1 {
                    self.cur_line = *line;
                }
                let idx = self.intern_name(attr);
                let site = self.sites.alloc(SiteKind::AttrLoad, 0);
                let ins_idx = self.emit(Instr::LoadAttr { idx, site });
                self.sites.retarget(ins_idx, site);
                self.lines.record(ins_idx, self.cur_line);
                Ok(())
            }
            Expr::Subscript(obj, key, line) => {
                self.lower_expr(obj)?;
                self.lower_expr(key)?;
                if *line >= 1 {
                    self.cur_line = *line;
                }
                self.emit_at_line(Instr::Subscript, self.cur_line);
                Ok(())
            }
            Expr::Slice(..) => Err(Unsupported("slice expression")),
            Expr::List(items) => {
                for it in items {
                    self.lower_expr(it)?;
                }
                let count = u16::try_from(items.len())
                    .map_err(|_| Unsupported("list display too large"))?;
                self.emit_at_line(Instr::BuildList(count), self.cur_line);
                Ok(())
            }
            Expr::Tuple(items) => {
                // `()` is the empty-tuple singleton fast path.
                if items.is_empty() {
                    self.emit_at_line(Instr::LoadConstEmptyTuple, self.cur_line);
                    return Ok(());
                }
                for it in items {
                    self.lower_expr(it)?;
                }
                let count = u16::try_from(items.len())
                    .map_err(|_| Unsupported("tuple display too large"))?;
                self.emit_at_line(Instr::BuildTuple(count), self.cur_line);
                Ok(())
            }
            Expr::Set(items) => {
                for it in items {
                    self.lower_expr(it)?;
                }
                let count =
                    u16::try_from(items.len()).map_err(|_| Unsupported("set display too large"))?;
                self.emit_at_line(Instr::BuildSet(count), self.cur_line);
                Ok(())
            }
            Expr::Dict(items) => {
                // Key first, value second (insertion order == push order;
                // BuildDict pairs chunks of 2 in push order). A ** expansion
                // item (DictExpand) stays unsupported.
                for i in items {
                    self.lower_expr(&i.key)?;
                    self.lower_expr(&i.value)?;
                }
                let count = u16::try_from(items.len())
                    .map_err(|_| Unsupported("dict display too large"))?;
                self.emit_at_line(Instr::BuildDict(count), self.cur_line);
                Ok(())
            }
            Expr::DictExpand(_) => Err(Unsupported("dict ** expansion")),
            Expr::Lambda(..) => Err(Unsupported("lambda")),
            Expr::ListComp(elt, clauses) => {
                self.lower_comprehension(CompShape::List, elt, None, clauses)
            }
            Expr::SetComp(elt, clauses) => {
                self.lower_comprehension(CompShape::Set, elt, None, clauses)
            }
            Expr::DictComp(k, v, clauses) => {
                self.lower_comprehension(CompShape::Dict, k, Some(v), clauses)
            }
            Expr::Generator(..) => Err(Unsupported("generator expression")),
            Expr::Yield(_) => Err(Unsupported("yield")),
            Expr::YieldFrom(_) => Err(Unsupported("yield from")),
            Expr::Await(_) => Err(Unsupported("await")),
            Expr::FString(_) => Err(Unsupported("f-string")),
            Expr::IfExp(..) => Err(Unsupported("conditional expression")),
            Expr::Starred(_) => Err(Unsupported("starred expression")),
            Expr::Named(..) => Err(Unsupported("walrus expression")),
        }
    }

    fn lower_call(&mut self, callee: &Expr, args: &[Arg], line: usize) -> Result<(), Unsupported> {
        // Positional + KEYWORD calls lower (leaf 02-calls): kwargs marshal
        // through CallFunctionKw (a name-table slice) into call_value's
        // kwargs slice — the tree-walker's exact binding machinery
        // (defaults / kw-only / **kwargs binding live in the call
        // boundary). *args/**kwargs CALL-SITE shapes (runtime unpack)
        // stay unsupported this leaf — loud per-function fallback.
        let mut pos_args = 0usize;
        let mut kw_names: Vec<&str> = Vec::new();
        let mut kw_exprs: Vec<&Expr> = Vec::new();
        for a in args {
            match a {
                Arg::Pos(_) => pos_args += 1,
                Arg::Kw(name, e) => {
                    kw_names.push(name);
                    kw_exprs.push(e);
                }
                Arg::Star(_) => return Err(Unsupported("*args call")),
                Arg::KwStar(_) => return Err(Unsupported("**kwargs call")),
            }
        }
        let argc = u16::try_from(pos_args).map_err(|_| Unsupported("call too many args"))?;
        let n_kw =
            u16::try_from(kw_names.len()).map_err(|_| Unsupported("call too many kwargs"))?;
        // FUSED METHOD CALL (leaf 01-core/01): `obj.name(args...)` — the
        // build-loop `out.append(x)` shape. Lowering LoadAttr then
        // CallFunction mints a fresh BoundMethod per iteration (~1.5us
        // alloc); the fused opcode routes receiver+name+args straight to
        // the method dispatcher (mint only as the fallback inside the
        // dispatcher). Kwarg'd method calls (rare) drop to the generic
        // CallFunctionKw arm below — the fused opcode carries no kw table.
        if n_kw == 0 {
            if let Expr::Attribute(base, attr, _) = callee {
                self.lower_expr(base)?;
                for a in args {
                    if let Arg::Pos(e) = a {
                        self.lower_expr(e)?;
                    }
                }
                if line >= 1 {
                    self.cur_line = line;
                }
                let idx = self.intern_name(attr);
                // Fused calls get their own Call-kind site (leaf
                // fp-inlinecache): the dispatcher's (receiver-tag, name)
                // decision memoizes here.
                let site = self.sites.alloc(SiteKind::Call, 0);
                self.emit_at_line(Instr::LoadAttrCall { idx, argc, site }, self.cur_line);
                let ins_idx = self.cur_instr_idx();
                self.sites.retarget(ins_idx, site);
                return Ok(());
            }
            self.lower_expr(callee)?;
            for a in args {
                if let Arg::Pos(e) = a {
                    self.lower_expr(e)?;
                }
            }
            if line >= 1 {
                self.cur_line = line;
            }
            let site = self.sites.alloc(SiteKind::Call, 0);
            let ins_idx = self.emit(Instr::CallFunction { argc, site });
            self.sites.retarget(ins_idx, site);
            self.lines.record(ins_idx, line.max(1));
            return Ok(());
        }
        // Kwarg'd call: callee, positionals, then kwarg VALUES in kw-name
        // order (names interned FIRST so the table slice is contiguous,
        // then the expressions lower — value order == name order).
        self.lower_expr(callee)?;
        for a in args {
            if let Arg::Pos(e) = a {
                self.lower_expr(e)?;
            }
        }
        // Reserve a CONTIGUOUS name-table slice [kw_first, kw_first+n_kw).
        // intern_name's dedup could return an EARLIER index when a kw name
        // was already interned (an attr/global read above) — that would
        // desync the value/name pairing at dispatch. Always push a FRESH
        // entry here: a duplicate table entry is harmless (nothing looks
        // names up by content; only emitted indices are read back).
        let kw_first = self.names.len() as u32;
        for name in &kw_names {
            self.names.push((*name).to_string());
        }
        for e in &kw_exprs {
            self.lower_expr(e)?;
        }
        if line >= 1 {
            self.cur_line = line;
        }
        let site = self.sites.alloc(SiteKind::Call, 0);
        let ins_idx = self.emit(Instr::CallFunctionKw {
            argc,
            kw_first,
            n_kw,
            site,
        });
        self.sites.retarget(ins_idx, site);
        self.lines.record(ins_idx, line.max(1));
        Ok(())
    }

    // --- comprehensions (leaf 02-calls) -------------------------------------

    /// Lower `[elt for t in it if c...]` / set / dict shapes to an INLINE
    /// loop — NO separate function frame. The comprehension's implicit
    /// function scope is emulated with a slot namespace pushed onto
    /// comp_scopes: clause targets (and a fresh accumulator) live in hidden
    /// slots that slot_of shadows and sync_slots_into_frame never exports.
    ///
    /// CPython scope semantics preserved:
    /// - the OUTERMOST clause's iterable evaluates in the ENCLOSING scope
    ///   (lowered before the scope push),
    /// - inner clause iterables evaluate INSIDE the comprehension scope
    ///   (they may reference outer clause targets),
    /// - clause targets never leak into the enclosing frame,
    /// - walrus inside a comp binds in the enclosing scope — NOT supported
    ///   here (Expr::Named stays a loud fallback, S-02-2 exact).
    ///
    /// GeneratorExp stays tree-walked (fiber machinery, master.md).
    fn lower_comprehension(
        &mut self,
        shape: CompShape,
        elt: &Expr,
        elt2: Option<&Box<Expr>>,
        clauses: &[ap_ir::expr::CompClause],
    ) -> Result<(), Unsupported> {
        if clauses.is_empty() {
            return Err(Unsupported("comprehension with no clauses"));
        }
        // Fresh accumulator — a NEW list/set/dict in a HIDDEN slot (the
        // enclosing frame's same-named local is never touched).
        let acc_slot = self.alloc_hidden_slot("<comp_acc>");
        // OUTERMOST iterable: ENCLOSING scope (CPython evaluates it in the
        // caller's frame before the implicit fn runs).
        self.lower_expr(&clauses[0].iter)?;
        self.emit_at_line(Instr::GetIter, self.cur_line);
        match shape {
            CompShape::List => self.emit_at_line(Instr::BuildList(0), self.cur_line),
            CompShape::Set => self.emit_at_line(Instr::BuildSet(0), self.cur_line),
            CompShape::Dict => self.emit_at_line(Instr::BuildDict(0), self.cur_line),
        }
        self.emit_at_line(Instr::StoreFast(acc_slot), self.cur_line);

        // The implicit function scope: everything from the first ForNext on
        // (inner iterables, targets, filters, element) evaluates inside it.
        // Clause targets allocate as HIDDEN slots through comp_target_slot;
        // sync_slots_into_frame exports visible local_names only, so the
        // comp's scope never leaks into the enclosing frame (S-02-2).
        self.comp_scopes.push(HashMap::new());

        // Per-clause ForNext instruction indices — the clause's continue
        // point (filter-false + back-edge target) and its exhaustion patch
        // site. Stack per clause: [.., it0, it1, ..] — handles are PEEKED
        // by ForNext and popped on exhaustion, so the inner exhaustion arc
        // jumping to the outer ForNext leaves the OUTER handle on top.
        let mut clause_fornexts: Vec<u32> = Vec::new();
        for (ci, clause) in clauses.iter().enumerate() {
            if ci > 0 {
                // Inner iterable: comp-scope evaluation (may read outer
                // clause targets — the scope is already pushed).
                self.lower_expr(&clause.iter)?;
                self.emit_at_line(Instr::GetIter, self.cur_line);
            }
            let fn_idx = self.emit_jump(Instr::ForNext(u32::MAX));
            clause_fornexts.push(fn_idx);
            let slot = self.comp_target_slot(&clause.target)?;
            self.emit_at_line(Instr::StoreFast(slot), self.cur_line);
            // Filters: false -> next iteration of THIS clause (its ForNext).
            for f in &clause.ifs {
                self.lower_expr(f)?;
                self.emit_jump(Instr::JumpIfFalse(fn_idx));
            }
        }

        // Body: element expression(s), comp-scope evaluated.
        // List/set shapes: LoadFast(acc) FIRST so the fused LoadAttrCall
        // sees receiver=acc under the arg (stack [acc, elt]); the elt
        // lowers AFTER the receiver push. Dict comps lower key/value into
        // temp slots, then rebuild [acc, key, value] for StoreSubscript.
        match shape {
            CompShape::List => {
                let append_idx = self.intern_name("append");
                self.emit_at_line(Instr::LoadFast(acc_slot), self.cur_line);
                self.lower_expr(elt)?;
                // acc.append(elt); PopTop — append's None result discards
                // (the fused opcode never mints the BoundMethod).
                let append_site = self.sites.alloc(SiteKind::Call, 0);
                self.emit_at_line(
                    Instr::LoadAttrCall {
                        idx: append_idx,
                        argc: 1,
                        site: append_site,
                    },
                    self.cur_line,
                );
                let ins_idx = self.cur_instr_idx();
                self.sites.retarget(ins_idx, append_site);
                self.emit_at_line(Instr::PopTop, self.cur_line);
            }
            CompShape::Set => {
                let add_idx = self.intern_name("add");
                self.emit_at_line(Instr::LoadFast(acc_slot), self.cur_line);
                self.lower_expr(elt)?;
                let add_site = self.sites.alloc(SiteKind::Call, 0);
                self.emit_at_line(
                    Instr::LoadAttrCall {
                        idx: add_idx,
                        argc: 1,
                        site: add_site,
                    },
                    self.cur_line,
                );
                let ins_idx = self.cur_instr_idx();
                self.sites.retarget(ins_idx, add_site);
                self.emit_at_line(Instr::PopTop, self.cur_line);
            }
            CompShape::Dict => {
                self.lower_expr(elt)?; // key
                if let Some(v) = elt2 {
                    self.lower_expr(v)?; // value
                }
                // Stack [.., key, value]; no rotate opcode this leaf — park
                // key/value in temp slots, rebuild [acc, key, value].
                let k_slot = self.alloc_hidden_slot("<comp_k>");
                let v_slot = self.alloc_hidden_slot("<comp_v>");
                self.emit_at_line(Instr::StoreFast(v_slot), self.cur_line);
                self.emit_at_line(Instr::StoreFast(k_slot), self.cur_line);
                // StoreSubscript stack discipline (lower_store_target's
                // layout): value TOS2, obj TOS1, key TOS0 — so emit
                // value, acc, key.
                self.emit_at_line(Instr::LoadFast(v_slot), self.cur_line);
                self.emit_at_line(Instr::LoadFast(acc_slot), self.cur_line);
                self.emit_at_line(Instr::LoadFast(k_slot), self.cur_line);
                self.emit_at_line(Instr::StoreSubscript, self.cur_line);
            }
        }
        // Back edge to the innermost clause's ForNext.
        let innermost = *clause_fornexts.last().expect("clauses non-empty");
        self.emit_jump(Instr::Jump(innermost));

        // Exit arcs: inner-clause exhaustion resumes the OUTER loop (its
        // handle is exposed on top after the inner pops); clause 0's
        // exhaustion exits the whole comprehension.
        let exit_here = self.instrs.len() as u32;
        for ci in 1..clauses.len() {
            self.patch_jump(clause_fornexts[ci], clause_fornexts[ci - 1]);
        }
        self.patch_jump(clause_fornexts[0], exit_here);

        // Leave the implicit scope (targets stay hidden — sync exports
        // visible local_names only).
        self.comp_scopes.pop().expect("comp scope pushed");

        // The comprehension's value.
        self.emit_at_line(Instr::LoadFast(acc_slot), self.cur_line);
        Ok(())
    }
}

/// Comprehension result shape (accumulator protocol per shape).
enum CompShape {
    List,
    Set,
    Dict,
}

/// Collect every name ASSIGNED in the statement tree (StoreFast scan).
/// Recurses into If/While/For/Try bodies etc. — a name assigned only
/// inside an un-taken branch is still a compile-time local (Python
/// scoping rule); the function falls back wholesale anyway when those
/// statements are present, but the scan stays shape-agnostic so slot
/// numbering is stable if 01-core lifts the rejection.
fn scan_assigned_names(stmts: &[Stmt], out: &mut Vec<String>) {
    for s in stmts {
        match s {
            Stmt::Assign(targets, value) => {
                for t in targets {
                    collect_target_names(t, out);
                }
                scan_assigned_expr_walrus(value, out);
            }
            Stmt::AugAssign(t, _, v) => {
                collect_target_names(t, out);
                scan_assigned_expr_walrus(v, out);
            }
            Stmt::AnnAssign(t, _, v) => {
                collect_target_names(t, out);
                if let Some(v) = v {
                    scan_assigned_expr_walrus(v, out);
                }
            }
            Stmt::If(c, body, orelse) => {
                scan_assigned_expr_walrus(c, out);
                scan_assigned_names(body, out);
                scan_assigned_names(orelse, out);
            }
            Stmt::While(c, body, orelse) => {
                scan_assigned_expr_walrus(c, out);
                scan_assigned_names(body, out);
                scan_assigned_names(orelse, out);
            }
            Stmt::For(t, it, body, orelse) => {
                collect_target_names(t, out);
                scan_assigned_expr_walrus(it, out);
                scan_assigned_names(body, out);
                scan_assigned_names(orelse, out);
            }
            Stmt::With(items, body) => {
                for i in items {
                    scan_assigned_expr_walrus(&i.expr, out);
                    if let Some(t) = &i.target {
                        collect_target_names(t, out);
                    }
                }
                scan_assigned_names(body, out);
            }
            Stmt::Try(t) => {
                scan_assigned_names(&t.body, out);
                for h in &t.handlers {
                    if let Some(n) = &h.name {
                        out.push(n.clone());
                    }
                    scan_assigned_names(&h.body, out);
                }
                scan_assigned_names(&t.orelse, out);
                scan_assigned_names(&t.finalbody, out);
            }
            Stmt::Raise(e, cause) => {
                if let Some(e) = e {
                    scan_assigned_expr_walrus(e, out);
                }
                if let Some(c) = cause {
                    scan_assigned_expr_walrus(c, out);
                }
            }
            Stmt::Match(subject, cases) => {
                scan_assigned_expr_walrus(subject, out);
                for c in cases {
                    if let Some(g) = &c.guard {
                        scan_assigned_expr_walrus(g, out);
                    }
                    scan_assigned_names(&c.body, out);
                }
            }
            Stmt::Assert(t, msg) => {
                scan_assigned_expr_walrus(t, out);
                if let Some(m) = msg {
                    scan_assigned_expr_walrus(m, out);
                }
            }
            Stmt::Return(Some(e)) | Stmt::ExprStmt(e) => scan_assigned_expr_walrus(e, out),
            Stmt::Del(targets) => {
                for t in targets {
                    collect_target_names(t, out);
                }
            }
            Stmt::LocalDef(f) => {
                scan_assigned_names(&f.body.blocks, out);
            }
            Stmt::LocalClass(c) => scan_assigned_names(&c.body, out),
            _ => {}
        }
    }
}

fn scan_assigned_expr_walrus(e: &Expr, out: &mut Vec<String>) {
    // Expr::Named(target, value): walrus BINDS in the enclosing scope.
    if let Expr::Named(t, v) = e {
        scan_assigned_expr_walrus(t, out);
        scan_assigned_expr_walrus(v, out);
        if let Expr::Name(n) = &**t {
            out.push(n.clone());
        }
        return;
    }
    // Recurse into container shapes so nested walrus binds register.
    match e {
        Expr::BinOp(_, a, b, _) => {
            scan_assigned_expr_walrus(a, out);
            scan_assigned_expr_walrus(b, out);
        }
        Expr::Attribute(a, _, _) => {
            scan_assigned_expr_walrus(a, out);
        }
        Expr::Named(a, b) => {
            scan_assigned_expr_walrus(a, out);
            scan_assigned_expr_walrus(b, out);
        }
        Expr::Subscript(a, b, _) => {
            scan_assigned_expr_walrus(a, out);
            scan_assigned_expr_walrus(b, out);
        }
        Expr::UnaryOp(_, a, _) => scan_assigned_expr_walrus(a, out),
        Expr::Compare(_, ops, _) => {
            for o in ops {
                scan_assigned_expr_walrus(o, out);
            }
        }
        Expr::Call(f, args, _) => {
            scan_assigned_expr_walrus(f, out);
            for a in args {
                match a {
                    Arg::Pos(e) | Arg::Star(e) | Arg::KwStar(e) => {
                        scan_assigned_expr_walrus(e, out)
                    }
                    Arg::Kw(_, e) => scan_assigned_expr_walrus(e, out),
                }
            }
        }
        Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => {
            for x in xs {
                scan_assigned_expr_walrus(x, out);
            }
        }
        Expr::Dict(items) => {
            for i in items {
                scan_assigned_expr_walrus(&i.key, out);
                scan_assigned_expr_walrus(&i.value, out);
            }
        }
        Expr::BoolOp(_, xs) => {
            for x in xs {
                scan_assigned_expr_walrus(x, out);
            }
        }
        Expr::IfExp(a, b, c, _) => {
            scan_assigned_expr_walrus(a, out);
            scan_assigned_expr_walrus(b, out);
            scan_assigned_expr_walrus(c, out);
        }
        Expr::Starred(a) | Expr::Await(a) | Expr::Yield(Some(a)) | Expr::YieldFrom(a) => {
            scan_assigned_expr_walrus(a, out)
        }
        _ => {}
    }
}

/// The operator's source spelling (CPython aug error-message fidelity:
/// "unsupported operand type(s) for +=: 'int' and 'str'").
fn op_symbol(op: ap_ir::BinOp) -> &'static str {
    use ap_ir::BinOp as B;
    match op {
        B::Add => "+=",
        B::Sub => "-=",
        B::Mul => "*=",
        B::MatMul => "@=",
        B::Div => "/=",
        B::FloorDiv => "//=",
        B::Mod => "%=",
        B::Pow => "**=",
        B::LShift => "<<=",
        B::RShift => ">>=",
        B::BitOr => "|=",
        B::BitXor => "^=",
        B::BitAnd => "&=",
    }
}

fn collect_target_names(t: &Target, out: &mut Vec<String>) {
    match t {
        Target::Name(n) => out.push(n.clone()),
        Target::Tuple(ts) => {
            for t in ts {
                collect_target_names(t, out);
            }
        }
        Target::Starred(t) => {
            // `*rest` collects a name (or a nested pattern expr); only the
            // plain-name case contributes a slot.
            if let Expr::Name(n) = &**t {
                out.push(n.clone());
            }
        }
        Target::Subscript(..) | Target::Attribute(..) => {}
    }
}

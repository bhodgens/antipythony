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
}

/// One enclosing loop's pending break/continue jump sites (instruction
/// indices whose u32 target gets backpatched).
struct LoopFrame {
    /// `break` jumps -> the loop's AFTER-ORELSE exit point.
    breaks: Vec<u32>,
    /// `continue` jumps -> the loop's CONTINUE point (For: the ForNext;
    /// While: the condition re-eval).
    continues: Vec<u32>,
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
        }
    }

    // --- emission helpers -------------------------------------------------

    fn emit(&mut self, i: Instr) -> u32 {
        let idx = self.instrs.len() as u32;
        self.instrs.push(i);
        idx
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
            | Instr::ForNext(t) => *t = target,
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
        // VarArg/KwArg/KwOnly params need call-binding shapes the dispatch
        // loop does not have yet (01-core 02 owns them).
        for p in &fn_def.params {
            match p.kind {
                ap_ir::ParamKind::PosOnly | ap_ir::ParamKind::PosOrKw => {}
                _ => return Err(Unsupported("non-positional param")),
            }
            // Defaults evaluated at def time are a def-time effect; the
            // frame pre-fills slots. Lowering the default EXPRESSION here
            // would double-execute at call time in leaf 02's frame — out
            // of scope until 02-frame-and-entry wires default prefill.
            if p.default.is_some() {
                return Err(Unsupported("param default"));
            }
        }

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

        Ok(CodeObject {
            name: fn_def.name.clone(),
            def_line: fn_def.def_line,
            n_locals: self.n_locals,
            local_names: self.local_names_sorted(),
            instrs: self.instrs,
            consts: self.consts,
            names: self.names,
            lines: self.lines,
            sites: self.sites,
        })
    }

    fn local_names_sorted(&self) -> Vec<String> {
        let mut v: Vec<(u16, String)> = self.slots.iter().map(|(n, s)| (*s, n.clone())).collect();
        v.sort_by_key(|(s, _)| *s);
        v.into_iter().map(|(_, n)| n).collect()
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
                self.emit_at_line(Instr::ReturnValue, self.cur_line);
                Ok(())
            }
            Stmt::Return(None) => {
                self.emit_at_line(Instr::ReturnNone, self.cur_line);
                Ok(())
            }
            Stmt::If(cond, then, orelse) => self.lower_if(cond, then, orelse),
            Stmt::While(cond, body, orelse) => self.lower_while(cond, body, orelse),
            Stmt::For(target, iter, body, orelse) => self.lower_for(target, iter, body, orelse),
            Stmt::With(..) => Err(Unsupported("with statement")),
            Stmt::Try(..) => Err(Unsupported("try statement")),
            Stmt::Raise(..) => Err(Unsupported("raise statement")),
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
        self.loop_stack.push(LoopFrame {
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.lower_stmts(body)?;
        let frame = self.loop_stack.pop().expect("for frame pushed");
        self.patch_all(&frame.continues, loop_start);
        self.emit_jump(Instr::Jump(loop_start));
        let exit_here = self.instrs.len() as u32;
        self.patch_jump(j_exit, exit_here);
        self.lower_stmts(orelse)?;
        self.patch_all(&frame.breaks, self.instrs.len() as u32);
        Ok(())
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
            Target::Attribute(..) => Err(Unsupported("attribute store")),
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
            Target::Attribute(..) => Err(Unsupported("attribute aug-target")),
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
            Expr::Set(..) => Err(Unsupported("set display")),
            Expr::Dict(..) => Err(Unsupported("dict display")),
            Expr::DictExpand(_) => Err(Unsupported("dict ** expansion")),
            Expr::Lambda(..) => Err(Unsupported("lambda")),
            Expr::ListComp(..) => Err(Unsupported("list comprehension")),
            Expr::SetComp(..) => Err(Unsupported("set comprehension")),
            Expr::DictComp(..) => Err(Unsupported("dict comprehension")),
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
        // Positional-only calls in the start set; kwargs / *args / **kw
        // shapes land with 01-core 02.
        let mut pos_args = 0usize;
        for a in args {
            match a {
                Arg::Pos(_) => pos_args += 1,
                Arg::Kw(..) => return Err(Unsupported("keyword call arg")),
                Arg::Star(_) => return Err(Unsupported("*args call")),
                Arg::KwStar(_) => return Err(Unsupported("**kwargs call")),
            }
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
        let ins_idx = self.emit(Instr::CallFunction {
            argc: u16::try_from(pos_args).map_err(|_| Unsupported("call too many args"))?,
            site,
        });
        self.sites.retarget(ins_idx, site);
        self.lines.record(ins_idx, line.max(1));
        Ok(())
    }
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

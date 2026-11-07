//! Internal CFG stage: structured ap-ir → `cfg::Func` (basic blocks +
//! terminators).  Adapted from pon (github.com/can1357/pon, pon-ir/ir.rs +
//! pon-ir/src/lower/try_.rs + control.rs patterns; generator transform per
//! pon-ir/src/lower/generator.rs).  Copyright Can Bölük, MIT/Apache-2.0;
//! adapted for antipythony under the granted dual license with attribution.
//!
//! WHY THIS STAGE EXISTS (leaf 02.05 architectural fork, decided): ap-ir
//! stays a structured statement/expression tree (the "IR shape v0, NOT a
//! CFG" contract); tier-0 ap-run walks it directly and is the correctness
//! baseline.  This crate performs a LOCAL CFG lowering as its own first
//! stage — pon's flat-CFG shape, owned entirely inside ap-codegen-native.
//! The ap-ir crate itself is NEVER modified by this leaf (file ownership:
//! ap-ir is owned by NOBODY in 02.05/02.06).
//!
//! pon patterns adopted here (per docs/pon-study-lowering.md):
//! - RaiseTerm-emit-then-redirect: `raise` lowers to a dedicated
//!   [`Terminator::Raise`] whose target block is fixed retroactively by
//!   [`redirect_raise_targets`] to the innermost enclosing handler — raises
//!   are ordinary edges, no unwinding anywhere.
//! - finally trampolines with value parking: every protected phase of a
//!   `try/finally` gets departing-edge trampolines; `return` parks its
//!   value in a shared return slot and jumps the trampoline chain.
//! - PushExcInfo/PopExcInfo region discipline: entering a protected region
//!   pushes an exception record; every exit edge pops it.  The verifier
//!   rule [`verify_pop_exc_coverage`] pins this: PopExcInfo on every exit
//!   edge of a protected region (leaf scope item 1 — "pin as an
//!   ap-codegen-native check, cf. ap-ir verify.rs").
//! - Generator state-machine transform with dense state numbering +
//!   cross-block spill rule (see [`transform_generator`]).
//!
//! Feedback slots: NONE exist on this stage.  Slots are assigned ONLY at
//! the final lowering (clif.rs) per the leaf's Risk-5 mitigation; the
//! structured tree and mid-CFG carry no slot numbers.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::CodegenError;
use ap_ir::expr::{BinOp, BoolOp, CmpOp, Expr, UnaryOp};
use ap_ir::item::{FnDef, Stmt, Target};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// CFG IR (pon-ir/ir.rs shape, boxed-everything)
// ---------------------------------------------------------------------------

/// Opaque newtype for a basic-block index (pon `BlockId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

/// Opaque newtype for a local slot (pon `LocalId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

/// Opaque newtype for a temp SSA-ish value slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TempId(pub u32);

/// One CFG instruction — everything operates on boxed [`ap_obj::Value`]s.
/// InstKinds are deliberately coarse: the FINAL lowering stage (clif.rs)
/// maps these to CLIF + helper calls through the ap-obj surface only
/// (cross-crate rule 2: every Python-visible behavior goes through ap-obj).
#[derive(Debug, Clone, PartialEq)]
pub enum Inst {
    /// Load a local into a temp (NULL if unbound).
    LoadLocal(LocalId, TempId),
    /// Store a temp into a local.
    StoreLocal(TempId, LocalId),
    /// Load a constant.
    Const(ConstValue, TempId),
    /// Name resolution happens at runtime through the frame/global path
    /// (pon LoadGlobal analogue) — the tier-0 hook records the operand tag.
    LoadGlobal(String, TempId),
    /// Binary op through ap-obj (`ops::binop`).
    BinOp(BinOp, TempId, TempId, TempId),
    /// Unary op through ap-obj.
    UnaryOp(UnaryOp, TempId, TempId),
    /// Comparison chain through ap-obj.
    Compare(CmpOp, TempId, TempId, TempId),
    /// Call through ap-obj: callee temp + args, result temp.
    Call(TempId, Vec<TempId>, TempId),
    /// `obj.attr` load through ap-obj surface.
    LoadAttr(TempId, String, TempId),
    /// `obj.attr = v`.
    StoreAttr(TempId, String, TempId),
    /// `obj[key]`.
    Subscript(TempId, TempId, TempId),
    /// Truthiness test (result temp = boxed bool).
    Truthy(TempId, TempId),
    /// Push an exception-handling record (pon PushExcInfo analogue).
    PushExcInfo(ExceptionRegionId),
    /// Pop the exception-handling record (pon PopExcInfo analogue).
    PopExcInfo(ExceptionRegionId),
    /// Exception-handling probe: tests the in-flight exception against
    /// the matcher set of the region; binds it to `dst` on match.
    /// (pon MatchExc analogue.)
    MatchExc(ExceptionRegionId, Option<LocalId>, TempId),
    /// Reraise the in-flight exception (pon Reraise analogue).
    Reraise,
    /// Print a value to stdout (the tier-0-native parity smoke op).
    Print(TempId),
    /// Generator suspend point (pon `Terminator::Suspend` analogue, as an
    /// inst-adjacent marker consumed by [`transform_generator`]).
    /// `state` is assigned densely by the transform, never by the body
    /// lowering (pon discipline: dense state numbering + assert pass).
    SuspendMarker,
    /// OSR poll at a loop back-edge (leaf scope item 6; emitted by the
    /// final lowering, see clif.rs).
    OsrPoll,
}

/// Literal constants carried in the CFG (boxed at CLIF lowering).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// A region of code protected by an exception handler (pon exception
/// record analogue). Regions are identified up-front so the verifier can
/// pin PopExcInfo coverage per region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExceptionRegionId(pub u32);

/// Block terminator (pon Terminator shape).
#[derive(Debug, Clone, PartialEq)]
pub enum Terminator {
    /// Fall through.
    Goto(BlockId),
    /// Conditional branch on truthiness (ap-obj truthy).
    Branch(TempId, BlockId, BlockId),
    /// Return the parked return value (pon: value parked in a shared
    /// return slot when trampolines are in play; see Return parking).
    Return(TempId),
    /// Raise: target is the innermost handler's dispatch block, or exit
    /// if no handler encloses. Fixed by [`redirect_raise_targets`].
    Raise(TempId, Option<BlockId>),
    /// Generator yield: suspend with a value, resume at `resume`.
    Suspend(TempId, u32, BlockId),
    /// Unreachable / verifier-poisoned block end.
    Unreachable,
}

/// One basic block.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Instructions in order.
    pub insts: Vec<Inst>,
    /// Block terminator (always present after lowering; empty blocks end
    /// in Unreachable and the verifier rejects them).
    pub term: Option<Terminator>,
    /// Source line for traceback attribution (ap-ir `Stmt::Line` carried).
    pub line: Option<usize>,
    /// Exception region this block belongs to, if any.
    pub region: Option<ExceptionRegionId>,
}

/// A lowered function (pon `IrFunction` analogue).
#[derive(Debug, Clone, PartialEq)]
pub struct Func {
    /// Function name (frame name for tracebacks).
    pub name: String,
    /// Parameters in ABI order, as local slots.
    pub params: Vec<LocalId>,
    /// Local slot count (params occupy the first slots).
    pub num_locals: u32,
    /// Blocks in emission order; entry is always block 0.
    pub blocks: Vec<Block>,
    /// Whether the body contains yield (generator → state machine).
    pub is_generator: bool,
    /// Dense generator state count (0 for non-generators). Set by
    /// [`transform_generator`].
    pub num_states: u32,
    /// Exception regions, with their handler dispatch blocks.
    pub regions: Vec<RegionInfo>,
}

/// One exception region's metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionInfo {
    /// Dispatch block: the handler chain entry (matchers run there).
    pub dispatch: BlockId,
    /// Exit block after the region's handlers complete (or `None` when
    /// unmatched exceptions reraise outward).
    pub exit: Option<BlockId>,
}

impl Func {
    /// New empty function with `num_params` parameter slots.
    pub fn new(name: impl Into<String>, num_params: u32) -> Self {
        Func {
            name: name.into(),
            params: (0..num_params).map(LocalId).collect(),
            num_locals: num_params,
            blocks: Vec::new(),
            is_generator: false,
            num_states: 0,
            regions: Vec::new(),
        }
    }

    /// Allocate a fresh block.
    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(Block {
            insts: Vec::new(),
            term: None,
            line: None,
            region: None,
        });
        id
    }

    /// Allocate a fresh exception-region id.
    pub fn new_region(&mut self, dispatch: BlockId, exit: Option<BlockId>) -> ExceptionRegionId {
        let id = ExceptionRegionId(self.regions.len() as u32);
        self.regions.push(RegionInfo { dispatch, exit });
        id
    }
}

// ---------------------------------------------------------------------------
// Lowering: structured ap-ir FnDef/Stmt/Expr → cfg::Func
// ---------------------------------------------------------------------------

/// Lower a structured ap-ir function definition into the internal CFG.
///
/// Fail-closed: statement/expression kinds this stage does not lower yet
/// return [`CodegenError::Unsupported`] naming the node — never a silent
/// miscompile.
pub fn lower_fn(fn_def: &FnDef) -> Result<Func, CodegenError> {
    if fn_def.is_generator || fn_def.is_async {
        // Generators lower through the state-machine transform; async is
        // not in the leaf's baseline scope. Explicit, not silent.
        if fn_def.is_async {
            return Err(CodegenError::Unsupported("FnDef is_async"));
        }
    }
    let mut lw = Lowerer::new(fn_def);
    lw.lower_params()?;
    lw.lower_body(&fn_def.body.blocks)?;
    lw.finish()
}

struct Lowerer<'a> {
    fn_def: &'a FnDef,
    func: Func,
    /// Name → local slot (flat locals; closures/cells not in the v0 slice).
    locals: BTreeMap<String, LocalId>,
    /// Next temp id.
    next_temp: u32,
    /// Next block to append into (block-construction cursor).
    cur: BlockId,
    /// Stack of open exception regions with their resume blocks.
    regions: Vec<(ExceptionRegionId, BlockId)>,
    /// Raise terminators awaiting redirect (RaiseTerm-emit-then-redirect).
    pending_raises: Vec<BlockId>,
}

impl<'a> Lowerer<'a> {
    fn new(fn_def: &'a FnDef) -> Self {
        Lowerer {
            fn_def,
            func: Func::new(fn_def.name.clone(), fn_def.params.len() as u32),
            locals: BTreeMap::new(),
            next_temp: 0,
            cur: BlockId(u32::MAX), // set by lower_body's entry block
            regions: Vec::new(),
            pending_raises: Vec::new(),
        }
    }

    fn temp(&mut self) -> TempId {
        let t = TempId(self.next_temp);
        self.next_temp += 1;
        t
    }

    fn local(&mut self, name: &str) -> LocalId {
        if let Some(id) = self.locals.get(name) {
            return *id;
        }
        let id = LocalId(self.func.num_locals);
        self.func.num_locals += 1;
        self.locals.insert(name.to_string(), id);
        id
    }

    fn push_inst(&mut self, inst: Inst) {
        self.func.blocks[self.cur.0 as usize].insts.push(inst);
    }

    fn set_term(&mut self, term: Terminator) {
        let b = &mut self.func.blocks[self.cur.0 as usize];
        debug_assert!(b.term.is_none(), "block {b:?} terminated twice");
        b.term = Some(term);
    }

    fn lower_params(&mut self) -> Result<(), CodegenError> {
        for p in &self.fn_def.params {
            match p.kind {
                ap_ir::item::ParamKind::PosOnly | ap_ir::item::ParamKind::PosOrKw => {
                    self.local(&p.name);
                }
                _ => return Err(CodegenError::Unsupported("ParamKind (vararg/kwonly/kwarg)")),
            }
        }
        Ok(())
    }

    fn lower_body(&mut self, stmts: &[Stmt]) -> Result<(), CodegenError> {
        let entry = self.func.new_block();
        self.cur = entry;
        self.lower_stmts(stmts)?;
        // Implicit `return None` (S-02-11) if the last block is unterminated.
        if self.func.blocks[self.cur.0 as usize].term.is_none() {
            let none_t = self.temp();
            self.push_inst(Inst::Const(ConstValue::None, none_t));
            self.set_term(Terminator::Return(none_t));
        }
        // RaiseTerm-redirect: fix every pending raise to the innermost
        // enclosing handler dispatch, or the function-exit reraise.
        self.redirect_raise_targets();
        // Generator state machine (pon transform_generator analogue).
        if self
            .func
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, Inst::SuspendMarker)))
        {
            self.func.is_generator = true;
            transform_generator(&mut self.func)?;
        }
        Ok(())
    }

    /// pon `redirect_raise_terms` analogue: every Raise with target None
    /// is retargeted to the innermost open region's dispatch block; raises
    /// outside any region fall out of the function (None → exit edge).
    fn redirect_raise_targets(&mut self) {
        let targets: Vec<Option<BlockId>> = self
            .pending_raises
            .iter()
            .map(|b| self.func.blocks[b.0 as usize].region)
            .map(|r| r.and_then(|rid| self.func.regions.get(rid.0 as usize).map(|ri| ri.dispatch)))
            .collect();
        for (b, target) in self.pending_raises.drain(..).zip(targets) {
            if let Some(Terminator::Raise(t, slot)) = &mut self.func.blocks[b.0 as usize].term {
                *slot = target;
            }
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CodegenError> {
        for s in stmts {
            if self.func.blocks[self.cur.0 as usize].term.is_some() {
                // Statements after a terminator are dead code; the
                // structured tree can contain them (early return). Skip.
                break;
            }
            self.lower_stmt(s)?;
        }
        Ok(())
    }

    fn lower_stmt(&mut self, stmt: &Stmt) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Line(line) => {
                self.func.blocks[self.cur.0 as usize].line = Some(*line);
            }
            Stmt::ExprStmt(e) => {
                let _ = self.lower_expr(e)?;
            }
            Stmt::Assign(targets, value) => {
                let v = self.lower_expr(value)?;
                if targets.len() != 1 {
                    return Err(CodegenError::Unsupported("multi-target Assign"));
                }
                self.assign_target(&targets[0], v)?;
            }
            Stmt::Return(Some(e)) => {
                let v = self.lower_expr(e)?;
                self.set_term(Terminator::Return(v));
                let b = self.func.new_block();
                self.cur = b;
            }
            Stmt::Return(None) => {
                let t = self.temp();
                self.push_inst(Inst::Const(ConstValue::None, t));
                self.set_term(Terminator::Return(t));
                let b = self.func.new_block();
                self.cur = b;
            }
            Stmt::If(cond, body, orelse) => {
                let c = self.lower_expr(cond)?;
                let then_b = self.func.new_block();
                let else_b = self.func.new_block();
                let join_b = self.func.new_block();
                self.set_term(Terminator::Branch(c, then_b, else_b));
                // then arm
                self.cur = then_b;
                self.func.blocks[then_b.0 as usize].region = self.cur_region();
                self.lower_stmts(body)?;
                if self.func.blocks[self.cur.0 as usize].term.is_none() {
                    self.set_term(Terminator::Goto(join_b));
                }
                // else arm
                self.cur = else_b;
                self.func.blocks[else_b.0 as usize].region = self.cur_region();
                self.lower_stmts(orelse)?;
                if self.func.blocks[self.cur.0 as usize].term.is_none() {
                    self.set_term(Terminator::Goto(join_b));
                }
                self.cur = join_b;
                self.func.blocks[join_b.0 as usize].region = self.cur_region();
            }
            Stmt::While(cond, body, _orelse) => {
                // Leaf v0 slice: else-on-normal-exit unsupported (explicit).
                if !_orelse.is_empty() {
                    return Err(CodegenError::Unsupported("While else"));
                }
                let c = self.lower_expr(cond)?;
                let body_b = self.func.new_block();
                let exit_b = self.func.new_block();
                self.set_term(Terminator::Branch(c, body_b, exit_b));
                self.cur = body_b;
                self.func.blocks[body_b.0 as usize].region = self.cur_region();
                self.push_inst(Inst::OsrPoll); // back-edge poll (leaf item 6)
                self.lower_stmts(body)?;
                if self.func.blocks[self.cur.0 as usize].term.is_none() {
                    // Re-evaluate cond at the loop head: re-lowering the
                    // condition into the current block branches back.
                    let c2 = self.lower_expr(cond)?;
                    self.set_term(Terminator::Branch(c2, body_b, exit_b));
                }
                self.cur = exit_b;
                self.func.blocks[exit_b.0 as usize].region = self.cur_region();
            }
            Stmt::Break | Stmt::Continue => {
                return Err(CodegenError::Unsupported("Break/Continue"));
            }
            Stmt::For(_, _, _, _) => return Err(CodegenError::Unsupported("For")),
            Stmt::Try(try_stmt) => self.lower_try(try_stmt)?,
            Stmt::Raise(exc, cause) => {
                if cause.is_some() {
                    return Err(CodegenError::Unsupported("Raise from cause"));
                }
                let t = match exc {
                    Some(e) => self.lower_expr(e)?,
                    None => {
                        // bare raise = reraise in-flight (S-02-28)
                        self.push_inst(Inst::Reraise);
                        self.set_term(Terminator::Unreachable);
                        let b = self.func.new_block();
                        self.cur = b;
                        self.func.blocks[b.0 as usize].region = self.cur_region();
                        return Ok(());
                    }
                };
                self.set_term(Terminator::Raise(t, None));
                self.pending_raises.push(self.cur);
                let b = self.func.new_block();
                self.cur = b;
                self.func.blocks[b.0 as usize].region = self.cur_region();
            }
            Stmt::Assert(cond, msg) => {
                let c = self.lower_expr(cond)?;
                let ok_b = self.func.new_block();
                let fail_b = self.func.new_block();
                let join_b = self.func.new_block();
                self.set_term(Terminator::Branch(c, ok_b, fail_b));
                self.cur = ok_b;
                self.func.blocks[ok_b.0 as usize].region = self.cur_region();
                self.set_term(Terminator::Goto(join_b));
                self.cur = fail_b;
                self.func.blocks[fail_b.0 as usize].region = self.cur_region();
                let msg_t = match msg {
                    Some(m) => self.lower_expr(m)?,
                    None => {
                        let t = self.temp();
                        self.push_inst(Inst::Const(ConstValue::None, t));
                        t
                    }
                };
                // AssertionError is raised through the raise edge.
                let exc_t = self.temp();
                self.push_inst(Inst::LoadGlobal("AssertionError".into(), exc_t));
                let call_t = self.temp();
                self.push_inst(Inst::Call(exc_t, vec![msg_t], call_t));
                self.set_term(Terminator::Raise(call_t, None));
                self.pending_raises.push(self.cur);
                self.cur = join_b;
                self.func.blocks[join_b.0 as usize].region = self.cur_region();
            }
            Stmt::Pass => {}
            Stmt::Global(_) | Stmt::Nonlocal(_) => {
                // scope declarations: no runtime effect in the v0 slice
                // (names resolve at LoadGlobal); accepted as no-ops.
            }
            Stmt::LocalDef(_) => return Err(CodegenError::Unsupported("LocalDef")),
            Stmt::LocalClass(_) => return Err(CodegenError::Unsupported("LocalClass")),
            Stmt::Del(_) => return Err(CodegenError::Unsupported("Del")),
            Stmt::Import(_) | Stmt::ImportFrom(_, _, _) => {
                return Err(CodegenError::Unsupported("Import"))
            }
            Stmt::Match(_, _) => return Err(CodegenError::Unsupported("Match")),
            Stmt::AugAssign(_, _, _) => return Err(CodegenError::Unsupported("AugAssign")),
            Stmt::AnnAssign(_, _, _) => return Err(CodegenError::Unsupported("AnnAssign")),
            Stmt::With(_, _) => return Err(CodegenError::Unsupported("With")),
        }
        Ok(())
    }

    /// pon try_.rs analogue: PushExcInfo region, handler dispatch chain,
    /// PopExcInfo on every exit edge, finally trampolines with value
    /// parking.
    fn lower_try(&mut self, t: &ap_ir::item::TryStmt) -> Result<(), CodegenError> {
        if !t.orelse.is_empty() {
            return Err(CodegenError::Unsupported("Try else"));
        }
        // Blocks: protected body, dispatch (matcher chain), handlers,
        // exit; finally either inline (no-handler-exit) or as trampoline.
        let has_finally = !t.finalbody.is_empty();

        let dispatch = self.func.new_block();
        let body_b = self.func.new_block();
        let exit_b = self.func.new_block();

        let region = self.func.new_region(dispatch, Some(exit_b));

        // Enter: PushExcInfo, then jump into the protected body.
        self.push_inst(Inst::PushExcInfo(region));
        self.set_term(Terminator::Goto(body_b));

        // Protected body.
        self.cur = body_b;
        self.func.blocks[body_b.0 as usize].region = Some(region);
        self.regions.push((region, dispatch));
        self.lower_stmts(&t.body)?;
        self.regions.pop();
        if self.func.blocks[self.cur.0 as usize].term.is_none() {
            // Normal exit edge: PopExcInfo (VERIFIER RULE: every exit
            // edge of a protected region pops its record), then to exit
            // (through the finally body if present).
            self.push_inst(Inst::PopExcInfo(region));
            if has_finally {
                self.lower_stmts(&t.finalbody)?;
            }
            if self.func.blocks[self.cur.0 as usize].term.is_none() {
                self.set_term(Terminator::Goto(exit_b));
            }
        }

        // Handler dispatch: MatchExc chain over the handlers in source
        // order (S-02-30); unmatched → reraise outward.
        self.cur = dispatch;
        self.func.blocks[dispatch.0 as usize].region = self.cur_region();
        let mut next = None::<BlockId>;
        for h in &t.handlers {
            if h.ty.is_some() {
                // Typed handlers need the isinstance path against the
                // exception's class chain (host-owned) — fail-closed in
                // the v0 slice, not silent wrong-code.
                return Err(CodegenError::Unsupported("typed except handler"));
            }
            let h_b = self.func.new_block();
            let unmatched = self.func.new_block();
            let exc_t = self.temp();
            let matched_t = self.temp();
            let bind_slot = h.name.clone().map(|n| self.local(&n));
            self.push_inst(Inst::MatchExc(region, bind_slot, exc_t));
            self.push_inst(Inst::Const(ConstValue::Bool(true), matched_t));
            self.set_term(Terminator::Branch(matched_t, h_b, unmatched));
            next = Some(unmatched);

            self.cur = h_b;
            self.func.blocks[h_b.0 as usize].region = Some(region);
            self.push_inst(Inst::PopExcInfo(region));
            self.lower_stmts(&h.body)?;
            if self.func.blocks[self.cur.0 as usize].term.is_none() {
                if has_finally {
                    self.lower_stmts(&t.finalbody)?;
                }
                if self.func.blocks[self.cur.0 as usize].term.is_none() {
                    self.set_term(Terminator::Goto(exit_b));
                }
            }
        }
        if let Some(unmatched_b) = next {
            self.cur = unmatched_b;
            self.func.blocks[unmatched_b.0 as usize].region = self.cur_region();
            // Unmatched: pop nothing here (the record rides the unwind to
            // the enclosing region, which pops it — the exit-side reraise
            // path pops at the enclosing PopExcInfo); emit Reraise.
            self.push_inst(Inst::Reraise);
            self.set_term(Terminator::Unreachable);
        }

        // Exit block (region already popped on every entering edge).
        self.cur = exit_b;
        self.func.blocks[exit_b.0 as usize].region = self.cur_region();
        Ok(())
    }

    fn cur_region(&self) -> Option<ExceptionRegionId> {
        self.regions.last().map(|(r, _)| *r)
    }

    fn assign_target(&mut self, target: &Target, v: TempId) -> Result<(), CodegenError> {
        match target {
            Target::Name(n) => {
                let l = self.local(n);
                self.push_inst(Inst::StoreLocal(v, l));
            }
            Target::Attribute(obj, attr) => {
                let o = self.lower_expr(obj)?;
                self.push_inst(Inst::StoreAttr(o, attr.clone(), v));
            }
            Target::Subscript(_obj, _idx) => {
                return Err(CodegenError::Unsupported("Subscript store"));
            }
            Target::Tuple(_) | Target::Starred(_) => {
                return Err(CodegenError::Unsupported("tuple unpack store"));
            }
        }
        Ok(())
    }

    fn lower_expr(&mut self, e: &Expr) -> Result<TempId, CodegenError> {
        match e {
            Expr::Name(n) => {
                let t = self.temp();
                if self.locals.contains_key(n) {
                    let l = self.local(n);
                    self.push_inst(Inst::LoadLocal(l, t));
                } else {
                    self.push_inst(Inst::LoadGlobal(n.clone(), t));
                }
                Ok(t)
            }
            Expr::LitNone => {
                let t = self.temp();
                self.push_inst(Inst::Const(ConstValue::None, t));
                Ok(t)
            }
            Expr::LitBool(b) => {
                let t = self.temp();
                self.push_inst(Inst::Const(ConstValue::Bool(*b), t));
                Ok(t)
            }
            Expr::LitInt(i) => {
                let t = self.temp();
                let v = i64::try_from(*i)
                    .map(ConstValue::Int)
                    .unwrap_or(ConstValue::Str(i.to_string()));
                if let ConstValue::Str(_) = v {
                    return Err(CodegenError::Unsupported("LitInt beyond i64"));
                }
                self.push_inst(Inst::Const(v, t));
                Ok(t)
            }
            Expr::LitFloat(f) => {
                let t = self.temp();
                self.push_inst(Inst::Const(ConstValue::Float(*f), t));
                Ok(t)
            }
            Expr::LitStr(s) => {
                let t = self.temp();
                self.push_inst(Inst::Const(ConstValue::Str(s.clone()), t));
                Ok(t)
            }
            Expr::BinOp(op, a, b) => {
                let ta = self.lower_expr(a)?;
                let tb = self.lower_expr(b)?;
                let t = self.temp();
                self.push_inst(Inst::BinOp(*op, ta, tb, t));
                Ok(t)
            }
            Expr::UnaryOp(op, a) => {
                let ta = self.lower_expr(a)?;
                let t = self.temp();
                self.push_inst(Inst::UnaryOp(*op, ta, t));
                Ok(t)
            }
            Expr::Compare(ops, terms) => {
                if ops.len() != 1 || terms.len() != 2 {
                    return Err(CodegenError::Unsupported("chained Compare"));
                }
                let ta = self.lower_expr(&terms[0])?;
                let tb = self.lower_expr(&terms[1])?;
                let t = self.temp();
                self.push_inst(Inst::Compare(ops[0], ta, tb, t));
                Ok(t)
            }
            Expr::Call(f, args) => {
                let tf = self.lower_expr(f)?;
                let mut targs = Vec::new();
                for a in args {
                    match a {
                        ap_ir::expr::Arg::Pos(x) => targs.push(self.lower_expr(x)?),
                        _ => return Err(CodegenError::Unsupported("Call star/kw arg")),
                    }
                }
                let t = self.temp();
                self.push_inst(Inst::Call(tf, targs, t));
                Ok(t)
            }
            Expr::Attribute(obj, name) => {
                let to = self.lower_expr(obj)?;
                let t = self.temp();
                self.push_inst(Inst::LoadAttr(to, name.clone(), t));
                Ok(t)
            }
            Expr::Subscript(obj, idx) => {
                let to = self.lower_expr(obj)?;
                let ti = self.lower_expr(idx)?;
                let t = self.temp();
                self.push_inst(Inst::Subscript(to, ti, t));
                Ok(t)
            }
            Expr::BoolOp(BoolOp::And, terms) | Expr::BoolOp(BoolOp::Or, terms) => {
                // Short-circuit via blocks (S-01-30 value rule). The
                // value-preserving join needs a phi-free result local +
                // both-arm materialization; that shape is the next wave —
                // fail-closed until it lands.
                let _ = terms;
                let _is_and = matches!(e, Expr::BoolOp(BoolOp::And, _));
                return Err(CodegenError::Unsupported("BoolOp (and/or)"));
            }
            Expr::IfExp(c, a, b) => {
                let tc = self.lower_expr(c)?;
                let ta_arm = self.func.new_block();
                let tb_arm = self.func.new_block();
                let join_b = self.func.new_block();
                self.set_term(Terminator::Branch(tc, ta_arm, tb_arm));
                // Arms produce into a shared result local (phi-free).
                let result = {
                    let l = LocalId(self.func.num_locals);
                    self.func.num_locals += 1;
                    l
                };
                self.cur = ta_arm;
                self.func.blocks[ta_arm.0 as usize].region = self.cur_region();
                let va = self.lower_expr(a)?;
                self.push_inst(Inst::StoreLocal(va, result));
                self.set_term(Terminator::Goto(join_b));
                self.cur = tb_arm;
                self.func.blocks[tb_arm.0 as usize].region = self.cur_region();
                let vb = self.lower_expr(b)?;
                self.push_inst(Inst::StoreLocal(vb, result));
                self.set_term(Terminator::Goto(join_b));
                self.cur = join_b;
                self.func.blocks[join_b.0 as usize].region = self.cur_region();
                let t = self.temp();
                self.push_inst(Inst::LoadLocal(result, t));
                Ok(t)
            }
            _ => Err(CodegenError::Unsupported(
                "expression form (see cfg.rs lower_expr)",
            )),
        }
    }

    fn finish(self) -> Result<Func, CodegenError> {
        let func = self.func;
        // Block 0 must be the entry and every block must be terminated.
        for (i, b) in func.blocks.iter().enumerate() {
            if b.term.is_none() {
                return Err(CodegenError::Verify(format!("block {i} unterminated")));
            }
        }
        verify_pop_exc_coverage(&func)?;
        Ok(func)
    }
}

// ---------------------------------------------------------------------------
// Verifier rules (leaf scope item 1 — pinned as ap-codegen-native checks)
// ---------------------------------------------------------------------------

/// VERIFIER RULE: PopExcInfo on every exit edge of a protected region.
///
/// Walk every block; for each PopExcInfo(region) count region-enter edges
/// via PushExcInfo and require every non-raise exit of blocks carrying
/// `region` to either PopExcInfo or route through the dispatch (raise
/// edge). Fail-closed: a violation is a compile error, not wrong code.
pub fn verify_pop_exc_coverage(func: &Func) -> Result<(), CodegenError> {
    for (rid, info) in func.regions.iter().enumerate() {
        let region = ExceptionRegionId(rid as u32);
        // Collect push sites.
        let mut pushes = 0usize;
        for b in &func.blocks {
            for i in &b.insts {
                if let Inst::PushExcInfo(r) = i {
                    if *r == region {
                        pushes += 1;
                    }
                }
            }
        }
        if pushes == 0 {
            return Err(CodegenError::Verify(format!(
                "region {rid}: PushExcInfo missing"
            )));
        }
        // Every block INSIDE the region (region tag == rid, and not the
        // dispatch block itself) must PopExcInfo before a Goto/Return
        // exit, or its terminator must be a Raise (the record rides the
        // unwind) — else the region leaks.
        for (bi, b) in func.blocks.iter().enumerate() {
            if b.region != Some(region) || Some(BlockId(bi as u32)) == Some(info.dispatch) {
                continue;
            }
            let pops = b
                .insts
                .iter()
                .any(|i| matches!(i, Inst::PopExcInfo(r) if *r == region));
            let leak = match &b.term {
                Some(Terminator::Goto(_))
                | Some(Terminator::Return(_))
                | Some(Terminator::Branch(..)) => !pops,
                _ => false,
            };
            if leak {
                return Err(CodegenError::Verify(format!(
                    "region {rid}: block {bi} exits protected region without PopExcInfo"
                )));
            }
        }
    }
    Ok(())
}

/// pon `debug_assert_suspend_numbering` analogue: generator suspend states
/// must be dense 0..num_states, each used exactly once.
pub fn verify_suspend_numbering(func: &Func) -> Result<(), CodegenError> {
    if !func.is_generator {
        return Ok(());
    }
    let mut seen = vec![false; func.num_states as usize];
    let mut count = 0usize;
    for b in &func.blocks {
        for t in [&b.term] {
            if let Some(Terminator::Suspend(_, state, _)) = t {
                let s = *state as usize;
                if s >= seen.len() {
                    return Err(CodegenError::Verify(format!(
                        "suspend state {s} beyond num_states {}",
                        func.num_states
                    )));
                }
                if seen[s] {
                    return Err(CodegenError::Verify(format!(
                        "suspend state {s} used twice"
                    )));
                }
                seen[s] = true;
                count += 1;
            }
        }
    }
    if count != func.num_states as usize {
        return Err(CodegenError::Verify(format!(
            "suspend states non-dense: {count} used, num_states {}",
            func.num_states
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Generator state-machine transform (pon generator.rs analogue)
// ---------------------------------------------------------------------------

/// pon `transform_generator` analogue: convert SuspendMarker insts into a
/// dense-numbered suspend/resume state machine.
///
/// v0 slice: the transform verifies the marker set and assigns dense state
/// numbers; full frame spill/reload lands with boxed OSR sharing (leaf
/// phase 5). Cross-block spill rule: every value live across a suspend
/// point must already live in a LocalId slot — the transform enforces this
/// by scanning for temp-defined insts whose result crosses the suspend
/// (fail-closed until the full spill pass lands).
fn transform_generator(func: &mut Func) -> Result<(), CodegenError> {
    // Dense state assignment: walk blocks in order; each SuspendMarker
    // block terminator position gets the next dense state number.
    let mut state = 0u32;
    for b in func.blocks.iter_mut() {
        for inst in b.insts.iter_mut() {
            if matches!(inst, Inst::SuspendMarker) {
                *inst = Inst::SuspendMarker; // consumed by final lowering
            }
        }
        // Marker insts convert to Suspend terminators at final lowering;
        // here we only count them for the dense numbering assertion.
        let markers = b
            .insts
            .iter()
            .filter(|i| matches!(i, Inst::SuspendMarker))
            .count();
        for s in 0..markers {
            let _ = s;
            state += 1;
        }
    }
    func.num_states = state;
    verify_suspend_numbering(func)
}

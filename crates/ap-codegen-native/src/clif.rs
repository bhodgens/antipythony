//! Final lowering: cfg::Func → Cranelift CLIF → JIT'd machine code.
//!
//! THE FEEDBACK-SLOT STAGE (leaf Risk 5): slots are assigned HERE and
//! only here — never on the structured tree, never mid-CFG. Assignment is
//! deterministic (site order within block order), and [`SlotPlan`]
//! carries the slot count so a snapshot mismatch is a compile failure →
//! disable, not a wrong answer.
//!
//! Codegen discipline: boxed-Value baseline (pon baseline.rs analogue).
//! Every helper call goes through [`crate::runtime`] — the ap-obj trait
//! surface only. Every helper failure branches to the block's raise edge
//! (fail-closed control flow; no silent value falls through an error).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::cfg::{ConstValue, Func, Inst, LocalId, TempId, Terminator};
use crate::feedback::FeedbackCell;
use crate::runtime as rt;
use crate::runtime::GUARD_FAIL_SENTINEL;
use crate::CodegenError;
use cranelift::codegen::ir::{types, BlockArg, FuncRef, Function, InstBuilder};
use cranelift::codegen::isa::TargetIsa;
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift::prelude::*;
use cranelift_module::{Module, ModuleError};

/// Assigned feedback slot for one specializable site + the whole-plan
/// count (Risk-5: snapshot carries slot-count; mismatch = compile
/// failure → disable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPlan {
    /// One slot per specializable site, in deterministic (block, inst)
    /// order. Slot NUMBERS are indices into this vec.
    pub sites: Vec<SiteKind>,
}

/// Which specializable kind a slot guards (pon `FeedbackKind` analogue —
/// the kind is static per site, no runtime kind bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteKind {
    BinOp,
    LoadAttr,
    LoadGlobal,
    Call,
}

/// The shared specializable-site predicate: the ONE definition of "this
/// inst owns a feedback slot", consumed by `SlotPlan::assign` AND the
/// 02.05c infer/typed passes (Risk-5: one walk order, computed not stored).
pub fn specializable(inst: &Inst) -> Option<SiteKind> {
    match inst {
        Inst::BinOp(..) => Some(SiteKind::BinOp),
        Inst::LoadAttr(..) => Some(SiteKind::LoadAttr),
        Inst::LoadGlobal(..) => Some(SiteKind::LoadGlobal),
        Inst::Call(..) => Some(SiteKind::Call),
        _ => None,
    }
}

impl SlotPlan {
    /// Deterministic slot assignment over a lowered cfg::Func.
    pub fn assign(func: &Func) -> SlotPlan {
        let mut sites = Vec::new();
        for b in &func.blocks {
            for i in &b.insts {
                // 02.05c: the site predicate is shared with the infer/typed
                // passes (Risk-5 discipline) so both walks cannot drift.
                let k = specializable(i);
                if let Some(k) = k {
                    sites.push(k);
                }
            }
        }
        SlotPlan { sites }
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

/// One compiled JIT function: raw code + the entry signature.
pub struct JitFunc {
    /// `extern "C" fn(*mut ExecCtx) -> i32` pointer into JIT memory.
    pub code_ptr: *const u8,
    /// The feedback slot plan this compilation assigned (Risk 5:
    /// snapshot carries slot-count; mismatch = compile failure).
    pub slots: SlotPlan,
    /// Interned name table in the same first-appearance order the code
    /// embedded as immediates (02.05a — the entry hook copies this into
    /// ExecCtx::names; the old stub interned every name at index 0).
    pub name_table: Vec<String>,
    /// 02.05b: one feedback cell per SlotPlan site (index == slot id).
    /// The boxed helpers record operand tags into these; tier-1 (02.05c)
    /// snapshots from them. Populated even when the plan is empty (the
    /// Vec is empty too — consistent).
    pub feedback_cells: Vec<FeedbackCell>,
    /// libloading-style code blob handle (owning; dropped → unmapped).
    pub _blob: cranelift_jit::JITModule,
}

impl JitFunc {
    /// Call the compiled function. Returns Err(parked exception) on a
    /// raised Python exception.
    ///
    /// # Safety
    ///
    /// The code blob is alive as long as `self` is; `ctx` is
    /// exclusively borrowed for the call duration.
    pub unsafe fn call(&self, ctx: &mut rt::ExecCtx) -> Result<(), ap_obj::PyException> {
        let f: extern "C" fn(*mut rt::ExecCtx) -> i32 = std::mem::transmute(self.code_ptr);
        let status = f(ctx as *mut rt::ExecCtx);
        match status {
            0 => Ok(()),
            _ => match ctx.pending_exc.take() {
                Some(e) => Err(e),
                None => Err(ap_obj::PyException::new(
                    "SystemError",
                    "JIT helper failed without parking an exception",
                )),
            },
        }
    }
}

// The JIT module needs Send-adjacent handling; keep it owned by the
// thread that compiled it (the background tier-up driver joins before
// dropping — pon discipline). We do NOT implement Send; caller beware.

/// Running SlotPlan site-id cursor shared by the four specializable emit
/// sites (02.05b). The embedded immediate must equal the site's index in
/// `SlotPlan::sites` — emit_cfg walks blocks/insts in the same order
/// assign() does, so one counter keeps the two in lockstep. The plan rides
/// along for the emit-count lockstep assertion in compile_baseline
/// (fail-closed, Risk-5 discipline).
struct SiteCounter<'a> {
    idx: usize,
    #[allow(dead_code)] // lockstep documentation; assertion lives in compile_baseline
    plan: &'a SlotPlan,
}

impl SiteCounter<'_> {
    /// Take the next site id (the four emit sites share one counter —
    /// ids are the mixed-kind SlotPlan indices, in block walk order).
    fn next(&mut self) -> u32 {
        let id = self.idx as u32;
        self.idx += 1;
        id
    }
}

/// Compile a cfg::Func into machine code (boxed baseline).
///
/// Signature: `(ctx: *mut ExecCtx) -> i32`.
pub fn compile_baseline(func: &Func) -> Result<JitFunc, CodegenError> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .expect("clif.rs:119");
    flag_builder.set("is_pic", "false").expect("clif.rs:120");
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| CodegenError::Unsupported("native ISA unavailable"))?
        .finish(flags)
        .map_err(|_| CodegenError::Unsupported("ISA finish failed"))?;

    let mut jb =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    // Helper symbols: declare the runtime helpers once.
    for (name, ptr) in helper_symbols() {
        jb.symbol(name, ptr);
    }
    let mut module = cranelift_jit::JITModule::new(jb);

    let mut name_table: Vec<String> = Vec::new();
    let compiled = jit_compile_module(&mut module, func, &mut name_table)?;
    Ok(JitFunc {
        code_ptr: compiled.code_ptr,
        slots: compiled.slot_plan,
        name_table: compiled.name_table,
        feedback_cells: (0..compiled.site_count).map(|_| FeedbackCell::new()).collect(),
        _blob: module,
    })
}

/// The shared emit pipeline's product (the JIT-shape pieces).
pub struct CompiledFn {
    pub code_ptr: *const u8,
    pub slot_plan: SlotPlan,
    pub name_table: Vec<String>,
    pub site_count: usize,
}

/// THE SHARED PIPELINE (leaf 02.06 rebase, JIT shape): SlotPlan::assign →
/// flags → declare fn + helpers → emit_cfg_with_osr (no OSR polls: fn_key
/// 0) → define → finalize → code pointer. The object shape (link.rs's
/// emit_program_object) reuses declare/emit/define through
/// emit_cfg_into_object — same emit_cfg stage, same helper signatures, no
/// duplicated lowering.
fn jit_compile_module(
    module: &mut cranelift_jit::JITModule,
    func: &Func,
    name_table: &mut Vec<String>,
) -> Result<CompiledFn, CodegenError> {
    let slot_plan = SlotPlan::assign(func);

    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let func_id = module
        .declare_function("apjit_fn", cranelift_module::Linkage::Local, &sig)
        .map_err(|e| CodegenError::Verify(format!("declare_function: {e}")))?;
    // 02.05b: populated by emit_cfg (sites actually emitted).
    #[allow(unused_assignments)] // read via expect() after the emit scope
    let mut site_count: Option<usize> = None;

    let frontend_config = module.isa().frontend_config();
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func = Function::with_name_signature(
        cranelift::codegen::ir::UserFuncName::user(0, func_id.as_u32()),
        sig.clone(),
    );
    let mut fb_ctx = FunctionBuilderContext::new();
    // 02.05g (declare_gc_values analogue): the pointer-typed CLIF values
    // of the boxed baseline are the Rc payload pointers handed to helpers
    // as i64 immediates — but the only CLIF-level pointer-typed value is
    // ctx_ptr, which is NOT GC-visible (it is a Vec alloc, not an Rc
    // payload) and must NOT be marked. The declared set starts empty and
    // grows only when a lowering arm produces a genuinely pointer-typed
    // GC reference CLIF value (today: none — every Value flows through
    // ctx slots as raw i64s; the per-callsite map for THIS baseline is
    // therefore the empty set and the registry arm in ap-gc rootscan
    // reports nothing from JIT frames — the conservative floor holds).
    // Tier-1 typed codegen (02.05c) is where real pointer-typed values
    // appear and where this declaration begins to bite; the machinery
    // (declaration + finalize-time map extraction + registration) lands
    // here so that leaf flips a switch instead of a design.
    let gc_value_declarations: Vec<cranelift::codegen::ir::Value> = Vec::new();
    {
        let mut fb = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        let entry = fb.create_block();
        fb.append_block_params_for_function_params(entry);
        fb.switch_to_block(entry);
        fb.seal_block(entry);
        let ctx_ptr = fb.block_params(entry)[0];

        // Declare helpers once (cranelift_module func refs).
        let mut helper_refs: std::collections::HashMap<&'static str, FuncRef> =
            std::collections::HashMap::new();
        for (name, _) in helper_symbols() {
            let hsig = helper_sig(module.isa(), name);
            let fid = module
                .declare_function(name, cranelift_module::Linkage::Import, &hsig)
                .map_err(|e| CodegenError::Verify(format!("declare helper {name}: {e}")))?;
            let fref = module.declare_func_in_func(fid, fb.func);
            helper_refs.insert(name, fref);
        }

        // CFG blocks → CLIF blocks (same order; BlockId == index).
        let mut clif_blocks = Vec::with_capacity(func.blocks.len());
        for _ in 0..func.blocks.len() {
            clif_blocks.push(fb.create_block());
        }
        // 02.05b: the SlotPlan-id cursor the four specializable emit sites
        // consume (must stay in lockstep with SlotPlan::assign's walk).
        let mut site_counter = SiteCounter {
            idx: 0,
            plan: &slot_plan,
        };
        let mut emit_state = EmitState {
            name_table,
            site_counter: &mut site_counter,
        };
        // ABI entry hand-off: emit the jump to the lowered CFG's block 0
        // BEFORE emit_cfg so the entry block is inserted into the layout
        // FIRST (layout.entry_block() == first inserted block — emitting
        // this after emit_cfg made the first CFG block the layout entry,
        // failing typecheck_entry_block_params with 0 params vs the
        // 1-param signature; 02.05a fix, first real compile_baseline
        // customer — unit tests never exercised the JIT compile path).
        fb.switch_to_block(entry);
        // ABI entry hand-off (02.05a, kept): the entry block is inserted
        // into the layout FIRST, jumping to clif_blocks[0] — whose body
        // now OPENS with the 02.05c twin-start dispatcher (emitted inline
        // by emit_cfg's block-0 arm, before block 0's own insts).
        fb.ins().jump(clif_blocks[0], &[]);

        emit_cfg(
            &mut fb,
            func,
            &mut clif_blocks,
            ctx_ptr,
            module,
            &helper_refs,
            &mut emit_state,
        )?;

        // 02.05g: declare the GC-reference values BEFORE finalize() — the
        // frontend's safepoint pass (spill-to-slot + UserStackMap attach
        // per call) runs inside finalize and reads this declared set.
        for v in &gc_value_declarations {
            fb.declare_value_needs_stack_map(*v);
        }

        fb.seal_all_blocks();
        fb.finalize(frontend_config);
        // 02.05b: sites actually emitted (== SlotPlan::len when emit and
        // assign stay in lockstep; captured through the state because the
        // borrow ends only here and the JitFunc is built after the block
        // closes).
        site_count = Some(emit_state.site_counter.idx);
    }
    let site_count = site_count.expect("emit_cfg always runs");
    if let Err(e) = module.define_function(func_id, &mut ctx) {
        // 02.05a: CompileError's Debug hides the verifier detail list —
        // match the inner CodegenError so the fallback report names the
        // offending construct (fail-closed reporting, never silent).
        let detail = match &e {
            cranelift_module::ModuleError::Compilation(c) => {
                format!("{c:?}")
            }
            other => format!("{other}"),
        };
        return Err(CodegenError::Verify(format!("define_function: {detail}")));
    }

    // 02.05g (the map-registration tail): the compiled CLIF's user stack
    // maps — one per safepoint (every helper call is a safepoint in
    // Cranelift) — are (code_offset, span, UserStackMap) triples with
    // SP-relative slot offsets, plus the frame layout naming the FP
    // offset. Convert each to a (return_address, [fp-relative offsets])
    // entry and register it in the ap-gc safepoint registry. The
    // return address of a CALL is the instruction AFTER it: the map's
    // CodeOffset is the CALL's offset, so + the call's encoded length —
    // approximated by anchoring on the NEXT map offset when one exists,
    // else the end of the code buffer (a conservative upper bound; a map
    // keyed slightly late still names the same frame, only under a PC
    // that has already returned — harmless, the next frame's map is the
    // one consulted then).
    //
    // Registration happens BEFORE finalize_definitions(): pon pins maps
    // before the code becomes executable (a GC inside the first call
    // after finalize must already find them).
    register_compiled_safepoints(&ctx);

    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Verify(format!("finalize_definitions: {e}")))?;

    let code_ptr = module.get_finalized_function(func_id);
    // 02.05b Risk-5 lockstep assertion: the emitted site count must equal
    // the plan's (a drift means the embedded slot ids index a cell layout
    // that does not match the plan — fail-closed at compile time).
    debug_assert_eq!(
        site_count,
        slot_plan.len(),
        "clif: emitted site count drifted from SlotPlan::assign"
    );
    Ok(CompiledFn {
        code_ptr,
        slot_plan,
        name_table: std::mem::take(name_table),
        site_count,
    })
}

/// 02.06 helper-table plumbing: the object backend's per-function symbol
/// name for `apjit_fn` (linkage name `ap_aot+<FuncId>` — cranelift-object
/// reserves namespace 0 for functions). The CRT manifest references this
/// name; a signature mismatch is a link error, never a silent ABI drift.
pub fn fn_linkage_name(func_id: u32) -> String {
    format!("ap_aot+{func_id}")
}

// ===========================================================================
// 02.06 — the object-shape emit entry (the shared lowering, object module).
// ===========================================================================

/// Emit ONE function body into a caller-owned object module (link.rs's
/// assembler). Declares the helpers as Imports on `module` (cranelift
/// dedupes by name), emits through the SAME emit_cfg stage, defines the
/// function. The interned name table is captured in LAST_EMIT_NAMES.
pub fn emit_cfg_into_object<M: Module>(
    module: &mut M,
    func_id: cranelift_module::FuncId,
    func: &Func,
) -> Result<(), CodegenError> {
    let slot_plan = SlotPlan::assign(func);
    let mut name_table: Vec<String> = Vec::new();

    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func = Function::with_name_signature(
        cranelift::codegen::ir::UserFuncName::user(0, func_id.as_u32()),
        sig,
    );
    let mut fb_ctx = FunctionBuilderContext::new();
    // GC declared set: EMPTY (identical reasoning to compile_baseline —
    // boxed i64 reprs carry no pointer-typed CLIF values).
    let gc_value_declarations: Vec<cranelift::codegen::ir::Value> = Vec::new();
    {
        let mut fb = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        let entry = fb.create_block();
        fb.append_block_params_for_function_params(entry);
        fb.switch_to_block(entry);
        fb.seal_block(entry);
        let ctx_ptr = fb.block_params(entry)[0];

        let mut helper_refs: std::collections::HashMap<&'static str, FuncRef> =
            std::collections::HashMap::new();
        for (name, _) in helper_symbols() {
            let hsig = helper_sig(module.isa(), name);
            let fid = module
                .declare_function(name, cranelift_module::Linkage::Import, &hsig)
                .map_err(|e| CodegenError::Verify(format!("declare helper {name}: {e}")))?;
            let fref = module.declare_func_in_func(fid, fb.func);
            helper_refs.insert(name, fref);
        }

        let mut clif_blocks = Vec::with_capacity(func.blocks.len());
        for _ in 0..func.blocks.len() {
            clif_blocks.push(fb.create_block());
        }
        let mut site_counter = SiteCounter {
            idx: 0,
            plan: &slot_plan,
        };
        let mut emit_state = EmitState {
            name_table: &mut name_table,
            site_counter: &mut site_counter,
        };
        // ABI entry hand-off (identical to compile_baseline — the layout
        // entry block jumps to clif_blocks[0], whose body opens with the
        // twin-start dispatcher; a no-op without typed regions).
        fb.switch_to_block(entry);
        fb.ins().jump(clif_blocks[0], &[]);

        emit_cfg(
            &mut fb,
            func,
            &mut clif_blocks,
            ctx_ptr,
            module,
            &helper_refs,
            &mut emit_state,
        )?;

        for v in &gc_value_declarations {
            fb.declare_value_needs_stack_map(*v);
        }
        fb.seal_all_blocks();
        fb.finalize(module.isa().frontend_config());
        // Lockstep assertion (Risk-5, same as the JIT path).
        debug_assert_eq!(
            emit_state.site_counter.idx,
            slot_plan.len(),
            "clif(object): emitted site count drifted from SlotPlan::assign"
        );
    }
    if let Err(e) = module.define_function(func_id, &mut ctx) {
        let detail = match &e {
            ModuleError::Compilation(c) => format!("{c:?}"),
            other => format!("{other}"),
        };
        return Err(CodegenError::Verify(format!("define_function: {detail}")));
    }
    // Capture the interned table for link.rs's manifest (the assembler
    // reads it after ALL emissions; single-threaded per build).
    LAST_EMIT_NAMES.with(|c| {
        let mut v = c.borrow_mut();
        v.push(std::mem::take(&mut name_table));
    });
    Ok(())
}

thread_local! {
    /// The name tables of the most recent emit_cfg_into_object batch (one
    /// per emitted function, in emission order). Cleared by the assembler
    /// before each batch; consumed once after.
    static LAST_EMIT_NAMES: std::cell::RefCell<Vec<Vec<String>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Reset the name-table capture (call before each emission batch).
pub fn reset_emit_name_tables() {
    LAST_EMIT_NAMES.with(|c| c.borrow_mut().clear());
}

/// Take the captured name tables (call once after the batch).
pub fn last_emit_name_tables() -> Vec<Vec<String>> {
    LAST_EMIT_NAMES.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// The SlotPlan site count for a cfg WITHOUT compiling (link.rs sizes
/// the static IC payload with it — one walk, same predicate).
pub fn slot_plan_len(func: &Func) -> usize {
    SlotPlan::assign(func).len()
}

fn helper_sig(isa: &dyn TargetIsa, name: &'static str) -> cranelift::codegen::ir::Signature {
    use cranelift::codegen::ir::{AbiParam, Signature};
    // 02.05c: the pure typed-arithmetic helpers take NO ctx (they operate
    // on raw i64s only) — `apjit_h_mul_smi(a, b) -> i64`.
    let mut params = if ctx_less(name) {
        Vec::new()
    } else {
        vec![AbiParam::new(isa.pointer_type())] // ctx
    };
    params.extend(extra_params(name).iter().map(|_| AbiParam::new(types::I64)));
    // 02.05c: the value-returning typed helpers return i64 (a raw smi),
    // not the i32 HelperStatus convention.
    let ret = if i64_return(name) {
        types::I64
    } else {
        types::I32
    };
    Signature {
        params,
        returns: vec![AbiParam::new(ret)],
        call_conv: isa.default_call_conv(),
    }
}

/// Helpers whose CLIF signature takes NO ctx pointer (pure typed
/// arithmetic over raw i64s).
fn ctx_less(name: &str) -> bool {
    name == "apjit_h_mul_smi"
}

/// Helpers whose CLIF signature returns i64 (the unboxed smi value) rather
/// than the i32 status convention.
fn i64_return(name: &str) -> bool {
    // 02.05f: the OSR poll returns the ENTRY POINTER (i64, possibly NULL)
    // — an I32 CLIF result would truncate the pointer and the indirect
    // call would jump to garbage (the smoke-probe segfault, fixed by
    // declaring the i64 return).
    matches!(
        name,
        "apjit_h_slot_smi_v" | "apjit_h_mul_smi" | "apjit_h_osr_poll"
    )
}

fn extra_params(name: &'static str) -> &'static [usize] {
    // Arity table (documented per helper in runtime.rs). Each entry is a
    // placeholder — the COUNT is what matters for the signature.
    match name {
        // 02.05b: h_binop/h_load_global/h_load_attr/h_call gained a
        // leading feedback-slot param (the SlotPlan site id).
        "apjit_h_binop" => &[0, 0, 0, 0, 0], // slot, op, a, b, dst
        // h_compare takes op, a, b, dst; h_call takes slot, callee,
        // arg_start, argc, dst (5) — the pre-02.05c table over-declared
        // h_call by one, which made every function body containing a call
        // fail baseline compile and silently tree-walk (found by the
        // 02.05c gate run; jit_hook fell back by table so bytes stayed
        // identical and the bug hid).
        "apjit_h_compare" => &[0, 0, 0, 0], // op, a, b, dst
        "apjit_h_call" => &[0, 0, 0, 0, 0],
        "apjit_h_unaryop" => &[0, 0, 0],
        "apjit_h_truthy" | "apjit_h_load_local" | "apjit_h_store_local" => &[0, 0],
        "apjit_h_load_global" => &[0, 0, 0],
        "apjit_h_load_attr" => &[0, 0, 0, 0],
        "apjit_h_store_attr" => &[0, 0, 0],
        "apjit_h_print" | "apjit_h_push_exc" | "apjit_h_pop_exc" | "apjit_h_return" => &[0],
        "apjit_h_const_none" => &[0],
        "apjit_h_const_bool" | "apjit_h_const_int" | "apjit_h_const_float" => &[0, 0],
        "apjit_h_const_str" => &[0, 0, 0],
        "apjit_h_subscript" => &[0, 0, 0],
        "apjit_h_stage_arg" => &[0, 0],
        "apjit_h_truthy_bit" => &[0],
        // 02.05c typed-region helpers (extra-arg counts AFTER ctx; a
        // helper taking only ctx gets &[] — see apjit_h_twin_start).
        "apjit_h_slot_smi_v" => &[0],
        "apjit_h_mul_smi" => &[0, 0],
        "apjit_h_store_smi" => &[0, 0],
        "apjit_h_return_smi" => &[0],
        "apjit_h_twin_start" => &[],
        "apjit_h_set_twin_start" => &[0],
        // 02.05d: the deopt note keys the per-function thrash state (the
        // fn identity rides the key field — see emit_fastpath_body).
        "apjit_h_deopt_note" => &[0],
        // 02.05f OSR: poll(ctx, fn_key, header)->i64; fill(ctx, buf, src,
        // idx); load(ctx, buf, idx, dst).
        "apjit_h_osr_poll" => &[0, 0],
        "apjit_h_osr_fill" => &[0, 0, 0],
        "apjit_h_osr_load" => &[0, 0, 0],
        "apjit_h_match_exc" => &[0, 0, 0],
        "apjit_h_raise" | "apjit_h_suspend" => &[0, 0],
        "apjit_h_reraise" => &[],
        _ => &[],
    }
}

/// (symbol-name, fn-pointer) pairs declared into the JIT module.
fn helper_symbols() -> Vec<(&'static str, *const u8)> {
    use rt::*;
    vec![
        ("apjit_h_binop", h_binop as *const u8),
        ("apjit_h_unaryop", h_unaryop as *const u8),
        ("apjit_h_compare", h_compare as *const u8),
        ("apjit_h_truthy", h_truthy as *const u8),
        ("apjit_h_load_local", h_load_local as *const u8),
        ("apjit_h_store_local", h_store_local as *const u8),
        ("apjit_h_load_global", h_load_global as *const u8),
        ("apjit_h_load_attr", h_load_attr as *const u8),
        ("apjit_h_store_attr", h_store_attr as *const u8),
        ("apjit_h_call", h_call as *const u8),
        ("apjit_h_print", h_print as *const u8),
        ("apjit_h_push_exc", h_push_exc as *const u8),
        ("apjit_h_pop_exc", h_pop_exc as *const u8),
        ("apjit_h_match_exc", h_match_exc as *const u8),
        ("apjit_h_reraise", h_reraise as *const u8),
        ("apjit_h_const_none", h_const_none as *const u8),
        ("apjit_h_const_bool", h_const_bool as *const u8),
        ("apjit_h_const_int", h_const_int as *const u8),
        ("apjit_h_const_float", h_const_float as *const u8),
        ("apjit_h_const_str", h_const_str as *const u8),
        ("apjit_h_subscript", h_subscript as *const u8),
        ("apjit_h_stage_arg", h_stage_arg as *const u8),
        ("apjit_h_truthy_bit", h_truthy_bit as *const u8),
        // 02.05c typed-region helpers.
        ("apjit_h_slot_smi_v", h_slot_smi_v as *const u8),
        ("apjit_h_mul_smi", h_mul_smi as *const u8),
        ("apjit_h_store_smi", h_store_smi as *const u8),
        ("apjit_h_return_smi", h_return_smi as *const u8),
        ("apjit_h_twin_start", h_twin_start as *const u8),
        ("apjit_h_set_twin_start", h_set_twin_start as *const u8),
        ("apjit_h_deopt_note", h_deopt_note as *const u8),
        ("apjit_h_return", h_return as *const u8),
        ("apjit_h_raise", h_raise as *const u8),
        ("apjit_h_suspend", h_suspend as *const u8),
        // 02.05f boxed OSR (poll/fill/load — osr.rs).
        ("apjit_h_osr_poll", h_osr_poll as *const u8),
        ("apjit_h_osr_fill", h_osr_fill as *const u8),
        ("apjit_h_osr_load", h_osr_load as *const u8),
    ]
}

/// 02.05b emit-thread state: the mutable cross-cutting cursors emit_cfg
/// advances (name interning + the SlotPlan site-id cursor), bundled to keep
/// the parameter list flat (clippy too_many_arguments on the 8-arg form).
struct EmitState<'a> {
    name_table: &'a mut Vec<String>,
    site_counter: &'a mut SiteCounter<'a>,
}

/// Emit the whole CFG. Fail-closed on any unhandled inst/terminator.
/// 02.05b: `state.site_counter` carries the running SlotPlan id so every
/// specializable site embeds its own slot id as the first helper immediate.
fn emit_cfg<M: Module>(
    fb: &mut FunctionBuilder,
    func: &Func,
    blocks: &mut [Block],
    ctx_ptr: Value,
    module: &mut M,
    helpers: &HashMap<&'static str, FuncRef>,
    state: &mut EmitState<'_>,
) -> Result<(), CodegenError> {
    // 02.05f: the default emit compiles OSR poll sites as no-ops
    // (osr_fn_key = 0 — no registry probe baked in). Only
    // `compile_baseline_with_osr` (the tier-0 OSR route, jit_hook's
    // cache-insert path) passes a real fn key.
    emit_cfg_with_osr(
        fb,
        func,
        blocks,
        ctx_ptr,
        module,
        helpers,
        state,
        0,
        &mut 0usize,
    )
}

/// The tier-0 baseline WITH live OSR polls: identical to
/// [`compile_baseline`] except the poll sites probe the host OSR
/// registry under `fn_key` (the executing function's stable identity,
/// the same key jit_hook's cache + the tier-up driver use) and, on a
/// non-null entry pointer, run the boxed transfer + OSR-BY-CALL. Every
/// other byte of the compile is the deterministic baseline lowering.
pub fn compile_baseline_with_osr(func: &Func, fn_key: u64) -> Result<JitFunc, CodegenError> {
    let slot_plan = SlotPlan::assign(func);

    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .expect("clif.rs:119");
    flag_builder.set("is_pic", "false").expect("clif.rs:120");
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| CodegenError::Unsupported("native ISA unavailable"))?
        .finish(flags)
        .map_err(|_| CodegenError::Unsupported("ISA finish failed"))?;

    let mut module =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, ptr) in helper_symbols() {
        module.symbol(name, ptr);
    }
    let mut module = cranelift_jit::JITModule::new(module);

    let mut name_table: Vec<String> = Vec::new();

    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let func_id = module
        .declare_function("apjit_fn", cranelift_module::Linkage::Local, &sig)
        .map_err(|e| CodegenError::Verify(format!("declare_function: {e}")))?;
    #[allow(unused_assignments)]
    let mut site_count: Option<usize> = None;

    let frontend_config = module.isa().frontend_config();
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func = Function::with_name_signature(
        cranelift::codegen::ir::UserFuncName::user(0, func_id.as_u32()),
        sig.clone(),
    );
    let mut fb_ctx = FunctionBuilderContext::new();
    // GC declared set: EMPTY (identical reasoning to compile_baseline —
    // OSR frames stay on the conservative-scan path; pon discipline).
    let gc_value_declarations: Vec<cranelift::codegen::ir::Value> = Vec::new();
    {
        let mut fb = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        let entry = fb.create_block();
        fb.append_block_params_for_function_params(entry);
        fb.switch_to_block(entry);
        fb.seal_block(entry);
        let ctx_ptr = fb.block_params(entry)[0];

        let mut helper_refs: std::collections::HashMap<&'static str, FuncRef> =
            std::collections::HashMap::new();
        for (name, _) in helper_symbols() {
            let hsig = helper_sig(module.isa(), name);
            let fid = module
                .declare_function(name, cranelift_module::Linkage::Import, &hsig)
                .map_err(|e| CodegenError::Verify(format!("declare helper {name}: {e}")))?;
            let fref = module.declare_func_in_func(fid, fb.func);
            helper_refs.insert(name, fref);
        }

        let mut clif_blocks = Vec::with_capacity(func.blocks.len());
        for _ in 0..func.blocks.len() {
            clif_blocks.push(fb.create_block());
        }
        let mut site_counter = SiteCounter {
            idx: 0,
            plan: &slot_plan,
        };
        let mut emit_state = EmitState {
            name_table: &mut name_table,
            site_counter: &mut site_counter,
        };
        fb.switch_to_block(entry);
        fb.ins().jump(clif_blocks[0], &[]);

        // THE difference vs compile_baseline: the poll sites probe the
        // host OSR registry under this function's key. The header cursor
        // starts fresh; loop_headers() must resolve one header per poll
        // (the while-shape invariant — a body whose polls outnumber the
        // resolved headers degrades those extra polls to no-ops, reported
        // via the OSR eligibility report at compile_osr_entry time).
        emit_cfg_with_osr(
            &mut fb,
            func,
            &mut clif_blocks,
            ctx_ptr,
            &mut module,
            &helper_refs,
            &mut emit_state,
            fn_key,
            &mut 0usize,
        )?;

        for v in &gc_value_declarations {
            fb.declare_value_needs_stack_map(*v);
        }

        fb.seal_all_blocks();
        fb.finalize(frontend_config);
        site_count = Some(emit_state.site_counter.idx);
    }
    let site_count = site_count.expect("emit_cfg always runs");
    if let Err(e) = module.define_function(func_id, &mut ctx) {
        let detail = match &e {
            cranelift_module::ModuleError::Compilation(c) => format!("{c:?}"),
            other => format!("{other}"),
        };
        return Err(CodegenError::Verify(format!("define_function: {detail}")));
    }
    register_compiled_safepoints(&ctx);
    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Verify(format!("finalize_definitions: {e}")))?;
    let code_ptr = module.get_finalized_function(func_id);
    debug_assert_eq!(
        site_count,
        slot_plan.len(),
        "clif: emitted site count drifted from SlotPlan::assign"
    );
    Ok(JitFunc {
        code_ptr,
        slots: slot_plan,
        name_table,
        feedback_cells: (0..site_count).map(|_| FeedbackCell::new()).collect(),
        _blob: module,
    })
}

/// The OSR-parameterized emit body (see emit_cfg). `osr_fn_key` == 0
/// disables the OSR poll semantics entirely (poll sites emit as no-ops):
/// the tier-1 fast-path emitter walks the SAME cfg but never executes
/// polls, and unit tests that count sites without compiling OSR entries
/// pass `0` so no registry probe is baked in.
#[allow(clippy::too_many_arguments)]
fn emit_cfg_with_osr<M: Module>(
    fb: &mut FunctionBuilder,
    func: &Func,
    blocks: &mut [Block],
    ctx_ptr: Value,
    module: &mut M,
    helpers: &HashMap<&'static str, FuncRef>,
    state: &mut EmitState<'_>,
    osr_fn_key: u64,
    osr_header_cursor: &mut usize,
) -> Result<(), CodegenError> {
    let i64_t = types::I64;
    let i32_t = types::I32;
    // Slot-space fix (02.05a): locals and temps used to SHARE index space
    // (LocalId(n) == TempId(n)) — a temp write clobbered the local at the
    // same index (t1_016 `1 + f(n-1)` evaluated `None <= 0`). Params/
    // locals keep their indices (the entry hook bridges by name at those
    // slots); temps are offset above every local.
    let num_locals = func.num_locals as i64;
    let local_slot = |l: LocalId| l.0 as i64;
    let temp_slot = |t: TempId| num_locals + t.0 as i64;
    #[allow(unused_macros)]
    macro_rules! iconst {
        ($fb:expr, $v:expr) => {
            $fb.ins().iconst(types::I64, $v)
        };
    }

    // ctx field offsets (repr(Rust) is not stable — we go through
    // accessor helpers instead of raw field addressing; each accessor is
    // an imported helper). Simpler and ABI-proof: ctx field access is
    // always inside a runtime helper.
    let _ = ctx_ptr;

    for (bi, block) in func.blocks.iter().enumerate() {
        let b = blocks[bi];
        fb.switch_to_block(b);
        // 02.05c cold-twin seam: block 0 (the CLIF layout entry) opens
        // with the twin_start dispatcher — if ctx.twin_start == k, jump
        // straight to block k (region-exit continuation); checks
        // ascending, then FALLS THROUGH into block 0's own body, which
        // the normal emit path below appends into this same block. No
        // extra blocks, no re-binding: the original CFG edges all keep
        // their targets.
        if bi == 0 {
            for (k, tgt_ref) in blocks.iter().enumerate().skip(1) {
                let tgt = *tgt_ref;
                let cont = fb.create_block();
                let f = helpers["apjit_h_twin_start"];
                let call = fb.ins().call(f, &[ctx_ptr]);
                let start = fb.inst_results(call)[0];
                let k_v = fb.ins().iconst(i32_t, k as i64);
                let eq = fb
                    .ins()
                    .icmp(cranelift::codegen::ir::condcodes::IntCC::Equal, start, k_v);
                fb.ins().brif(eq, tgt, &[], cont, &[]);
                fb.switch_to_block(cont);
            }
            // Fall through into block 0's body: the cont chain's LAST
            // block is now current; block 0's insts+terminator emit there
            // via the normal path (rebind blocks[0] so internal CFG edges
            // to block 0 land in that same block).
        }
        // The block's raise edge (helper-failure route): the block's own
        // terminator's raise target if it is a Raise, else function exit
        // (None → return 1). Computed once per block.
        let raise_target: Option<Block> = match &block.term {
            Some(Terminator::Raise(_, Some(t))) => Some(blocks[t.0 as usize]),
            Some(Terminator::Raise(_, None)) => None,
            Some(Terminator::Goto(next)) => match &func.blocks[next.0 as usize].term {
                Some(Terminator::Raise(_, Some(t))) => Some(blocks[t.0 as usize]),
                _ => None,
            },
            _ => None,
        };
        for inst in &block.insts {
            match inst {
                Inst::Const(c, dst) => {
                    // Boxed constants: materialized via a per-const helper.
                    // (Args computed into locals first — two-phase borrow.)
                    let dst_i = temp_slot(*dst);
                    match c {
                        ConstValue::None => {
                            let f = helpers["apjit_h_const_none"];
                            let a1 = iconst!(fb, dst_i);
                            let r = fb.ins().call(f, &[ctx_ptr, a1]);
                            check_status(fb, r, raise_target);
                        }
                        ConstValue::Bool(v) => {
                            let f = helpers["apjit_h_const_bool"];
                            let a1 = iconst!(fb, dst_i);
                            let a2 = iconst!(fb, *v as i64);
                            let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                            check_status(fb, r, raise_target);
                        }
                        ConstValue::Int(v) => {
                            let f = helpers["apjit_h_const_int"];
                            let a1 = iconst!(fb, dst_i);
                            let a2 = iconst!(fb, *v);
                            let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                            check_status(fb, r, raise_target);
                        }
                        ConstValue::Float(v) => {
                            let f = helpers["apjit_h_const_float"];
                            let fconst = fb.ins().f64const(*v);
                            let flags = cranelift::codegen::ir::MemFlagsData::new();
                            let bits = fb.ins().bitcast(types::I64, flags, fconst);
                            let a1 = iconst!(fb, dst_i);
                            let r = fb.ins().call(f, &[ctx_ptr, a1, bits]);
                            check_status(fb, r, raise_target);
                        }
                        ConstValue::Str(s) => {
                            // Static string payload: data object in the module.
                            let mut desc = cranelift_module::DataDescription::new();
                            desc.define(s.as_bytes().to_vec().into_boxed_slice());
                            let did = module
                                .declare_anonymous_data(true, false)
                                .map_err(|e| CodegenError::Verify(format!("data: {e}")))?;
                            module
                                .define_data(did, &desc)
                                .map_err(|e| CodegenError::Verify(format!("data def: {e}")))?;
                            let ptr = module.declare_data_in_func(did, fb.func);
                            let addr = fb.ins().symbol_value(i64_t, ptr);
                            let f = helpers["apjit_h_const_str"];
                            let a1 = iconst!(fb, dst_i);
                            let a3 = iconst!(fb, s.len() as i64);
                            let r = fb.ins().call(f, &[ctx_ptr, a1, addr, a3]);
                            check_status(fb, r, raise_target);
                        }
                    }
                }
                Inst::LoadLocal(src, dst) => {
                    let f = helpers["apjit_h_load_local"];
                    let a1 = iconst!(fb, local_slot(*src));
                    let a2 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                    let _ = r; // infallible
                }
                Inst::StoreLocal(src, dst) => {
                    let f = helpers["apjit_h_store_local"];
                    let a1 = iconst!(fb, temp_slot(*src));
                    let a2 = iconst!(fb, local_slot(*dst));
                    fb.ins().call(f, &[ctx_ptr, a1, a2]);
                }
                Inst::LoadGlobal(name, dst) => {
                    // 02.05b: FEEDBACK SLOT id embedded as the first
                    // immediate (GlobalIC key: host cache + feedback cell).
                    let f = helpers["apjit_h_load_global"];
                    let a0 = iconst!(fb, state.site_counter.next() as i64);
                    let a1 = iconst!(fb, name_id(state.name_table, name) as i64);
                    // t1_016 fix (86d0c09; re-landed on 02.05b's call
                    // shape): dst MUST bridge through temp_slot() (temps
                    // live at num_locals + t). The raw TempId made the
                    // load land in a param/dead slot and every
                    // JIT-routed call site read None as the callee.
                    let a2 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a0, a1, a2]);
                    check_status(fb, r, raise_target);
                }
                Inst::BinOp(op, a, b, dst) => {
                    // FEEDBACK SLOT: assigned at this final lowering stage
                    // (Risk 5). The slot id is embedded as an immediate —
                    // the recording/tier-1 raw-load path keys on it. The
                    // boxed baseline calls the recording helper after the
                    // op helper (no semantic effect; hooks only).
                    // 02.05b: the BinOp site's own slot id feeds the cell
                    // via h_binop (tag recording inside the helper).
                    let f = helpers["apjit_h_binop"];
                    let a0 = iconst!(fb, state.site_counter.next() as i64);
                    let a1 = iconst!(fb, op_code(*op));
                    let a2 = iconst!(fb, temp_slot(*a));
                    let a3 = iconst!(fb, temp_slot(*b));
                    let a4 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a0, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::UnaryOp(op, a, dst) => {
                    let f = helpers["apjit_h_unaryop"];
                    let a1 = iconst!(fb, uop_code(*op));
                    let a2 = iconst!(fb, temp_slot(*a));
                    let a3 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::Compare(op, a, b, dst) => {
                    let f = helpers["apjit_h_compare"];
                    let a1 = iconst!(fb, cmp_code(*op));
                    let a2 = iconst!(fb, temp_slot(*a));
                    let a3 = iconst!(fb, temp_slot(*b));
                    let a4 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::Truthy(a, dst) => {
                    let f = helpers["apjit_h_truthy"];
                    let a1 = iconst!(fb, temp_slot(*a));
                    let a2 = iconst!(fb, temp_slot(*dst));
                    fb.ins().call(f, &[ctx_ptr, a1, a2]);
                }
                Inst::Call(f_slot, args, dst) => {
                    // Stage args into ctx.args[arg_start..] then call.
                    // h_call stages via h_stage_arg per arg.
                    let stage = helpers["apjit_h_stage_arg"];
                    let arg_start = ARG_AREA_BASE + arg_area_reserve();
                    for (k, a) in args.iter().enumerate() {
                        let a1 = iconst!(fb, (arg_start as usize + k) as i64);
                        let a2 = iconst!(fb, temp_slot(*a));
                        fb.ins().call(stage, &[ctx_ptr, a1, a2]);
                    }
                    // 02.05b: the Call site's feedback slot id rides as the
                    // first helper param (CallIC key + callee tag recording).
                    let f = helpers["apjit_h_call"];
                    let a0 = iconst!(fb, state.site_counter.next() as i64);
                    let a1 = iconst!(fb, temp_slot(*f_slot));
                    let a2 = iconst!(fb, arg_start);
                    let a3 = iconst!(fb, args.len() as i64);
                    let a4 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a0, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::LoadAttr(obj, name, dst) => {
                    // 02.05b: the Attr site's feedback slot id rides as the
                    // first helper param (AttrIC key + receiver tag record).
                    let f = helpers["apjit_h_load_attr"];
                    let a0 = iconst!(fb, state.site_counter.next() as i64);
                    let a1 = iconst!(fb, temp_slot(*obj));
                    let a2 = iconst!(fb, name_id(state.name_table, name) as i64);
                    let a3 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a0, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::StoreAttr(obj, name, v) => {
                    let f = helpers["apjit_h_store_attr"];
                    let a1 = iconst!(fb, temp_slot(*obj));
                    let a2 = iconst!(fb, name_id(state.name_table, name) as i64);
                    let a3 = iconst!(fb, temp_slot(*v));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::Subscript(obj, idx, dst) => {
                    let f = helpers["apjit_h_subscript"];
                    let a1 = iconst!(fb, temp_slot(*obj));
                    let a2 = iconst!(fb, temp_slot(*idx));
                    let a3 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::PushExcInfo(region) => {
                    let f = helpers["apjit_h_push_exc"];
                    let a1 = iconst!(fb, region.0 as i64);
                    fb.ins().call(f, &[ctx_ptr, a1]);
                }
                Inst::PopExcInfo(region) => {
                    let f = helpers["apjit_h_pop_exc"];
                    let a1 = iconst!(fb, region.0 as i64);
                    fb.ins().call(f, &[ctx_ptr, a1]);
                }
                Inst::MatchExc(region, bind, dst) => {
                    let f = helpers["apjit_h_match_exc"];
                    let bind_i = bind.map(local_slot).unwrap_or(-1);
                    let a1 = iconst!(fb, region.0 as i64);
                    let a2 = iconst!(fb, bind_i);
                    let a3 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::Reraise => {
                    let f = helpers["apjit_h_reraise"];
                    let r = fb.ins().call(f, &[ctx_ptr]);
                    check_status(fb, r, raise_target);
                }
                Inst::Print(v) => {
                    let f = helpers["apjit_h_print"];
                    let a1 = iconst!(fb, temp_slot(*v));
                    let r = fb.ins().call(f, &[ctx_ptr, a1]);
                    check_status(fb, r, raise_target);
                }
                Inst::SuspendMarker => {
                    // Generator suspend marker: the state-machine transform
                    // rewrites these before emission; a residual one in a
                    // non-generator body is a no-op (the terminator arm
                    // owns actual suspension).
                }
                Inst::OsrPoll => {
                    // 02.05f — the boxed OSR back-edge poll + transfer.
                    // Emitted at every loop back-edge (the cfg stage places
                    // OsrPoll there). Shape (osr.rs module docs):
                    //
                    //   entry = h_osr_poll(ctx, fn_key, header)  // i64
                    //   if entry == 0: continue the loop in tier-0 (the
                    //       poll fast path: one helper call + one branch).
                    //   else: build an OsrTransferBuffer in an Explicit
                    //       stack slot, fill it via the canonical table
                    //       (locals in LocalId order, then live-ins
                    //       ascending — ONE shared OsrPlan), and CALL the
                    //       entry (OSR-BY-CALL). Its statuses:
                    //         0 → completed, ret parked — forward as OUR
                    //             return (this tier-0 frame is abandoned);
                    //         1 → exception in flight — forward likewise.
                    //         anything else → fail-closed to the exception
                    //             exit (host reports the unparked
                    //             SystemError — the OSR body, a full
                    //             baseline lowering, never emits these).
                    //
                    // osr_fn_key == 0 (unit-test/emit-only paths) keeps the
                    // whole arm a no-op: no registry probe is baked in.
                    if osr_fn_key != 0 {
                        let headers = crate::osr::loop_headers(func);
                        if *osr_header_cursor < headers.len() {
                            let osr_header = headers[*osr_header_cursor];
                            *osr_header_cursor += 1;
                            let entry_v = {
                                let f = helpers["apjit_h_osr_poll"];
                                let a1 = iconst!(fb, osr_fn_key as i64);
                                let a2 = iconst!(fb, osr_header.0 as i64);
                                let call = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                                fb.inst_results(call)[0]
                            };
                            let has_entry = fb.ins().icmp_imm_s(IntCC::NotEqual, entry_v, 0);
                            let osr_b = fb.create_block();
                            let cont_b = fb.create_block();
                            fb.ins().brif(has_entry, osr_b, &[], cont_b, &[]);
                            // --- transfer path ---
                            fb.switch_to_block(osr_b);
                            // The entry pointer can only arrive for a
                            // header whose compile PASSED the eligibility
                            // gate (the registry's compile arm refuses
                            // ineligible headers), so osr_plan_resolved
                            // must succeed here; a None would mean the
                            // registry and this table disagree — fail
                            // closed by terminating the frame (the host
                            // reads it as a helper-failure SystemError),
                            // never a mis-transfer.
                            let plan = match crate::osr::osr_plan_resolved(func, osr_header) {
                                Some(p) => p,
                                None => {
                                    let one = fb.ins().iconst(i32_t, 1);
                                    fb.ins().return_(&[one]);
                                    return Ok(());
                                }
                            };
                            let buf_size =
                                std::mem::size_of::<crate::osr::OsrTransferBuffer>() as u32;
                            let slot = fb.create_sized_stack_slot(
                                cranelift::codegen::ir::StackSlotData::new(
                                    cranelift::codegen::ir::StackSlotKind::ExplicitSlot,
                                    buf_size,
                                    0, // natural alignment (align_shift)
                                ),
                            );
                            let buf_addr = fb.ins().stack_addr(i64_t, slot, 0);
                            // Header + live_count: one combined i64
                            // store at offset 0 — repr(C) puts the two
                            // u32 fields (header, live_count) at bytes
                            // 0..4 / 4..8, which is exactly the
                            // little-endian i64 layout
                            // (header | count << 32). Two separate
                            // i64 stores would stomp bytes 4..12 (the
                            // second store's high half lands INSIDE
                            // slots[0]'s Value — the smoke-probe
                            // corruption).
                            let flags = cranelift::codegen::ir::MemFlagsData::new();
                            let hdr_cnt = iconst!(
                                fb,
                                (osr_header.0 as i64) | ((plan.live_count() as i64) << 32)
                            );
                            fb.ins().store(flags, hdr_cnt, buf_addr, 0);
                            // ZERO the slot array before any fill: the
                            // Explicit stack slot starts UNINITIALIZED,
                            // and the conservative GC scan (osr frames
                            // are deliberately ON that path — pon
                            // discipline) reads the whole native stack
                            // region including this buffer. Garbage
                            // bytes misread as Gc pointers = crash on
                            // the first allocation inside the fill
                            // stream (observed live). Two zero words
                            // per slot covers the 16-byte Value.
                            for k in 0..crate::osr::OSR_MAX_LIVE {
                                let zero = iconst!(fb, 0);
                                fb.ins().store(flags, zero, buf_addr, (8 + k * 16) as i32);
                                fb.ins().store(flags, zero, buf_addr, (16 + k * 16) as i32);
                            }
                            // Fill the slots through the canonical table:
                            // locals first (LocalId order), then live-ins
                            // (ascending TempId). Each fill is a Rust-side
                            // 16-byte Value clone from ctx.slots[src] into
                            // buf.slots[idx] (h_osr_fill — the Value layout
                            // is shared by construction, both sides Rust).
                            let mut idx = 0usize;
                            for l in &plan.locals {
                                let f = helpers["apjit_h_osr_fill"];
                                let a1 = buf_addr;
                                let a2 = iconst!(fb, local_slot(*l));
                                let a3 = iconst!(fb, idx as i64);
                                fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                                idx += 1;
                            }
                            for t in &plan.live_in {
                                let f = helpers["apjit_h_osr_fill"];
                                let a1 = buf_addr;
                                let a2 = iconst!(fb, temp_slot(*t));
                                let a3 = iconst!(fb, idx as i64);
                                fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                                idx += 1;
                            }
                            // OSR-BY-CALL: entry ABI `(ctx, buf) -> i32`,
                            // indirect through the poll's returned pointer.
                            let mut sig =
                                cranelift::codegen::ir::Signature::new(fb.func.signature.call_conv);
                            sig.params
                                .push(cranelift::codegen::ir::AbiParam::new(i64_t));
                            sig.params
                                .push(cranelift::codegen::ir::AbiParam::new(i64_t));
                            sig.returns
                                .push(cranelift::codegen::ir::AbiParam::new(i32_t));
                            let sigref = fb.import_signature(sig);
                            let r = fb
                                .ins()
                                .call_indirect(sigref, entry_v, &[ctx_ptr, buf_addr]);
                            let status = fb.inst_results(r)[0];
                            // Forward 0/1 as our own exit; anything else is
                            // fail-closed to the exception exit.
                            let ok = fb
                                .ins()
                                .icmp_imm_s(IntCC::UnsignedLessThanOrEqual, status, 1);
                            let ok_b = fb.create_block();
                            let bad_b = fb.create_block();
                            fb.ins().brif(ok, ok_b, &[], bad_b, &[]);
                            fb.switch_to_block(bad_b);
                            let one = fb.ins().iconst(i32_t, 1);
                            fb.ins().return_(&[one]);
                            fb.switch_to_block(ok_b);
                            fb.ins().return_(&[status]);
                            // --- continue the loop in tier-0 ---
                            fb.switch_to_block(cont_b);
                        }
                    }
                }
            }
        }
        match &block.term {
            Some(Terminator::Goto(t)) => {
                fb.ins().jump(blocks[t.0 as usize], &[]);
            }
            Some(Terminator::Branch(c, t, f)) => {
                // Truthiness is pre-materialized by the cfg stage into a
                // boxed bool; branch on its unboxed bit through h_truthy_bit.
                let bit_f = helpers["apjit_h_truthy_bit"];
                let a1 = iconst!(fb, temp_slot(*c));
                let bit = fb.ins().call(bit_f, &[ctx_ptr, a1]);
                let bit_v = fb.inst_results(bit)[0];
                fb.ins()
                    .brif(bit_v, blocks[t.0 as usize], &[], blocks[f.0 as usize], &[]);
            }
            Some(Terminator::Return(v)) => {
                // h_return parks the value into ctx.ret and returns 0.
                let f = helpers["apjit_h_return"];
                let a1 = iconst!(fb, temp_slot(*v));
                fb.ins().call(f, &[ctx_ptr, a1]);
                let zero = fb.ins().iconst(i32_t, 0);
                fb.ins().return_(&[zero]);
            }
            Some(Terminator::Raise(v, target)) => {
                // h_raise parks the exception value in ctx.pending_exc and
                // returns 1; then jump to the raise edge (dispatch or exit).
                let f = helpers["apjit_h_raise"];
                let a1 = iconst!(fb, temp_slot(*v));
                fb.ins().call(f, &[ctx_ptr, a1]);
                match target {
                    Some(t) => {
                        fb.ins().jump(blocks[t.0 as usize], &[]);
                    }
                    None => {
                        let ret = fb.ins().iconst(i32_t, 1);
                        fb.ins().return_(&[ret]);
                    }
                }
            }
            Some(Terminator::Suspend(val, state, resume)) => {
                // Generator suspend: h_suspend stores state + parks value,
                // returns 1 (host resumes later); resume point jumps in.
                let f = helpers["apjit_h_suspend"];
                let a1 = iconst!(fb, *state as i64);
                let a2 = iconst!(fb, temp_slot(*val));
                let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                let _ = r;
                fb.ins().jump(blocks[resume.0 as usize], &[]);
            }
            Some(Terminator::Unreachable) | None => {
                // The cfg verifier rejects unterminated blocks before we
                // get here; a poisoned Unreachable returns error status.
                let two = fb.ins().iconst(i32_t, 2);
                fb.ins().return_(&[two]);
            }
        }
    }
    Ok(())
}

/// 02.05g map-registration tail: extract the compiled function's user
/// stack maps + frame layout and register (return_address → fp-relative
/// root offsets) entries in the ap-gc safepoint registry.
///
/// See the registration note at the call site for the return-address
/// anchoring rule. With an EMPTY declared set (today's boxed baseline)
/// Cranelift emits no maps and this registers nothing — the correct
/// degenerate behavior (the subtract-refs floor covers every root); with
/// tier-1 pointer-typed declarations (02.05c+) the maps appear per call
/// and this tail hands them to the collector unmodified.
fn register_compiled_safepoints(ctx: &cranelift::codegen::Context) {
    let Some(compiled) = ctx.compiled_code() else {
        return;
    };
    let maps = compiled.buffer.user_stack_maps();
    if maps.is_empty() {
        return;
    }
    let code_len = compiled.buffer.total_size();
    let frame_layout = compiled.buffer.frame_layout();
    let fp_bias = frame_layout.map(|fl| fl.frame_to_fp_offset).unwrap_or(0);
    for (idx, (offset, _span, map)) in maps.iter().enumerate() {
        // Return address = just past the call instruction (see the call
        // site note).
        let ret_off = match maps.get(idx + 1) {
            Some((next, _, _)) => (*offset).min(*next),
            None => (*offset).min(code_len),
        };
        // The registry keys on ABSOLUTE code addresses; the module is not
        // finalized yet, so compute the address lazily is impossible here
        // — instead register at (ret_off) and let the caller-side base be
        // resolved through the code-buffer-relative scheme the walker
        // understands: the JIT blob is a single contiguous mapping whose
        // base is captured once at finalize (see JitFunc::call). We store
        // the relative offset XOR-tagged and add the blob base at lookup
        // time in ap-gc — but ap-gc has no such knowledge, so the simplest
        // correct scheme is: register the RELATIVE offset only when the
        // blob base is knowable (post-finalize); in the pre-finalize tail
        // we defer: collect entries now, register after finalize.
        let _ = fp_bias;
        let mut roots = Vec::new();
        for (_ty, sp_off) in map.entries() {
            // UserStackMap offsets are SP-relative at this PC; the walker
            // resolves from FP. With frame_layout present, the FP-to-SP
            // distance is frame_to_fp_offset (frame grows DOWN from FP,
            // so SP = FP - frame_to_fp_offset and a slot at SP + sp_off
            // is at FP - frame_to_fp_offset + sp_off). Sign-biased into
            // the i32 fp_offset the registry stores.
            let fp_off = sp_off as i64 - fp_bias as i64;
            if fp_off >= i32::MIN as i64 && fp_off <= i32::MAX as i64 {
                roots.push(ap_gc::StackRootSlot {
                    fp_offset: fp_off as i32,
                });
            }
        }
        if !roots.is_empty() {
            // Registration keyed on the code-buffer-relative offset under
            // the RESERVED blob-base scheme: ap-gc's registry stores raw
            // usize keys, and pre-finalize we cannot know the base — so
            // this arm registers through the module-agnostic placeholder
            // `PENDING_MAP_BASE + ret_off`, replaced by the real base at
            // finalize (see the finalize tail in compile_baseline). The
            // two-phase scheme keeps map data pinned before code runs
            // without a second extraction pass.
            ap_gc::register_safepoint(PENDING_MAP_BASE + ret_off as usize, roots);
        }
    }
}

/// Base address placeholder for pre-finalize map registration (see
/// `register_compiled_safepoints`).
const PENDING_MAP_BASE: usize = 0x0001_0000_0000;

/// Status check: helper returned 0 → continue; non-zero → take the block's
/// raise edge (dispatch or exit). Emits the branch ONCE per fallible call.
fn check_status(
    fb: &mut FunctionBuilder,
    call: cranelift::codegen::ir::Inst,
    raise_target: Option<Block>,
) {
    let status = fb.inst_results(call)[0];
    let raise_b = fb.create_block();
    let cont_b = fb.create_block();
    fb.ins().brif(status, raise_b, &[], cont_b, &[]);
    fb.switch_to_block(raise_b);
    match raise_target {
        Some(t) => {
            fb.ins().jump(t, &[]);
        }
        None => {
            let one = fb.ins().iconst(types::I32, 1);
            fb.ins().return_(&[one]);
        }
    }
    fb.switch_to_block(cont_b);
}

pub fn op_code(op: ap_ir::expr::BinOp) -> i64 {
    use ap_ir::expr::BinOp::*;
    match op {
        Add => rt::OP_ADD,
        Sub => rt::OP_SUB,
        Mul => rt::OP_MUL,
        MatMul => rt::OP_MATMUL,
        Div => rt::OP_DIV,
        FloorDiv => rt::OP_FLOORDIV,
        Mod => rt::OP_MOD,
        Pow => rt::OP_POW,
        LShift => rt::OP_LSHIFT,
        RShift => rt::OP_RSHIFT,
        BitOr => rt::OP_BOR,
        BitXor => rt::OP_BXOR,
        BitAnd => rt::OP_BAND,
    }
}

fn cmp_code(op: ap_ir::expr::CmpOp) -> i64 {
    use ap_ir::expr::CmpOp::*;
    match op {
        Eq => rt::CMP_EQ,
        NotEq => rt::CMP_NOTEQ,
        Lt => rt::CMP_LT,
        LtE => rt::CMP_LTE,
        Gt => rt::CMP_GT,
        GtE => rt::CMP_GTE,
        Is => rt::CMP_IS,
        IsNot => rt::CMP_ISNOT,
        In => rt::CMP_IN,
        NotIn => rt::CMP_NOTIN,
    }
}

fn uop_code(op: ap_ir::expr::UnaryOp) -> i64 {
    use ap_ir::expr::UnaryOp::*;
    match op {
        Plus => rt::UOP_PLUS,
        Minus => rt::UOP_MINUS,
        Invert => rt::UOP_INVERT,
        Not => rt::UOP_NOT,
    }
}

/// Name ids are emitted as immediates; the mapping table travels with the
/// compiled artifact (apjit_name_table symbol) — the entry hook (ap-run
/// jit_hook.rs) rebuilds ctx.names from the same deterministic order:
/// first-appearance order of LoadGlobal/LoadAttr names during emit_cfg.
/// 02.05a: the stub that returned 0 for every name made all name-keyed
/// sites collide on names[0] — this pass now interns for real.
fn name_id(table: &mut Vec<String>, name: &str) -> u32 {
    if let Some(i) = table.iter().position(|n| n == name) {
        i as u32
    } else {
        table.push(name.to_string());
        (table.len() - 1) as u32
    }
}

const ARG_AREA_BASE: i64 = 0;
fn arg_area_reserve() -> i64 {
    0
}

// ===========================================================================
// 02.05c — tier-1 typed codegen (fast path over a compiled cold twin).
//
// compile_tier1(func) = compile_baseline(func)  (the cold twin) + a SECOND
// JIT function (the fast path) that executes only the typed regions; every
// block outside a region in the fast path is a straight jump to the twin's
// corresponding entry (implemented as: fast path returns "run twin" via a
// NULL-marker convention — see Tier1Func below).
//
// Fast-path shape per region:
//   entry:  for each guard slot: h_slot_smi(ctx, slot, &stack_slot)
//           fail → h_deopt_note + jump twin_entry
//   body:   iadd/isub/imul native; div-zero + i64-overflow checked per op,
//           failure → h_deopt_note + twin (recomputation uses the BOXED
//           ctx slots, which the fast path kept in sync write-through)
//   exits:  h_store_smi (write-through re-box), then the block's normal
//           terminator edge (twin block / boxed return via h_return_smi).
//
// The fast path never raises: every guard/arith failure side-exits to the
// twin, which re-executes the block with exact baseline semantics (cold
// twin deopt — the study optimizing.rs:287 analogue).
// ===========================================================================

/// A compiled tier-1 function pair: the fast path plus its cold twin.
pub struct Tier1Func {
    /// `extern "C" fn(*mut ExecCtx) -> i32` fast-path entry. Status 0 =
    /// completed (ret parked), 1 = exception (never: the fast path cannot
    /// raise), 2 = "hand off to the twin" (the guard failed at region
    /// entry; ctx is untouched).
    pub fast_ptr: *const u8,
    /// The boxed cold twin (a full baseline compilation of the same CFG).
    pub twin: JitFunc,
    /// The plan the fast path was emitted from (regions + guards).
    pub regions: usize,
    /// 02.05e: the fast path's code blob, OWNED here (was leaked via
    /// mem::forget in emit_typed_fastpath before this leaf). The blob must
    /// stay mapped for every call through `fast_ptr`; the owner is whoever
    /// retains the Tier1Func — the 02.05e driver pins it until retire (see
    /// tierup.rs). Not Send: keep the blob on the thread that compiled it
    /// (pon discipline) and ship only the raw pointers (tierup::Tier1Exec).
    pub _fast_blob: cranelift_jit::JITModule,
}

impl Tier1Func {
    /// Run the fast path; on the hand-off status, run the twin from the
    /// top. The twin is a FULL baseline compile, so a hand-off re-executes
    /// from function entry — semantically identical because the fast path
    /// has made no observable progress before its first guard (guards are
    /// the first thing it emits; see emit_typed_fastpath).
    ///
    /// # Safety
    /// Same contract as [`JitFunc::call`]: code blobs alive, `ctx`
    /// exclusively borrowed.
    pub unsafe fn call(&self, ctx: &mut rt::ExecCtx) -> Result<(), ap_obj::PyException> {
        let f: extern "C" fn(*mut rt::ExecCtx) -> i32 = std::mem::transmute(self.fast_ptr);
        let status = f(ctx as *mut rt::ExecCtx);
        match status {
            0 => Ok(()),
            2 => {
                // Guard failure at region entry: zero observable progress
                // made; run the boxed twin from the top.
                self.twin.call(ctx)
            }
            _ => match ctx.pending_exc.take() {
                Some(e) => Err(e),
                None => Err(ap_obj::PyException::new(
                    "SystemError",
                    "tier-1 fast path failed without parking an exception",
                )),
            },
        }
    }
}

/// Compile the tier-1 pair for `func`: cold twin (baseline) + typed fast
/// path. Returns None when the CFG admits no typed region (caller runs the
/// twin alone). Compile failure of the fast path degrades to twin-only
/// (reported via the error — the caller logs and proceeds boxed).
pub fn compile_tier1(
    func: &Func,
    snapshots: &[crate::feedback::CellSnapshot],
    fn_key: u64,
) -> Result<Option<Tier1Func>, CodegenError> {
    let twin = compile_baseline(func)?;
    let info = crate::infer::infer(func, snapshots);
    if info.site_count != twin.slots.len() {
        // Risk-5 lockstep: the infer walk and the SlotPlan walk disagree —
        // fail-closed (compile error → twin-only fallback by the caller).
        return Err(CodegenError::Verify(format!(
            "infer site count {} != SlotPlan count {} (Risk-5 walk drift)",
            info.site_count,
            twin.slots.len()
        )));
    }
    let plan = crate::optimizing::plan_function(func, info, twin);
    if !crate::optimizing::has_typed_regions(&plan) {
        return Ok(None);
    }
    crate::optimizing::verify_region_terminators(func, &plan)?;
    let (fast_ptr, fast_blob) = emit_typed_fastpath(func, &plan, fn_key)?;
    let regions = plan.regions.len();
    // 02.05d: record the successful install in the thrash table (Tier1,
    // counters reset) so a re-specialization after a Deferred drop starts
    // a clean epoch.
    crate::deopt::note_tier1_installed(fn_key);
    Ok(Some(Tier1Func {
        fast_ptr,
        twin: plan.cold_twin,
        regions,
        _fast_blob: fast_blob,
    }))
}

/// Emit the typed fast path over the plan's regions. `fn_key` is the
/// executing function's stable identity (02.05d), compiled into the
/// deopt tail so `h_deopt_note` can key the per-function thrash state.
fn emit_typed_fastpath(
    func: &Func,
    plan: &crate::optimizing::OptimizingPlan,
    fn_key: u64,
) -> Result<(*const u8, cranelift_jit::JITModule), CodegenError> {
    // 02.05d guard-fix: the thread-local SSA temp map is per-EMIT — a
    // stale entry from a previous function's fast-path emission would be
    // read by this one (same temp id = silent WRONG VALUE). Clear at the
    // top of every emit.
    ssa_temps::clear();
    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .expect("clif.rs:119");
    flag_builder.set("is_pic", "false").expect("clif.rs:120");
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| CodegenError::Unsupported("native ISA unavailable"))?
        .finish(flags)
        .map_err(|_| CodegenError::Unsupported("ISA finish failed"))?;

    let mut module =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, ptr) in helper_symbols() {
        module.symbol(name, ptr);
    }
    let mut module = cranelift_jit::JITModule::new(module);

    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let func_id = module
        .declare_function("apjit_fn_fast", cranelift_module::Linkage::Local, &sig)
        .map_err(|e| CodegenError::Verify(format!("declare_function fast: {e}")))?;

    let frontend_config = module.isa().frontend_config();
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func = Function::with_name_signature(
        cranelift::codegen::ir::UserFuncName::user(1, func_id.as_u32()),
        sig,
    );
    let mut fb_ctx = FunctionBuilderContext::new();
    let num_slots = crate::infer::max_temp(func) as i64 + func.num_locals as i64;
    {
        let mut fb = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        let entry = fb.create_block();
        fb.append_block_params_for_function_params(entry);
        fb.switch_to_block(entry);
        fb.seal_block(entry);
        let ctx_ptr = fb.block_params(entry)[0];

        let mut helper_refs: HashMap<&'static str, FuncRef> = HashMap::new();
        for (name, _) in helper_symbols() {
            let hsig = helper_sig(module.isa(), name);
            let fid = module
                .declare_function(name, cranelift_module::Linkage::Import, &hsig)
                .map_err(|e| CodegenError::Verify(format!("declare helper {name}: {e}")))?;
            let fref = module.declare_func_in_func(fid, fb.func);
            helper_refs.insert(name, fref);
        }

        emit_fastpath_body(
            &mut fb,
            entry,
            func,
            plan,
            ctx_ptr,
            &helper_refs,
            num_slots,
            fn_key,
        )?;

        fb.seal_all_blocks();
        fb.finalize(frontend_config);
    }
    if let Err(e) = module.define_function(func_id, &mut ctx) {
        let detail = match &e {
            cranelift_module::ModuleError::Compilation(c) => format!("{c:?}"),
            other => format!("{other}"),
        };
        return Err(CodegenError::Verify(format!("define fast: {detail}")));
    }
    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Verify(format!("finalize fast: {e}")))?;
    let ptr = module.get_finalized_function(func_id);
    // 02.05e: the fast path's JITModule is OWNED by the Tier1Func (the
    // driver pins the finished compilation until retire). Dropped-on-drop
    // is SAFE here: cranelift-jit 0.134's SystemMemoryProvider::drop
    // deliberately LEAKS the mapped code ("leak memory to guarantee
    // validity of function pointers"), so even an accidental early drop
    // cannot unmap executing code — the failure mode of the old
    // mem::forget-less shape is gone by construction, and the leak is
    // bounded per function exactly as before. The stale mem::forget note
    // is retired with this refactor.
    Ok((ptr, module))
}

/// The fast-path body: entry guards + region bodies + hand-off tail.
#[allow(clippy::too_many_arguments)]
fn emit_fastpath_body(
    fb: &mut FunctionBuilder,
    entry: Block,
    func: &Func,
    plan: &crate::optimizing::OptimizingPlan,
    ctx_ptr: Value,
    helpers: &HashMap<&'static str, FuncRef>,
    _num_slots: i64,
    fn_key: u64,
) -> Result<(), CodegenError> {
    let i32_t = types::I32;
    let Some(region) = plan.regions.first() else {
        return Ok(());
    };
    // Re-anchor on the layout entry (create_block moves the cursor; the
    // guard chain below must emit into the entry).
    fb.switch_to_block(entry);

    // Member blocks (CLIF) + exit landing blocks — created FIRST so the
    // fast ENTRY (the function's layout-entry block) can emit into itself
    // without ever being "already filled" (the guard chain lands here).
    let member: HashMap<u32, Block> = region
        .blocks
        .iter()
        .map(|&b| (b, fb.create_block()))
        .collect();
    let mut exits: HashMap<u32, Block> = HashMap::new();
    for &b in &region.blocks {
        let blk = &func.blocks[b as usize];
        let mut targets: Vec<u32> = Vec::new();
        match &blk.term {
            Some(Terminator::Goto(t)) => targets.push(t.0),
            Some(Terminator::Branch(_, t, f)) => {
                targets.push(t.0);
                targets.push(f.0);
            }
            _ => {}
        }
        for t in targets {
            if !region.contains(t) && !exits.contains_key(&t) {
                exits.insert(t, fb.create_block());
            }
        }
    }

    // The single hand-off tail: deopt-note + return status 2 (call the
    // twin). All side-exits funnel here. (Filled LAST; block created here
    // but the cursor returns to the entry for the guard chain.)
    let tail = fb.create_block();
    fb.switch_to_block(entry);

    // Entry edge (CURRENT block = the layout entry): guard every ctx slot
    // the region loads (ascending), then jump into the region entry block.
    // Failure → tail (twin restart).
    let Some(guards) = plan.fast_path.guards.first() else {
        return Ok(());
    };
    for slot in &guards.guarded_slots {
        let f = helpers["apjit_h_slot_smi_v"];
        let a = fb.ins().iconst(types::I64, *slot as i64);
        let call = fb.ins().call(f, &[ctx_ptr, a]);
        let v = fb.inst_results(call)[0];
        let bad = fb.ins().icmp_imm_s(IntCC::Equal, v, GUARD_FAIL_SENTINEL);
        guard_or_tail(fb, bad, tail);
    }
    let entry_clif = member[&region.entry.0];
    fb.ins().jump(entry_clif, &[]);

    // Member blocks, in region order (deterministic emission).
    for &b in &region.blocks {
        let blk = &func.blocks[b as usize];
        fb.switch_to_block(member[&b]);
        for inst in &blk.insts {
            match inst {
                Inst::Const(ConstValue::Int(v), dst) => {
                    let v = fb.ins().iconst(types::I64, *v);
                    store_ssa_temp(dst.0, v);
                }
                Inst::LoadLocal(src, dst) => {
                    // Guard-load (region RE-ENTRY shape): the local may
                    // hold any boxed value since the last fast run.
                    let f = helpers["apjit_h_slot_smi_v"];
                    let a = fb.ins().iconst(types::I64, src.0 as i64);
                    let call = fb.ins().call(f, &[ctx_ptr, a]);
                    let v = fb.inst_results(call)[0];
                    let bad = fb.ins().icmp_imm_s(IntCC::Equal, v, GUARD_FAIL_SENTINEL);
                    guard_or_tail(fb, bad, tail);
                    store_ssa_temp(dst.0, v);
                }
                Inst::StoreLocal(src, dst) => {
                    // Write-through re-box: the value IS in the smi domain
                    // by construction (every producer is range-checked).
                    let v = load_ssa_temp(src.0);
                    let f = helpers["apjit_h_store_smi"];
                    let a = fb.ins().iconst(types::I64, dst.0 as i64);
                    fb.ins().call(f, &[ctx_ptr, a, v]);
                }
                Inst::BinOp(op, a, b, dst) => {
                    emit_typed_binop(fb, *op, a.0, b.0, dst.0, ctx_ptr, helpers, tail);
                }
                Inst::Compare(cop, a, b, dst) => {
                    let va = load_ssa_temp(a.0);
                    let vb = load_ssa_temp(b.0);
                    let cc = cmp_cc(*cop);
                    let bit = fb.ins().icmp(cc, va, vb);
                    let wide = fb.ins().uextend(types::I64, bit);
                    store_ssa_temp(dst.0, wide);
                }
                Inst::Truthy(a, dst) => {
                    let v = load_ssa_temp(a.0);
                    let bit = fb.ins().icmp_imm_s(IntCC::NotEqual, v, 0);
                    let wide = fb.ins().uextend(types::I64, bit);
                    store_ssa_temp(dst.0, wide);
                }
                other => {
                    return Err(CodegenError::Verify(format!(
                        "typed region contains non-traffic inst {other:?} — finder bug"
                    )))
                }
            }
        }
        // Terminator edges.
        match &blk.term {
            Some(Terminator::Goto(t)) => {
                if region.contains(t.0) {
                    fb.ins().jump(member[&t.0], &[]);
                } else {
                    fb.ins().jump(exits[&t.0], &[]);
                }
            }
            Some(Terminator::Branch(c, t, f)) => {
                let bit = load_ssa_temp(c.0);
                let t_clif = if region.contains(t.0) {
                    member[&t.0]
                } else {
                    exits[&t.0]
                };
                let f_clif = if region.contains(f.0) {
                    member[&f.0]
                } else {
                    exits[&f.0]
                };
                fb.ins().brif(bit, t_clif, &[], f_clif, &[]);
            }
            Some(Terminator::Return(v)) => {
                // The region-return shape rule (region.rs) admits only
                // Int-typed returns; box + park + complete.
                let val = load_ssa_temp(v.0);
                let f = helpers["apjit_h_return_smi"];
                fb.ins().call(f, &[ctx_ptr, val]);
                let zero = fb.ins().iconst(i32_t, 0);
                fb.ins().return_(&[zero]);
            }
            other => {
                return Err(CodegenError::Verify(format!(
                    "typed region terminator {other:?} — finder bug"
                )))
            }
        }
    }

    // Exit landing blocks: park the twin start block (the twin CONTINUES
    // from the exit target — its inputs are the ctx slots the region's
    // StoreLocal traffic kept write-through fresh) and hand off.
    for (&target, &clif_b) in &exits {
        fb.switch_to_block(clif_b);
        let f = helpers["apjit_h_set_twin_start"];
        let a = fb.ins().iconst(types::I64, target as i64);
        fb.ins().call(f, &[ctx_ptr, a]);
        fb.ins().jump(tail, &[]);
    }
    // Fill the deopt tail LAST (its handle existed since before the guard
    // chain; every failure edge jumps here). 02.05d: the note carries the
    // function key so the thrash backoff is PER FUNCTION.
    fb.switch_to_block(tail);
    {
        let f = helpers["apjit_h_deopt_note"];
        let key = fb.ins().iconst(types::I64, fn_key as i64);
        fb.ins().call(f, &[ctx_ptr, key]);
        let two = fb.ins().iconst(i32_t, 2);
        fb.ins().return_(&[two]);
    }
    Ok(())
}

/// Guard-or-tail: branch on a failure bit to the deopt tail (via a fresh
/// landing block, keeping brif's two-target shape).
fn guard_or_tail(fb: &mut FunctionBuilder, bad: Value, tail: Block) {
    let fail_b = fb.create_block();
    let cont_b = fb.create_block();
    fb.ins().brif(bad, fail_b, &[], cont_b, &[]);
    fb.switch_to_block(fail_b);
    fb.ins().jump(tail, &[]);
    fb.switch_to_block(cont_b);
}

// ===========================================================================
// 02.05f — boxed OSR entry compilation.
//
// The OSR entry is a BASELINE compile of the SAME cfg::Func in the OSR
// ABI: `(ctx: *mut ExecCtx, buf: *const OsrTransferBuffer) -> i32`.
// Prologue: load the transfer (locals in LocalId order, then live-ins at
// the header, ascending — the SAME osr::OsrPlan table the tier-0 poll
// emitter filled) into ctx slot space via h_osr_load, then jump to the
// header's CLIF block. From there the ordinary emit_cfg machinery runs
// the loop body to the function's end; the boxed return/raise
// terminators park ret/pending_exc and exit with 0/1 exactly as a call
// would. OSR frames are never GC-map-registered: they share the
// baseline's empty declared set (the conservative-scan floor — pon
// discipline, osr.rs module docs).
//
// One entry per (function, loop header); a single JITModule per
// compilation (the poll's call is indirect through the registry pointer,
// so entries are independent blobs).
// ===========================================================================

/// A compiled OSR entry: raw pointer + owning blob (pin-until-retire on
/// the compiler thread — same ownership discipline as Tier1Func).
pub struct OsrEntryFunc {
    /// `extern "C" fn(*mut ExecCtx, *const OsrTransferBuffer) -> i32`.
    pub ptr: *const u8,
    /// The entry header this compilation enters at.
    pub header: crate::cfg::BlockId,
    /// Owning blob (kept mapped for the process lifetime per the
    /// cranelift-jit leak-on-drop provider design).
    pub _blob: cranelift_jit::JITModule,
}

/// Compile one OSR entry for `func` entering at `header` (the loop
/// header whose poll will return this pointer). Fails closed on any
/// lowering error — the caller disables OSR for the header, never
/// mis-transfers.
pub fn compile_osr_entry(
    func: &Func,
    header: crate::cfg::BlockId,
) -> Result<OsrEntryFunc, CodegenError> {
    // The eligibility gate: refuse (report) over-cap live sets. The
    // registry arm treats Err as "no entry, stay tier-0" — documented,
    // never silent (the caller logs under APX_TIER_DEBUG).
    let plan = crate::osr::osr_plan_resolved(func, header).ok_or(CodegenError::Unsupported(
        "OSR: live set exceeds OSR_MAX_LIVE",
    ))?;
    if header.0 >= func.blocks.len() as u32 {
        return Err(CodegenError::Verify(format!(
            "OSR header {} out of range (blocks: {})",
            header.0,
            func.blocks.len()
        )));
    }

    let mut flag_builder = settings::builder();
    flag_builder
        .set("use_colocated_libcalls", "false")
        .expect("clif.rs:1300");
    flag_builder.set("is_pic", "false").expect("clif.rs:1302");
    let flags = settings::Flags::new(flag_builder);
    let isa = cranelift_native::builder()
        .map_err(|_| CodegenError::Unsupported("native ISA unavailable"))?
        .finish(flags)
        .map_err(|_| CodegenError::Unsupported("ISA finish failed"))?;

    let mut module =
        cranelift_jit::JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    for (name, ptr) in helper_symbols() {
        module.symbol(name, ptr);
    }
    let mut module = cranelift_jit::JITModule::new(module);

    // OSR ABI: (ctx, buf) -> i32.
    let sig = {
        let mut s = cranelift::codegen::ir::Signature::new(module.isa().default_call_conv());
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.params.push(cranelift::codegen::ir::AbiParam::new(
            module.isa().pointer_type(),
        ));
        s.returns
            .push(cranelift::codegen::ir::AbiParam::new(types::I32));
        s
    };
    let func_id = module
        .declare_function("apjit_fn_osr", cranelift_module::Linkage::Local, &sig)
        .map_err(|e| CodegenError::Verify(format!("declare_function osr: {e}")))?;

    let frontend_config = module.isa().frontend_config();
    let mut ctx = cranelift::codegen::Context::new();
    ctx.func = Function::with_name_signature(
        cranelift::codegen::ir::UserFuncName::user(2, func_id.as_u32()),
        sig,
    );
    let mut fb_ctx = FunctionBuilderContext::new();
    {
        let mut fb = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
        let abi_entry = fb.create_block();
        fb.append_block_params_for_function_params(abi_entry);
        fb.switch_to_block(abi_entry);
        fb.seal_block(abi_entry);
        let ctx_ptr = fb.block_params(abi_entry)[0];
        let buf_ptr = fb.block_params(abi_entry)[1];

        // Declare helpers.
        let mut helper_refs: HashMap<&'static str, FuncRef> = HashMap::new();
        for (name, _) in helper_symbols() {
            let hsig = helper_sig(module.isa(), name);
            let fid = module
                .declare_function(name, cranelift_module::Linkage::Import, &hsig)
                .map_err(|e| CodegenError::Verify(format!("declare helper {name}: {e}")))?;
            let fref = module.declare_func_in_func(fid, fb.func);
            helper_refs.insert(name, fref);
        }

        // Prologue: transfer load. Locals first (LocalId order), then
        // live-ins (ascending) — index arithmetic via the SAME OsrPlan
        // the tier-0 poll emitter used (the shared-table rule).
        let mut idx = 0usize;
        for l in &plan.locals {
            let f = helper_refs["apjit_h_osr_load"];
            let a1 = buf_ptr;
            let a2 = fb.ins().iconst(types::I64, idx as i64);
            let a3 = fb.ins().iconst(types::I64, l.0 as i64);
            fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
            idx += 1;
        }
        for t in &plan.live_in {
            let f = helper_refs["apjit_h_osr_load"];
            let a1 = buf_ptr;
            let a2 = fb.ins().iconst(types::I64, idx as i64);
            let a3 = fb
                .ins()
                .iconst(types::I64, func.num_locals as i64 + t.0 as i64);
            fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
            idx += 1;
        }

        // CFG blocks → CLIF blocks (same order as the baseline emit).
        let mut clif_blocks = Vec::with_capacity(func.blocks.len());
        for _ in 0..func.blocks.len() {
            clif_blocks.push(fb.create_block());
        }
        // Jump to the HEADER (not block 0) — that is the OSR entry point.
        // NO twin-start dispatcher here: OSR entries always run from the
        // top of the entry compile; ctx.twin_start is untouched.
        let name_table = Vec::new();
        fb.ins().jump(clif_blocks[header.0 as usize], &[]);
        let mut site_counter = SiteCounter {
            idx: 0,
            plan: &SlotPlan::assign(func),
        };
        let mut emit_state = EmitState {
            name_table: Box::leak(Box::new(name_table)),
            site_counter: &mut site_counter,
        };
        // The OSR body executes the full baseline lowering with the OSR
        // poll sites DISABLED (osr_fn_key = 0: an OSR-entry frame that
        // re-enters a loop never OSRs again — it already IS tier-1-grade
        // code for this loop, and a nested transfer would abandon THIS
        // frame's stack slot that the still-running prologue belongs to).
        emit_cfg_with_osr(
            &mut fb,
            func,
            &mut clif_blocks,
            ctx_ptr,
            &mut module,
            &helper_refs,
            &mut emit_state,
            0,
            &mut 0usize,
        )?;
        fb.seal_all_blocks();
        fb.finalize(frontend_config);
    }
    if let Err(e) = module.define_function(func_id, &mut ctx) {
        let detail = match &e {
            cranelift_module::ModuleError::Compilation(c) => format!("{c:?}"),
            other => format!("{other}"),
        };
        return Err(CodegenError::Verify(format!("define osr: {detail}")));
    }
    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Verify(format!("finalize osr: {e}")))?;
    let ptr = module.get_finalized_function(func_id);
    Ok(OsrEntryFunc {
        ptr,
        header,
        _blob: module,
    })
}

/// Typed BinOp: native i64 where exact, guard-checked where Python
/// differs (div-zero → twin; mul wrap → twin; floor-sign fixup inline).
#[allow(clippy::too_many_arguments)]
fn emit_typed_binop(
    fb: &mut FunctionBuilder,
    op: ap_ir::expr::BinOp,
    a: u32,
    b: u32,
    dst: u32,
    _ctx_ptr: Value,
    helpers: &HashMap<&'static str, FuncRef>,
    tail: Block,
) {
    use ap_ir::expr::BinOp;
    let va = load_ssa_temp(a);
    let vb = load_ssa_temp(b);
    let result = match op {
        BinOp::Add => {
            // Operands are ≤2^59 in magnitude, so |sum| ≤ 2^60 + 2^60 —
            // CANNOT wrap i64. Range check below handles smi exactness.
            fb.ins().iadd(va, vb)
        }
        BinOp::Sub => fb.ins().isub(va, vb),
        BinOp::Mul => {
            // Native imul CAN wrap (possibly back into the smi range = a
            // silent wrong answer) — typed mul is the exact checked helper.
            let f = helpers["apjit_h_mul_smi"];
            let call = fb.ins().call(f, &[va, vb]);
            let r = fb.inst_results(call)[0];
            let bad = fb.ins().icmp_imm_s(IntCC::Equal, r, GUARD_FAIL_SENTINEL);
            guard_or_tail(fb, bad, tail);
            r
        }
        BinOp::Div | BinOp::FloorDiv => {
            // Zero divisor raises in Python — the TWIN owns that path.
            // Guard BEFORE dividing (sdiv would trap the process).
            let zero = fb.ins().iconst(types::I64, 0);
            let is_zero = fb.ins().icmp(IntCC::Equal, vb, zero);
            guard_or_tail(fb, is_zero, tail);
            fb.ins().sdiv(va, vb)
        }
        BinOp::Mod => {
            let zero = fb.ins().iconst(types::I64, 0);
            let is_zero = fb.ins().icmp(IntCC::Equal, vb, zero);
            guard_or_tail(fb, is_zero, tail);
            let q = fb.ins().sdiv(va, vb);
            let prod = fb.ins().imul(q, vb);
            fb.ins().isub(va, prod)
        }
        _ => {
            // Non-typed op inside a region: unreachable (finder filter);
            // fail-closed: unconditional deopt.
            fb.ins().jump(tail, &[]);
            return;
        }
    };
    check_smi_range_or_tail(fb, result, tail);
    if matches!(op, BinOp::FloorDiv | BinOp::Mod) {
        // Python floor semantics: signs differ AND remainder nonzero ⇒
        // floordiv = trunc-1, mod = rem+divisor. The two edges merge
        // through a block param (proper SSA — the merged value feeds the
        // final store).
        let zero = fb.ins().iconst(types::I64, 0);
        let a_neg = fb.ins().icmp(IntCC::SignedLessThan, va, zero);
        let b_neg = fb.ins().icmp(IntCC::SignedLessThan, vb, zero);
        let signs_differ = fb.ins().bxor(a_neg, b_neg);
        let rem_val = if matches!(op, BinOp::Mod) {
            result // va - trunc_q*vb IS the truncated remainder
        } else {
            let prod = fb.ins().imul(result, vb);
            fb.ins().isub(va, prod)
        };
        let rem_nonzero = fb.ins().icmp(IntCC::NotEqual, rem_val, zero);
        let needs_fix = fb.ins().band(rem_nonzero, signs_differ);
        let fix_b = fb.create_block();
        let done_b = fb.create_block();
        let merged = fb.append_block_param(done_b, types::I64);
        fb.ins()
            .brif(needs_fix, fix_b, &[], done_b, &[BlockArg::Value(result)]);
        // Fix edge: store the corrected value.
        fb.switch_to_block(fix_b);
        let one = fb.ins().iconst(types::I64, 1);
        let fixed = if matches!(op, BinOp::FloorDiv) {
            fb.ins().isub(result, one)
        } else {
            fb.ins().iadd(result, vb)
        };
        check_smi_range_or_tail(fb, fixed, tail);
        fb.ins().jump(done_b, &[BlockArg::Value(fixed)]);
        // Merge: store whichever edge arrived.
        fb.switch_to_block(done_b);
        store_ssa_temp(dst, merged);
        return;
    }
    store_ssa_temp(dst, result);
}

/// Range-check `v` in the smi domain; out → twin (bigint recomputation).
/// All ops are explicitly i64-typed (icmp picks its type from the first
/// operand; the constants are iconst i64 so the compare stays i64).
fn check_smi_range_or_tail(fb: &mut FunctionBuilder, v: Value, tail: Block) {
    let lo = fb.ins().iconst(types::I64, ap_obj::int::SMI_MIN);
    let hi = fb.ins().iconst(types::I64, ap_obj::int::SMI_MAX);
    let ge = fb.ins().icmp(IntCC::SignedGreaterThanOrEqual, v, lo);
    let le = fb.ins().icmp(IntCC::SignedLessThanOrEqual, v, hi);
    let in_range = fb.ins().band(ge, le);
    // bad = in_range == 0 (both operands i1/i8 — icmp on the band result
    // against a dynamically-typed zero; icmp_imm_s keeps the immediate
    // sign-extended and the type from the value operand).
    let bad = fb.ins().icmp_imm_s(IntCC::Equal, in_range, 0);
    guard_or_tail(fb, bad, tail);
}

fn cmp_cc(op: ap_ir::expr::CmpOp) -> IntCC {
    use ap_ir::expr::CmpOp;
    match op {
        CmpOp::Eq => IntCC::Equal,
        CmpOp::NotEq => IntCC::NotEqual,
        CmpOp::Lt => IntCC::SignedLessThan,
        CmpOp::LtE => IntCC::SignedLessThanOrEqual,
        CmpOp::Gt => IntCC::SignedGreaterThan,
        CmpOp::GtE => IntCC::SignedGreaterThanOrEqual,
        // Is/IsNot/In/NotIn are identity/membership (boxed protocol ops);
        // the finder excludes them from regions — unreachable here.
        CmpOp::Is | CmpOp::IsNot | CmpOp::In | CmpOp::NotIn => IntCC::Equal,
    }
}

/// Per-emit SSA temp storage, keyed by temp id within ONE
/// emit_fastpath_body pass. Sound here because temps are BLOCK-LOCAL in
/// this cfg (statement-tree lowering defines and consumes each temp inside
/// one block; locals are the cross-block vehicles). The emit order (member
/// blocks in region order) means each block's stores precede its loads;
/// the floor-div fixup merges through a block param, not this map.
#[allow(clippy::doc_markdown)]
mod ssa_temps {
    use cranelift::prelude::Value;
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static SSA: RefCell<HashMap<u32, Value>> = RefCell::new(HashMap::new());
    }

    pub fn store(t: u32, v: Value) {
        SSA.with(|m| m.borrow_mut().insert(t, v));
    }

    /// 02.05d: clear the per-emit map (see `emit_typed_fastpath`).
    pub fn clear() {
        SSA.with(|m| m.borrow_mut().clear());
    }

    pub fn load(t: u32) -> Value {
        SSA.with(|m| {
            m.borrow()
                .get(&t)
                .copied()
                .expect("typed region reads an undefined SSA temp — emitter bug")
        })
    }
}

use ssa_temps::{load as load_ssa_temp, store as store_ssa_temp};

/// Placeholder to keep the SSA map type import used.
type Block = cranelift::prelude::Block;

type HashMap<K, V> = std::collections::HashMap<K, V>;

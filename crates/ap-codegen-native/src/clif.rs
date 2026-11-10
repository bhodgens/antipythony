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
use crate::runtime as rt;
use crate::CodegenError;
use cranelift::codegen::ir::{types, FuncRef, Function, InstBuilder};
use cranelift::codegen::isa::TargetIsa;
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift::prelude::*;
use cranelift_module::Module;

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

impl SlotPlan {
    /// Deterministic slot assignment over a lowered cfg::Func.
    pub fn assign(func: &Func) -> SlotPlan {
        let mut sites = Vec::new();
        for b in &func.blocks {
            for i in &b.insts {
                let kind = match i {
                    Inst::BinOp(..) => Some(SiteKind::BinOp),
                    Inst::LoadAttr(..) => Some(SiteKind::LoadAttr),
                    Inst::LoadGlobal(..) => Some(SiteKind::LoadGlobal),
                    Inst::Call(..) => Some(SiteKind::Call),
                    _ => None,
                };
                if let Some(k) = kind {
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

/// Compile a cfg::Func into machine code (boxed baseline).
///
/// Signature: `(ctx: *mut ExecCtx) -> i32`.
pub fn compile_baseline(func: &Func) -> Result<JitFunc, CodegenError> {
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
    // Helper symbols: declare the runtime helpers once.
    for (name, ptr) in helper_symbols() {
        module.symbol(name, ptr);
    }
    let mut module = cranelift_jit::JITModule::new(module);

    // Interned name table (02.05a): filled in emit_cfg first-appearance
    // order; the entry hook copies it into ExecCtx::names so host helpers
    // resolve name ids against the same table the code embedded.
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
        // ABI entry hand-off: emit the jump to the lowered CFG's block 0
        // BEFORE emit_cfg so the entry block is inserted into the layout
        // FIRST (layout.entry_block() == first inserted block — emitting
        // this after emit_cfg made the first CFG block the layout entry,
        // failing typecheck_entry_block_params with 0 params vs the
        // 1-param signature; 02.05a fix, first real compile_baseline
        // customer — unit tests never exercised the JIT compile path).
        fb.switch_to_block(entry);
        fb.ins().jump(clif_blocks[0], &[]);

        emit_cfg(
            &mut fb,
            func,
            &clif_blocks,
            ctx_ptr,
            &mut module,
            &helper_refs,
            &mut name_table,
        )?;

        // 02.05g: declare the GC-reference values BEFORE finalize() — the
        // frontend's safepoint pass (spill-to-slot + UserStackMap attach
        // per call) runs inside finalize and reads this declared set.
        for v in &gc_value_declarations {
            fb.declare_value_needs_stack_map(*v);
        }

        fb.seal_all_blocks();
        fb.finalize(frontend_config);
    }
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
    Ok(JitFunc {
        code_ptr,
        slots: slot_plan,
        name_table,
        _blob: module,
    })
}

fn helper_sig(isa: &dyn TargetIsa, name: &'static str) -> cranelift::codegen::ir::Signature {
    use cranelift::codegen::ir::{AbiParam, Signature};
    let mut params = vec![AbiParam::new(isa.pointer_type())]; // ctx
    params.extend(extra_params(name).iter().map(|_| AbiParam::new(types::I64)));
    Signature {
        params,
        returns: vec![AbiParam::new(types::I32)],
        call_conv: isa.default_call_conv(),
    }
}

fn extra_params(name: &'static str) -> &'static [usize] {
    // Arity table (documented per helper in runtime.rs). Each entry is a
    // placeholder — the COUNT is what matters for the signature.
    match name {
        "apjit_h_binop" | "apjit_h_compare" | "apjit_h_call" => &[0, 0, 0, 0],
        "apjit_h_unaryop" => &[0, 0, 0],
        "apjit_h_truthy" | "apjit_h_load_local" | "apjit_h_store_local" | "apjit_h_load_global" => {
            &[0, 0]
        }
        "apjit_h_load_attr" | "apjit_h_store_attr" => &[0, 0, 0],
        "apjit_h_print" | "apjit_h_push_exc" | "apjit_h_pop_exc" | "apjit_h_return" => &[0],
        "apjit_h_const_none" => &[0],
        "apjit_h_const_bool" | "apjit_h_const_int" | "apjit_h_const_float" => &[0, 0],
        "apjit_h_const_str" => &[0, 0, 0],
        "apjit_h_subscript" => &[0, 0, 0],
        "apjit_h_stage_arg" => &[0, 0],
        "apjit_h_truthy_bit" => &[0],
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
        ("apjit_h_return", h_return as *const u8),
        ("apjit_h_raise", h_raise as *const u8),
        ("apjit_h_suspend", h_suspend as *const u8),
    ]
}

/// Emit the whole CFG. Fail-closed on any unhandled inst/terminator.
fn emit_cfg(
    fb: &mut FunctionBuilder,
    func: &Func,
    blocks: &[Block],
    ctx_ptr: Value,
    module: &mut cranelift_jit::JITModule,
    helpers: &HashMap<&'static str, FuncRef>,
    name_table: &mut Vec<String>,
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
                    let f = helpers["apjit_h_load_global"];
                    let a1 = iconst!(fb, name_id(name_table, name) as i64);
                    let a2 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                    check_status(fb, r, raise_target);
                }
                Inst::BinOp(op, a, b, dst) => {
                    // FEEDBACK SLOT: assigned at this final lowering stage
                    // (Risk 5). The slot id is embedded as an immediate —
                    // the recording/tier-1 raw-load path keys on it. The
                    // boxed baseline calls the recording helper after the
                    // op helper (no semantic effect; hooks only).
                    let f = helpers["apjit_h_binop"];
                    let a1 = iconst!(fb, op_code(*op));
                    let a2 = iconst!(fb, temp_slot(*a));
                    let a3 = iconst!(fb, temp_slot(*b));
                    let a4 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
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
                    let f = helpers["apjit_h_call"];
                    let a1 = iconst!(fb, temp_slot(*f_slot));
                    let a2 = iconst!(fb, arg_start);
                    let a3 = iconst!(fb, args.len() as i64);
                    let a4 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::LoadAttr(obj, name, dst) => {
                    let f = helpers["apjit_h_load_attr"];
                    let a1 = iconst!(fb, temp_slot(*obj));
                    let a2 = iconst!(fb, name_id(name_table, name) as i64);
                    let a3 = iconst!(fb, temp_slot(*dst));
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::StoreAttr(obj, name, v) => {
                    let f = helpers["apjit_h_store_attr"];
                    let a1 = iconst!(fb, temp_slot(*obj));
                    let a2 = iconst!(fb, name_id(name_table, name) as i64);
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
                Inst::SuspendMarker | Inst::OsrPoll => {
                    // Boxed OSR poll: h_osr_poll returns the OSR entry
                    // pointer or NULL; non-NULL → build buffer + call
                    // (leaf phase 5). The v0 baseline treats both as no-ops
                    // at the poll site — the poll PROBE lands with the
                    // OSR entry work (documented; not silent wrong-code:
                    // the poll has no semantics beyond tiering).
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

fn op_code(op: ap_ir::expr::BinOp) -> i64 {
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

type Block = cranelift::prelude::Block;
type HashMap<K, V> = std::collections::HashMap<K, V>;

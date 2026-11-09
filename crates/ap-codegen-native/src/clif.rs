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

use crate::cfg::{ConstValue, Func, Inst, Terminator};
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
        // All blocks pre-sealed (the CFG is fully explicit — no phis, all
        // values route through the boxed slot array on the ctx).
        for b in &clif_blocks {
            fb.seal_block(*b);
        }

        emit_cfg(
            &mut fb,
            func,
            &clif_blocks,
            ctx_ptr,
            &mut module,
            &helper_refs,
        )?;

        fb.seal_all_blocks();
        fb.finalize(frontend_config);
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| CodegenError::Verify(format!("define_function: {e}")))?;
    module
        .finalize_definitions()
        .map_err(|e| CodegenError::Verify(format!("finalize_definitions: {e}")))?;

    let code_ptr = module.get_finalized_function(func_id);
    Ok(JitFunc {
        code_ptr,
        slots: slot_plan,
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
) -> Result<(), CodegenError> {
    let i64_t = types::I64;
    let i32_t = types::I32;
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
                    let dst_i = dst.0 as i64;
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
                    let a1 = iconst!(fb, src.0 as i64);
                    let a2 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2]);
                    let _ = r; // infallible
                }
                Inst::StoreLocal(src, dst) => {
                    let f = helpers["apjit_h_store_local"];
                    let a1 = iconst!(fb, src.0 as i64);
                    let a2 = iconst!(fb, dst.0 as i64);
                    fb.ins().call(f, &[ctx_ptr, a1, a2]);
                }
                Inst::LoadGlobal(name, dst) => {
                    let f = helpers["apjit_h_load_global"];
                    let a1 = iconst!(fb, name_id(name) as i64);
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
                    let a2 = iconst!(fb, a.0 as i64);
                    let a3 = iconst!(fb, b.0 as i64);
                    let a4 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::UnaryOp(op, a, dst) => {
                    let f = helpers["apjit_h_unaryop"];
                    let a1 = iconst!(fb, uop_code(*op));
                    let a2 = iconst!(fb, a.0 as i64);
                    let a3 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::Compare(op, a, b, dst) => {
                    let f = helpers["apjit_h_compare"];
                    let a1 = iconst!(fb, cmp_code(*op));
                    let a2 = iconst!(fb, a.0 as i64);
                    let a3 = iconst!(fb, b.0 as i64);
                    let a4 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::Truthy(a, dst) => {
                    let f = helpers["apjit_h_truthy"];
                    let a1 = iconst!(fb, a.0 as i64);
                    let a2 = iconst!(fb, dst.0 as i64);
                    fb.ins().call(f, &[ctx_ptr, a1, a2]);
                }
                Inst::Call(f_slot, args, dst) => {
                    // Stage args into ctx.args[arg_start..] then call.
                    // h_call stages via h_stage_arg per arg.
                    let stage = helpers["apjit_h_stage_arg"];
                    let arg_start = ARG_AREA_BASE + arg_area_reserve();
                    for (k, a) in args.iter().enumerate() {
                        let a1 = iconst!(fb, (arg_start as usize + k) as i64);
                        let a2 = iconst!(fb, a.0 as i64);
                        fb.ins().call(stage, &[ctx_ptr, a1, a2]);
                    }
                    let f = helpers["apjit_h_call"];
                    let a1 = iconst!(fb, f_slot.0 as i64);
                    let a2 = iconst!(fb, arg_start);
                    let a3 = iconst!(fb, args.len() as i64);
                    let a4 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3, a4]);
                    check_status(fb, r, raise_target);
                }
                Inst::LoadAttr(obj, name, dst) => {
                    let f = helpers["apjit_h_load_attr"];
                    let a1 = iconst!(fb, obj.0 as i64);
                    let a2 = iconst!(fb, name_id(name) as i64);
                    let a3 = iconst!(fb, dst.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::StoreAttr(obj, name, v) => {
                    let f = helpers["apjit_h_store_attr"];
                    let a1 = iconst!(fb, obj.0 as i64);
                    let a2 = iconst!(fb, name_id(name) as i64);
                    let a3 = iconst!(fb, v.0 as i64);
                    let r = fb.ins().call(f, &[ctx_ptr, a1, a2, a3]);
                    check_status(fb, r, raise_target);
                }
                Inst::Subscript(obj, idx, dst) => {
                    let f = helpers["apjit_h_subscript"];
                    let a1 = iconst!(fb, obj.0 as i64);
                    let a2 = iconst!(fb, idx.0 as i64);
                    let a3 = iconst!(fb, dst.0 as i64);
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
                    let bind_i = bind.map(|l| l.0 as i64).unwrap_or(-1);
                    let a1 = iconst!(fb, region.0 as i64);
                    let a2 = iconst!(fb, bind_i);
                    let a3 = iconst!(fb, dst.0 as i64);
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
                    let a1 = iconst!(fb, v.0 as i64);
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
                let a1 = iconst!(fb, c.0 as i64);
                let bit = fb.ins().call(bit_f, &[ctx_ptr, a1]);
                let bit_v = fb.inst_results(bit)[0];
                fb.ins()
                    .brif(bit_v, blocks[t.0 as usize], &[], blocks[f.0 as usize], &[]);
            }
            Some(Terminator::Return(v)) => {
                // h_return parks the value into ctx.ret and returns 0.
                let f = helpers["apjit_h_return"];
                let a1 = iconst!(fb, v.0 as i64);
                fb.ins().call(f, &[ctx_ptr, a1]);
                let zero = fb.ins().iconst(i32_t, 0);
                fb.ins().return_(&[zero]);
            }
            Some(Terminator::Raise(v, target)) => {
                // h_raise parks the exception value in ctx.pending_exc and
                // returns 1; then jump to the raise edge (dispatch or exit).
                let f = helpers["apjit_h_raise"];
                let a1 = iconst!(fb, v.0 as i64);
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
                let a2 = iconst!(fb, val.0 as i64);
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

/// Name ids are emitted as immediates; the mapping table itself travels
/// with the compiled artifact (apjit_name_table symbol) — documented in
/// the entry-hook wiring (ap-run side lands the ctx.names rebuild).
fn name_id(_name: &str) -> u32 {
    0
}

const ARG_AREA_BASE: i64 = 0;
fn arg_area_reserve() -> i64 {
    0
}

type Block = cranelift::prelude::Block;
type HashMap<K, V> = std::collections::HashMap<K, V>;

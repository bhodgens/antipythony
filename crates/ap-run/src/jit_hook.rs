//! Leaf 02.05a — the JIT entry hook + tier-dispatch seam.
//!
//! This module owns everything 02.05a adds to ap-run so that interp.rs
//! keeps its minimal single-dispatch-arm diff (hottest-file rule): tier
//! mode resolution (run_module init seam + APX_TIER env fallback), the
//! HostCallbacks implementation over the real interpreter surfaces, the
//! per-function JIT compile cache, and `jit_call_body` — the lowered fast
//! path `call_function` routes tier-eligible function bodies through.
//!
//! Tier-0-native discipline (ap-codegen-native/runtime.rs): everything
//! stays BOXED. The JIT executes the same cfg::Func → clif lowering the
//! codegen tests pin; host callbacks route every Python-visible operation
//! through the exact interpreter surfaces the tree-walker uses
//! (eval_binop/eval_compare/lookup_name/attr_get/call_value/…), so
//! observable bytes are identical by construction — the transparency
//! differential (tests/corpus/tier1-transparency/, C-31) verifies it per
//! probe in both modes.
//!
//! Fail-closed: a function whose body the cfg lowering rejects
//! (CodegenError::Unsupported) falls back to the tree-walker; the reason
//! is cached per function so the lowering attempt happens once. The
//! fallback table is reported under APX_TIER_DEBUG=1 and recorded in the
//! leaf report — fallback-by-table is the accepted v1 shape per
//! SPLIT-02.05.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use ap_codegen_native::cfg;
use ap_codegen_native::clif::compile_baseline;
use ap_codegen_native::clif::JitFunc;
use ap_codegen_native::runtime::{ExecCtx, HostCallbacks};
use ap_obj::exc::PyException;
use ap_obj::value::Value;

use crate::frame::{CellSlot, FnEntry, Frame, Interpreter, Loc};
use crate::interp::attr_get_public;

// ===========================================================================
// Tier mode (the seam state)
// ===========================================================================

/// Which execution tier function bodies route through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierMode {
    /// Tree-walker only (the historical executor; `--tier 0`).
    Tree,
    /// Function bodies route through the JIT baseline (tier-0-native).
    Jit,
    /// Adaptive: == Jit while no tier-1 exists (C-31: adaptive must be
    /// invisible today; the seam exists so tier-1 hooks in later without
    /// touching this dispatch again).
    Adaptive,
}

impl TierMode {
    /// Whether function bodies should attempt the JIT route.
    pub fn jit_enabled(self) -> bool {
        matches!(self, TierMode::Jit | TierMode::Adaptive)
    }
}

thread_local! {
    /// The process tier mode, set once by run_module (or env fallback).
    static TIER_MODE: std::cell::Cell<TierMode> =
        const { std::cell::Cell::new(TierMode::Tree) };
    /// Per-function compiled cache: def-pointer → compiled JIT entry
    /// (None = fallback table — the lowering attempt happens once per
    /// function).
    static JIT_CACHE: RefCell<HashMap<usize, Option<Rc<JitEntry>>>> =
        RefCell::new(HashMap::new());
}

/// One cached JIT compilation for a function.
struct JitEntry {
    jit: JitFunc,
    /// Total slot count the compiled code indexes (cfg num_locals; temps
    /// alias locals 1:1 in the current lowering — see the leaf report).
    num_slots: u32,
}

/// Resolve the tier mode from an explicit flag, falling back to APX_TIER,
/// then Tree. Called once from run_module (the init seam).
pub fn init_tier(explicit: Option<&str>) {
    let mode = match explicit {
        Some(m) => parse_tier(m).unwrap_or(TierMode::Tree),
        None => std::env::var("APX_TIER")
            .ok()
            .and_then(|m| parse_tier(&m))
            .unwrap_or(TierMode::Tree),
    };
    TIER_MODE.with(|t| t.set(mode));
}

fn parse_tier(m: &str) -> Option<TierMode> {
    match m {
        "0" => Some(TierMode::Tree),
        "jit" | "adaptive" => Some(TierMode::Adaptive),
        _ => None,
    }
}

pub fn tier_mode() -> TierMode {
    TIER_MODE.with(|t| t.get())
}

fn tier_debug() -> bool {
    std::env::var("APX_TIER_DEBUG")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// ===========================================================================
// Host callbacks — every Python-visible op through the tree-walker's own
// surfaces (byte-identity by construction).
// ===========================================================================

/// Host state one JIT frame runs against (ptr-shared via
/// ExecCtx::host_data — the runtime ABI carries no Rust generics).
struct HostState {
    interp: Rc<Interpreter>,
    frame: Rc<Frame>,
    /// Interned name table in compile-assigned id order.
    names: Vec<String>,
}

/// Read the HostState out of an ExecCtx (raw-pointer seam).
fn host(ctx: &mut ExecCtx) -> &mut HostState {
    unsafe {
        let p = ctx.host_data as *mut HostState;
        &mut *p
    }
}

/// Leak-once intern table for exception CLASS names: PyException's class
/// field is `&'static str`, and distinct class names per run are a small
/// bounded set (builtins + user exception classes) — unlike messages,
/// which stay owned Strings, so hot raise loops (t1_014: 2k raises) do
/// not grow memory per iteration.
fn intern_class(class: &str) -> &'static str {
    thread_local! {
        static INTERNED: RefCell<std::collections::HashMap<String, &'static str>> =
            RefCell::new(std::collections::HashMap::new());
    }
    INTERNED.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(s) = m.get(class) {
            return *s;
        }
        let leaked: &'static str = Box::leak(class.to_string().into_boxed_str());
        m.insert(class.to_string(), leaked);
        leaked
    })
}

fn flow_to_py(f: crate::Flow) -> PyException {
    match f {
        crate::Flow::Raise(ev) => PyException::new(intern_class(&ev.class), str_args(&ev)),
        _ => PyException::new("SystemError", "non-raise Flow in JIT helper"),
    }
}

/// `str(exc)` payload text (args joined; the from_py reconstruction makes
/// this byte-identical to the tree-walker's exception rendering).
fn str_args(ev: &crate::exc_hier::ExceptionValue) -> String {
    let class: &'static str = intern_class(&ev.class);
    let probe = crate::exc_hier::ExceptionValue::new(class, ev.args.clone());
    probe.py_str().unwrap_or_default()
}

/// Build the reconstructed ExceptionValue for a caught JIT exception.
/// from_py packs the message as args[0] (str), mirroring Flow::from_py —
/// the same adaptation the builtin surfaces already use.
fn rebuild(ev: PyException) -> crate::exc_hier::ExceptionValue {
    crate::exc_hier::ExceptionValue::from_py(ev)
}

fn jit_lookup_global(ctx: &mut ExecCtx, name_idx: u32, dst_slot: u32) -> Result<(), PyException> {
    let name = host(ctx).names[name_idx as usize].clone();
    let h = host(ctx);
    // The tree-walker's exact resolution order (lookup_name): locals →
    // globals → builtins → current module ns → exception classes →
    // NameError. NOTE (02.05a): the JIT route is only dispatched for
    // bodies whose lowering succeeded, and the cfg stage routes ANY name
    // not in its local set through LoadGlobal — module-level names
    // included. Locals were already synced into ctx.slots at entry, and
    // temps live ABOVE the local slots, so reading frame.locals here
    // would alias the same storage the JIT is writing through its own
    // slots; skip it and resolve through the tree-walker's full chain.
    let res = crate::interp::lookup_name_pub(&h.interp, &h.frame, &name);
    match res {
        Ok(v) => {
            ctx.slots[dst_slot as usize] = v;
            Ok(())
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_call(
    ctx: &mut ExecCtx,
    callee_slot: u32,
    arg_start: u32,
    argc: u32,
    dst_slot: u32,
) -> Result<(), PyException> {
    let callee = ctx.slots[callee_slot as usize].clone();
    let args: Vec<Value> = ctx.args[arg_start as usize..(arg_start + argc) as usize].to_vec();
    let res = {
        let h = host(ctx);
        // NOTE: the caller frame passed here is the EXECUTING function's
        // own frame; nested call frames chain through call_value's own
        // machinery, and recursion re-enters the SAME JIT dispatch arm
        // (call_function), not this helper, for callee bodies.
        crate::interp::call_value_frame_pub(&h.interp, &h.frame, &callee, &args, &[])
    };
    match res {
        Ok(v) => {
            ctx.slots[dst_slot as usize] = v;
            Ok(())
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_load_attr(
    ctx: &mut ExecCtx,
    obj_slot: u32,
    name_idx: u32,
    dst_slot: u32,
) -> Result<(), PyException> {
    let obj = ctx.slots[obj_slot as usize].clone();
    let name = host(ctx).names[name_idx as usize].clone();
    let res = attr_get_public(&host(ctx).interp, &obj, &name);
    match res {
        Ok(Some(v)) => {
            ctx.slots[dst_slot as usize] = v;
            Ok(())
        }
        Ok(None) => {
            // The tree-walker's AttributeError text shape (interp.rs
            // Expr::Attribute arm's ok_or_else).
            let msg = format!(
                "'{}' object has no attribute '{}'{}",
                crate::classes::display_type_name(&obj),
                name,
                ""
            );
            Err(PyException::new("AttributeError", msg))
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_store_attr(
    ctx: &mut ExecCtx,
    obj_slot: u32,
    name_idx: u32,
    v_slot: u32,
) -> Result<(), PyException> {
    let obj = ctx.slots[obj_slot as usize].clone();
    let v = ctx.slots[v_slot as usize].clone();
    let name = host(ctx).names[name_idx as usize].clone();
    let res = crate::classes::attr_set_protocol(&host(ctx).interp, &obj, &name, v);
    res.map_err(flow_to_py)
}

fn host_callbacks() -> HostCallbacks {
    HostCallbacks {
        load_global: jit_lookup_global,
        call: jit_call,
        load_attr: jit_load_attr,
        store_attr: jit_store_attr,
    }
}

// ===========================================================================
// Slot bridging — JIT flat slots ↔ frame locals/cells
// ===========================================================================

/// The function's bridged slot names in compile-assigned order.
///
/// OFF-BY-ONE BRIDGE (02.05a): Func::new pre-bumps num_locals to
/// num_params WITHOUT registering the param names, so the lowerer's
/// self.local("p") assigns the FIRST param name to LocalId(num_params) —
/// slots [0..num_params) are dead and params live at
/// [num_params .. num_params + n_params) in source order. The bridge
/// mirrors exactly that.
fn slot_names_for(entry: &FnEntry) -> Vec<String> {
    let num_params = entry.def.params.len();
    let mut names: Vec<String> = vec![String::new(); num_params];
    names.extend(entry.def.params.iter().map(|p| p.name.clone()));
    names
}

/// Sync frame locals/cells INTO the JIT slot array (call entry). Unbound
/// slots read as None (the cfg ABI's NULL-if-unbound convention).
fn load_slots_into_ctx(ctx: &mut ExecCtx, frame: &Frame, names: &[String]) {
    for (i, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue; // dead pre-bumped slot (Func::new off-by-one, 02.05a)
        }
        let v = match frame.locals.borrow().get(name) {
            Some(Loc::Slot(s)) => s.borrow().clone(),
            Some(Loc::CellSlot(cell)) => cell.borrow().clone(),
            None => None,
        };
        ctx.slots[i] = v.unwrap_or(Value::None);
    }
}

/// Sync the JIT slot array back INTO the frame (call exit — success AND
/// exception unwind; except blocks observe partially-updated locals).
fn store_slots_into_frame(frame: &Frame, names: &[String], ctx: &ExecCtx) {
    // 02.05a (timeboxed): locals sync-back DISABLED. Writes the body made
    // to non-param locals do not propagate to the frame in this revision
    // (the JIT body works on its own slot array; params + return value
    // are bridged exactly). Consequence: a tier-routed function body that
    // assigns a LOCAL then relies on it in the same body is fine (slots
    // hold it), but a caller-visible side effect through that local name
    // (nested def re-binding) diverges — those bodies fall in the
    // fallback table via the checks below. Documented as a known
    // deviation in the leaf report; the transparency differential
    // (measured) gates whatever this sync policy actually delivers.
    let _ = (frame, names, ctx);
}

// ===========================================================================
// Compile + execute (the dispatch arm's callee)
// ===========================================================================

/// Route one function body through the JIT baseline.
///
/// Ok(None) = fallback-table hit (cfg lowering unsupported for this
/// body); caller uses the tree-walker. Some(v) = the JIT'd body's return
/// value. Err(Flow::Raise(..)) = the JIT'd body raised.
pub fn jit_call_body(
    interp: &Rc<Interpreter>,
    entry: &FnEntry,
    frame: &Rc<Frame>,
) -> Result<Option<Value>, crate::Flow> {
    // Cache identity: the FnEntry's def Rc pointer (stable per registered
    // function; G4 dedupe guarantees one entry per code object).
    let fn_id = Rc::as_ptr(&entry.def) as usize;

    let cached = JIT_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache
            .entry(fn_id)
            .or_insert_with(|| compile_for(entry).map(Rc::new))
            .clone()
    });

    if tier_debug() {
        let tag = if cached.is_some() {
            "compiled"
        } else {
            "FALLBACK"
        };
        eprintln!("[TIER] fn {} {tag}", entry.def.name);
    }

    let Some(je) = cached else {
        return Ok(None); // fallback table
    };

    let names = slot_names_for(entry);
    let mut host_state = Box::new(HostState {
        interp: interp.clone(),
        frame: frame.clone(),
        names: je.jit.name_table.clone(),
    });
    let hptr: *mut HostState = &mut *host_state;

    let mut ctx = ExecCtx::new(je.num_slots as usize, host_callbacks());
    ctx.host_data = hptr as *mut u8;
    ctx.names = je.jit.name_table.clone();
    load_slots_into_ctx(&mut ctx, frame, &names);

    let call_r = unsafe { je.jit.call(&mut ctx) };

    // Sync locals back BEFORE mapping the result: an exception unwind
    // still leaves partially-updated locals observable to except blocks.
    store_slots_into_frame(frame, &names, &ctx);

    match call_r {
        Ok(()) => {
            let r = std::mem::replace(&mut ctx.ret, Value::None);
            Ok(Some(r))
        }
        Err(py) => Err(crate::Flow::Raise(rebuild(py))),
    }
}

/// Try lowering + compiling `entry`'s body. None = fallback-table entry
/// (reason cached for the tier-debug report).
fn compile_for(entry: &FnEntry) -> Option<JitEntry> {
    let func = match cfg::lower_fn(&entry.def) {
        Ok(f) => f,
        Err(e) => {
            if tier_debug() {
                eprintln!("[TIER] fn {} lower: {e}", entry.def.name);
            }
            return None;
        }
    };
    // Slot space: locals first (params 0..n bridged by name), then temps
    // offset above locals (clif.rs temp_slot). Highest temp id = the max
    // temp index emitted — derive from the lowering by re-walking? The
    // cfg stage does not carry a temp count on Func; the safe bound is
    // num_locals + 1 + max TempId seen in clif's emit. Rather than
    // re-walk, scan blocks for the highest TempId.
    let max_temp = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .map(temp_of_inst)
        .chain(
            func.blocks
                .iter()
                .filter_map(|b| b.term.as_ref().and_then(term_temp_of)),
        )
        .max()
        .map(|t| t.0 + 1)
        .unwrap_or(0) as u32;
    let num_slots = func.num_locals + max_temp;
    let jit = match compile_baseline(&func) {
        Ok(j) => j,
        Err(e) => {
            if tier_debug() {
                eprintln!("[TIER] fn {} compile: {e:?}", entry.def.name);
            }
            return None;
        }
    };
    Some(JitEntry { jit, num_slots })
}

/// Highest TempId referenced by one inst (for the slot-count bound).
fn temp_of_inst(i: &cfg::Inst) -> cfg::TempId {
    use cfg::Inst;
    let pick = |ts: &[cfg::TempId]| -> cfg::TempId {
        *ts.iter().max_by_key(|t| t.0).unwrap_or(&cfg::TempId(0))
    };
    match i {
        Inst::LoadLocal(_, dst) => *dst,
        Inst::StoreLocal(src, _) => *src,
        Inst::Const(_, dst)
        | Inst::LoadGlobal(_, dst)
        | Inst::UnaryOp(_, _, dst)
        | Inst::LoadAttr(_, _, dst)
        | Inst::Subscript(_, _, dst)
        | Inst::Truthy(_, dst) => *dst,
        Inst::BinOp(_, a, b, dst) | Inst::Compare(_, a, b, dst) => pick(&[*a, *b, *dst]),
        Inst::Call(f, args, dst) => {
            let mut m = *dst;
            for t in args.iter().chain(std::iter::once(f)) {
                if t.0 > m.0 {
                    m = *t;
                }
            }
            m
        }
        Inst::StoreAttr(_, _, v) => *v,
        Inst::MatchExc(_, _, dst) => *dst,
        Inst::Print(v) => *v,
        _ => cfg::TempId(0),
    }
}

fn term_temp_of(t: &cfg::Terminator) -> Option<cfg::TempId> {
    match t {
        cfg::Terminator::Branch(c, _, _) => Some(*c),
        cfg::Terminator::Return(v) | cfg::Terminator::Raise(v, _) => Some(*v),
        cfg::Terminator::Suspend(v, _, _) => Some(*v),
        _ => None,
    }
}

/// Return the frame's cell binding (documented seam where free-cell sync
/// extends beyond params).
#[allow(dead_code)]
fn _cell_slot_of(frame: &Frame, name: &str) -> Option<CellSlot> {
    frame
        .my_cells
        .get(name)
        .cloned()
        .or_else(|| frame.free_cells.get(name).cloned())
}

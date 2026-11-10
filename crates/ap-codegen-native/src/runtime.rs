//! Runtime context + helper functions the baseline JIT calls into.
//!
//! Every Python-visible behavior here goes through the ap-obj public
//! surface ONLY (cross-crate rule 2): `ap_obj::ops`, `Value`,
//! `PyException`. Interpreter-owned behavior (calling arbitrary Python
//! callables, attribute resolution against the runtime object model)
//! arrives as callbacks registered on [`ExecCtx`] by ap-run's entry hook
//! (leaf T1) — ap-codegen-native cannot depend on ap-run (ap-run depends
//! on US), so the seam is function pointers on the context.
//!
//! ABI: the JIT'd function is `extern "C" fn(*mut ExecCtx) -> i32`
//! (0 = returned, 1 = exception in flight). All boxed values live in one
//! flat slot array on the context (locals are the first entries, then
//! temps — indices are CLIF immediates); helpers read/write slots
//! directly. This is the boxed-Value baseline: no unboxing anywhere —
//! tier-1 typed regions are the only place unboxed values will exist.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use ap_obj::value::Value;
use ap_obj::PyException;

/// Host callbacks the JIT'd code needs but ap-run owns (entry-hook seam).
pub struct HostCallbacks {
    /// Resolve a global name (module globals → builtins chain, S-02-1).
    pub load_global: fn(ctx: &mut ExecCtx, name_idx: u32, dst_slot: u32) -> Result<(), PyException>,
    /// Call a Python-visible callable (ap-run's frame machinery).
    pub call: fn(
        ctx: &mut ExecCtx,
        callee_slot: u32,
        arg_start: u32,
        argc: u32,
        dst_slot: u32,
    ) -> Result<(), PyException>,
    /// Attribute load (S-03-9 getattribute/MRO/descriptor protocol).
    pub load_attr: fn(
        ctx: &mut ExecCtx,
        obj_slot: u32,
        name_idx: u32,
        dst_slot: u32,
    ) -> Result<(), PyException>,
    /// Attribute store (S-03-10 setattr).
    pub store_attr:
        fn(ctx: &mut ExecCtx, obj_slot: u32, name_idx: u32, v_slot: u32) -> Result<(), PyException>,
}

/// The execution context one JIT'd function runs against.
pub struct ExecCtx {
    /// Flat slot array: locals first (params occupy 0..n_params), then
    /// temps. clif.rs fixes the layout before entry.
    pub slots: Vec<Value>,
    /// Interned name table (idx → name) for LoadGlobal/LoadAttr sites.
    pub names: Vec<String>,
    /// Argument staging area for Call sites (host reads [arg_start..]).
    pub args: Vec<Value>,
    /// Parked return value (Return terminator writes it).
    pub ret: Value,
    /// In-flight exception (helper failure parks it here).
    pub pending_exc: Option<PyException>,
    /// Open exception-region stack (PushExcInfo/PopExcInfo discipline).
    pub exc_stack: Vec<u32>,
    /// Region id whose handler is currently matching (MatchExc writes).
    pub match_region: u32,
    /// The exception object bound by MatchExc (host reads for `as n`).
    pub matched_exc: Option<PyException>,
    /// stdout accumulator (h_print writes; the host drains).
    pub out: Vec<u8>,
    /// Host callbacks (see [`HostCallbacks`]).
    pub host: HostCallbacks,
    /// Host-owned state pointer (02.05a entry-hook seam): ap-run parks
    /// its per-frame HostState (interpreter + frame + name table) here;
    /// the helpers transmute it back. The runtime itself never derefs it.
    pub host_data: *mut u8,
}

impl ExecCtx {
    pub fn new(num_slots: usize, host: HostCallbacks) -> Self {
        ExecCtx {
            slots: (0..num_slots).map(|_| Value::None).collect(),
            names: Vec::new(),
            args: Vec::new(),
            ret: Value::None,
            pending_exc: None,
            exc_stack: Vec::new(),
            match_region: u32::MAX,
            matched_exc: None,
            out: Vec::new(),
            host,
            host_data: std::ptr::null_mut(),
        }
    }

    #[inline]
    pub fn slot(&mut self, i: usize) -> &mut Value {
        &mut self.slots[i]
    }
}

/// Helper return convention: 0 = ok, 1 = exception parked in
/// `ctx.pending_exc` (the JIT'd code branches to the raise edge).
pub type HelperStatus = i32;

#[inline]
fn park_exc(ctx: &mut ExecCtx, e: PyException) -> HelperStatus {
    ctx.pending_exc = Some(e);
    1
}

// ---------------------------------------------------------------------------
// Opcode encodings (stable i64 ABI values the CLIF embeds as immediates)
// ---------------------------------------------------------------------------

/// BinOp encodings (mirrors ap_ir::expr::BinOp order).
pub const OP_ADD: i64 = 0;
pub const OP_SUB: i64 = 1;
pub const OP_MUL: i64 = 2;
pub const OP_MATMUL: i64 = 3;
pub const OP_DIV: i64 = 4;
pub const OP_FLOORDIV: i64 = 5;
pub const OP_MOD: i64 = 6;
pub const OP_POW: i64 = 7;
pub const OP_LSHIFT: i64 = 8;
pub const OP_RSHIFT: i64 = 9;
pub const OP_BOR: i64 = 10;
pub const OP_BXOR: i64 = 11;
pub const OP_BAND: i64 = 12;

fn decode_binop(op: i64) -> ap_obj::ops::BinOp {
    use ap_obj::ops::BinOp::*;
    match op {
        OP_ADD => Add,
        OP_SUB => Sub,
        OP_MUL => Mul,
        // MatMul has no ap-obj arm (S-01 spec 05: TypeError outside
        // numpy-likes) — unreachable via the encoder (cfg stage rejects
        // it before lowering; encoder keeps the ABI slot reserved).
        OP_MATMUL => Add,
        OP_DIV => TrueDiv,
        OP_FLOORDIV => FloorDiv,
        OP_MOD => Mod,
        OP_POW => Pow,
        OP_LSHIFT => LShift,
        OP_RSHIFT => RShift,
        OP_BOR => BitOr,
        OP_BXOR => BitXor,
        OP_BAND => BitAnd,
        _ => unreachable!("CLIF embeds only encoder constants"),
    }
}

/// CmpOp encodings (mirrors ap_ir::expr::CmpOp order).
pub const CMP_EQ: i64 = 0;
pub const CMP_NOTEQ: i64 = 1;
pub const CMP_LT: i64 = 2;
pub const CMP_LTE: i64 = 3;
pub const CMP_GT: i64 = 4;
pub const CMP_GTE: i64 = 5;
pub const CMP_IS: i64 = 6;
pub const CMP_ISNOT: i64 = 7;
pub const CMP_IN: i64 = 8;
pub const CMP_NOTIN: i64 = 9;

/// UnaryOp encodings.
pub const UOP_PLUS: i64 = 0;
pub const UOP_MINUS: i64 = 1;
pub const UOP_INVERT: i64 = 2;
pub const UOP_NOT: i64 = 3;

// ---------------------------------------------------------------------------
// Helpers — one public symbol per cfg::Inst that needs runtime work
// ---------------------------------------------------------------------------

/// BinOp through ap_obj::ops::binop (clauses S-01-3..S-03-25 preserved by
/// that surface — the JIT adds no semantics of its own).
pub extern "C" fn h_binop(ctx: &mut ExecCtx, op: i64, a: u32, b: u32, dst: u32) -> HelperStatus {
    let a = ctx.slots[a as usize].clone();
    let b = ctx.slots[b as usize].clone();
    match ap_obj::ops::binop(&a, decode_binop(op), &b) {
        Ok(v) => {
            ctx.slots[dst as usize] = v;
            0
        }
        Err(e) => park_exc(ctx, e),
    }
}

/// UnaryOp through the ap-obj surface.
pub extern "C" fn h_unaryop(ctx: &mut ExecCtx, op: i64, a: u32, dst: u32) -> HelperStatus {
    let v = ctx.slots[a as usize].clone();
    let r = match op {
        UOP_PLUS => Ok(v.clone()),
        UOP_MINUS => ap_obj::ops::neg(&v),
        UOP_INVERT => ap_obj::ops::invert(&v),
        UOP_NOT => {
            // S-01-27: `not x` is exactly True/False by truthiness.
            Ok(Value::Bool(!v.is_truthy()))
        }
        _ => unreachable!("CLIF embeds only encoder constants"),
    };
    match r {
        Ok(v) => {
            ctx.slots[dst as usize] = v;
            0
        }
        Err(e) => park_exc(ctx, e),
    }
}

/// Compare through the ap-obj surface.
pub extern "C" fn h_compare(ctx: &mut ExecCtx, op: i64, a: u32, b: u32, dst: u32) -> HelperStatus {
    use ap_obj::value::Value;
    let a = ctx.slots[a as usize].clone();
    let b = ctx.slots[b as usize].clone();
    let r = match op {
        CMP_EQ => Ok(Value::Bool(ap_obj::ops::eq(&a, &b))),
        CMP_NOTEQ => Ok(Value::Bool(!ap_obj::ops::eq(&a, &b))),
        CMP_LT => ap_obj::ops::lt(&a, &b).map(Value::Bool),
        CMP_LTE => ap_obj::ops::lt(&b, &a).map(|gt| Value::Bool(!gt)),
        CMP_GT => ap_obj::ops::lt(&b, &a).map(Value::Bool),
        CMP_GTE => ap_obj::ops::lt(&a, &b).map(|lt| Value::Bool(!lt)),
        CMP_IS | CMP_ISNOT => {
            // Identity: same object or same singleton value.
            let same = a_ptr_eq(&a, &b);
            Ok(Value::Bool(if op == CMP_IS { same } else { !same }))
        }
        CMP_IN | CMP_NOTIN => {
            // `a in b` — membership via iteration protocol on b.
            Err(PyException::new(
                "TypeError",
                "membership (in) not supported by baseline JIT yet",
            ))
        }
        _ => unreachable!("CLIF embeds only encoder constants"),
    };
    match r {
        Ok(v) => {
            ctx.slots[dst as usize] = v;
            0
        }
        Err(e) => park_exc(ctx, e),
    }
}

fn a_ptr_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::None, Value::None) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Ellipsis, Value::Ellipsis) => true,
        // Gc-backed arms (Gc<T> = Rc<T>): same allocation ⇒ same object.
        (Value::Str(x), Value::Str(y)) => std::rc::Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Truthiness test (S-01-27 falsy set) — result is a boxed bool.
pub extern "C" fn h_truthy(ctx: &mut ExecCtx, a: u32, dst: u32) -> HelperStatus {
    let t = ctx.slots[a as usize].is_truthy();
    ctx.slots[dst as usize] = Value::Bool(t);
    0
}

/// Load a local slot into another slot (NULL-if-unbound modeled as None).
pub extern "C" fn h_load_local(ctx: &mut ExecCtx, src: u32, dst: u32) -> HelperStatus {
    let v = ctx.slots[src as usize].clone();
    ctx.slots[dst as usize] = v;
    0
}

/// Store a value into a local slot.
pub extern "C" fn h_store_local(ctx: &mut ExecCtx, src: u32, dst: u32) -> HelperStatus {
    let v = ctx.slots[src as usize].clone();
    ctx.slots[dst as usize] = v;
    0
}

/// Global load through the host seam (NameError on unbound — S-02-1).
pub extern "C" fn h_load_global(ctx: &mut ExecCtx, name_idx: u32, dst: u32) -> HelperStatus {
    match (ctx.host.load_global)(ctx, name_idx, dst) {
        Ok(()) => 0,
        Err(e) => park_exc(ctx, e),
    }
}

/// Attribute load through the host seam (S-03-9).
pub extern "C" fn h_load_attr(
    ctx: &mut ExecCtx,
    obj: u32,
    name_idx: u32,
    dst: u32,
) -> HelperStatus {
    match (ctx.host.load_attr)(ctx, obj, name_idx, dst) {
        Ok(()) => 0,
        Err(e) => park_exc(ctx, e),
    }
}

/// Attribute store through the host seam (S-03-10).
pub extern "C" fn h_store_attr(ctx: &mut ExecCtx, obj: u32, name_idx: u32, v: u32) -> HelperStatus {
    match (ctx.host.store_attr)(ctx, obj, name_idx, v) {
        Ok(()) => 0,
        Err(e) => park_exc(ctx, e),
    }
}

/// Call through the host seam (S-02-7..S-02-10, S-03-28). Args are staged
/// in ctx.args[arg_start..arg_start+argc] by the JIT'd code before the call.
pub extern "C" fn h_call(
    ctx: &mut ExecCtx,
    callee: u32,
    arg_start: u32,
    argc: u32,
    dst: u32,
) -> HelperStatus {
    match (ctx.host.call)(ctx, callee, arg_start, argc, dst) {
        Ok(()) => 0,
        Err(e) => park_exc(ctx, e),
    }
}

/// print() fast path used by the hello-world parity probe: str() the value
/// (ap_obj::ops::to_str — C-18 rendering through the ap-obj surface) and
/// append to ctx.out; the host flushes.
pub extern "C" fn h_print(ctx: &mut ExecCtx, v: u32) -> HelperStatus {
    let v = ctx.slots[v as usize].clone();
    match ap_obj::ops::to_str(&v) {
        Ok(s) => {
            ctx.out.extend_from_slice(s.as_bytes());
            ctx.out.push(b'\n');
            0
        }
        Err(e) => park_exc(ctx, e),
    }
}

/// PushExcInfo: open a protected region.
pub extern "C" fn h_push_exc(ctx: &mut ExecCtx, region: u32) -> HelperStatus {
    ctx.exc_stack.push(region);
    0
}

/// PopExcInfo: close a protected region (VERIFIER RULE companion — the
/// cfg verifier guarantees every exit edge emits this call).
pub extern "C" fn h_pop_exc(ctx: &mut ExecCtx, region: u32) -> HelperStatus {
    // Pop down to (and including) `region`; mismatch = internal bug, but
    // fail-closed: report as a runtime error, never continue silently.
    match ctx.exc_stack.pop() {
        Some(r) if r == region => 0,
        other => {
            ctx.exc_stack = Vec::new();
            park_exc(
                ctx,
                PyException::new(
                    "SystemError",
                    format!(
                        " PopExcInfo region mismatch: expected {region}, got {:?}",
                        other
                    ),
                ),
            )
        }
    }
}

/// MatchExc: bind the in-flight exception for the region's handler chain.
/// The typed-handler test is host-owned (ap-run classes.rs); the baseline
/// accepts bare-except shapes only (typed handlers fail-closed at cfg
/// lowering).
pub extern "C" fn h_match_exc(ctx: &mut ExecCtx, region: u32, dst: u32) -> HelperStatus {
    if ctx.match_region != region {
        return park_exc(
            ctx,
            PyException::new("SystemError", format!("MatchExc region mismatch: {region}")),
        );
    }
    // The exception object itself is host-represented; the baseline binds
    // None into the name slot (bare except doesn't bind). If a name binds,
    // the host patches ctx.slots[dst] from its exception record.
    ctx.slots[dst as usize] = Value::None;
    0
}

/// Reraise: propagate the in-flight exception outward (host reads
/// pending_exc at the function boundary).
pub extern "C" fn h_reraise(ctx: &mut ExecCtx) -> HelperStatus {
    if ctx.pending_exc.is_none() {
        return park_exc(
            ctx,
            PyException::new("SystemError", "Reraise with no in-flight exception"),
        );
    }
    1
}

// ---------------------------------------------------------------------------
// Const materialization helpers (clif.rs Const lowering)
// ---------------------------------------------------------------------------

pub extern "C" fn h_const_none(ctx: &mut ExecCtx, dst: u32) -> HelperStatus {
    ctx.slots[dst as usize] = Value::None;
    0
}

pub extern "C" fn h_const_bool(ctx: &mut ExecCtx, dst: u32, v: i64) -> HelperStatus {
    ctx.slots[dst as usize] = Value::Bool(v != 0);
    0
}

pub extern "C" fn h_const_int(ctx: &mut ExecCtx, dst: u32, v: i64) -> HelperStatus {
    ctx.slots[dst as usize] = Value::Int(ap_obj::Int::from_i64(v));
    0
}

pub extern "C" fn h_const_float(ctx: &mut ExecCtx, dst: u32, bits: i64) -> HelperStatus {
    let f = f64::from_bits(bits as u64);
    ctx.slots[dst as usize] = Value::Float(f);
    0
}

/// `ptr` points at a static byte payload in the JIT data section (alive
/// for the module's lifetime — the JitFunc owns the blob).
pub extern "C" fn h_const_str(ctx: &mut ExecCtx, dst: u32, ptr: i64, len: i64) -> HelperStatus {
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            ctx.slots[dst as usize] = Value::Str(ap_obj::PyString::new(s));
            0
        }
        Err(_) => park_exc(
            ctx,
            PyException::new("SystemError", "non-UTF8 string constant"),
        ),
    }
}

/// Flattened host subscript-reader signature: `Err` carries the rendered
/// "Class: message" text (the Flow::Raise rendering flattened — the
/// codegen seam reconstructs the Python exception from it).
type HostSubscriptGet =
    fn(&ap_obj::value::Value, &ap_obj::value::Value) -> Result<ap_obj::value::Value, String>;

/// Subscript read (S-01-15) — routed through the host seam via the ctx
/// slot carry (t1_016-family fix, landed with leaf 02.05g's gate run):
/// the JIT has no host borrow here, so the helper implements the read
/// through ap-run's `methods::subscript_get` surface the tree-walker
/// uses — lists/strings/tuples/bytes/dict-string reads all work; the
/// errors flow back as Python exceptions, not SystemErrors. The ap-run
/// dependency for this one surface is taken through a process-global
/// function pointer installed at JIT init (same inverted-seam pattern
/// as weakbridge — ap-codegen-native cannot depend on ap-run).
static SUBSCRIPT_GET_FN: std::sync::RwLock<Option<HostSubscriptGet>> = std::sync::RwLock::new(None);

/// Install the host subscript reader (ap-run calls once at entry).
pub fn set_host_subscript_get(f: HostSubscriptGet) {
    if let Ok(mut g) = SUBSCRIPT_GET_FN.write() {
        *g = Some(f);
    }
}

pub extern "C" fn h_subscript(ctx: &mut ExecCtx, obj: u32, idx: u32, dst: u32) -> HelperStatus {
    // Class-name interning: PyException::new takes a &'static str (the
    // same leak-intern pattern jit_hook::intern_class uses — the error
    // class vocabulary is small and closed).
    fn intern_class(class: &str) -> &'static str {
        thread_local! {
            static INTERNED: std::cell::RefCell<std::collections::HashMap<String, &'static str>> =
                std::cell::RefCell::new(std::collections::HashMap::new());
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
    let o = ctx.slots[obj as usize].clone();
    let k = ctx.slots[idx as usize].clone();
    let f = SUBSCRIPT_GET_FN.read().ok().and_then(|g| *g);
    match f {
        Some(f) => match f(&o, &k) {
            Ok(v) => {
                ctx.slots[dst as usize] = v;
                0
            }
            Err(class_msg) => {
                // class_msg is "Class: message" (the Flow::Raise rendering
                // the host seam flattens); split on the first colon.
                let (class, msg) = match class_msg.split_once(": ") {
                    Some((c, m)) => (c.to_string(), m.to_string()),
                    None => ("KeyError".to_string(), class_msg),
                };
                park_exc(ctx, PyException::new(intern_class(&class), msg))
            }
        },
        None => park_exc(
            ctx,
            PyException::new(
                "SystemError",
                format!(
                    "Subscript read not supported by baseline JIT yet (got {})",
                    o.type_name()
                ),
            ),
        ),
    }
}

/// Stage one call argument into ctx.args[k] (clif.rs Call lowering).
pub extern "C" fn h_stage_arg(ctx: &mut ExecCtx, k: u32, src: u32) -> HelperStatus {
    let k = k as usize;
    if ctx.args.len() <= k {
        ctx.args.resize(k + 1, Value::None);
    }
    ctx.args[k] = ctx.slots[src as usize].clone();
    0
}

/// Branch-side truthiness: unboxed 0/1 for CLIF brif (no allocation).
pub extern "C" fn h_truthy_bit(ctx: &mut ExecCtx, a: u32) -> i32 {
    ctx.slots[a as usize].is_truthy() as i32
}

/// Return terminator: park the value and complete.
pub extern "C" fn h_return(ctx: &mut ExecCtx, v: u32) -> HelperStatus {
    ctx.ret = ctx.slots[v as usize].clone();
    0
}

/// Raise terminator: park the exception object and take the raise edge.
/// The boxed exception value converts to the payload layer here; the host
/// re-materializes the traceback-wrapped exception at the boundary.
pub extern "C" fn h_raise(ctx: &mut ExecCtx, v: u32) -> HelperStatus {
    let v = ctx.slots[v as usize].clone();
    // The boxed value IS the exception instance (Obj arm) in real runs;
    // converting to the PyException payload layer is host-owned. v0:
    // raise sites raise pre-constructed exception objects via Call, so
    // the pending exception was parked by the raising helper already;
    // this helper is the terminator-side marker (returns raise status).
    let _ = v;
    1
}

/// Generator suspend: record state, park the yielded value.
pub extern "C" fn h_suspend(ctx: &mut ExecCtx, state: u32, v: u32) -> HelperStatus {
    ctx.ret = ctx.slots[v as usize].clone();
    let _ = state; // consumed by the generator state machine (phase 5)
    1
}

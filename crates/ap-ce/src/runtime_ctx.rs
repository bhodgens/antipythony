//! Leaf 10.04 (S1 item 1): the runtime-context registry — how a C extension
//! calls BACK into the running interpreter (PyObject_Call family).
//!
//! Model decision (recorded in docs/plan10-1004-s1-bridge-report.md):
//! ap-ce cannot depend on ap-run (workspace layering: ap-run depends on
//! ap-ce), so the call-back is a REGISTERED FUNCTION POINTER: ap-run's
//! ce_bridge installs one `PycallFn` at import time (`install_pycall`);
//! the exported PyObject_Call* symbols dispatch through it. The
//! interpreter handle itself rides the runtime's existing thread-local
//! (`interp::current_interp_handle`, the 02.09 per-worker model that
//! classes.rs metaclass hooks already use) — the explicit
//! runtime-context-parameter alternative was rejected because C slot fns
//! must keep CPython's exact ABI and the thread-local is already the
//! runtime's own convention.
//!
//! Re-entrancy is safe by construction: C fn -> registered pycall ->
//! call_value -> Python code -> C method -> registered pycall -> ... —
//! each hop is an ordinary Rust stack frame (no GIL to drop, no
//! thread-state swap; PEP 703 posture).

use std::ffi::c_int;
use std::sync::OnceLock;

use ap_obj::value::Value;

use crate::ffi_types::PyObjPtr;

/// The call-back ABI: (callable, args tuple, kwargs dict-or-NULL) -> result
/// handle. NULL return == error state set (CPython convention).
pub type PycallFn = unsafe extern "C" fn(PyObjPtr, PyObjPtr, PyObjPtr) -> PyObjPtr;

static PYCALL: OnceLock<PycallFn> = OnceLock::new();

/// Install the runtime's call-back (idempotent; first installer wins —
/// one runtime per process).
pub fn install_pycall(f: PycallFn) -> bool {
    PYCALL.set(f).is_ok()
}

/// The registered call-back, if the runtime is attached.
pub fn pycall_fn() -> Option<PycallFn> {
    PYCALL.get().copied()
}

// ===========================================================================
// Leaf 1005c (S1 band, item 2): the runtime seams the error/traceback
// projections need. Same registered-fn-pointer discipline: ap-run's
// ce_bridge installs one value-level entry (`ErrTbFn`) at import; the
// exported PyErr_*/PyFrame_*/PyCode_* symbols dispatch through it. The
// entry is a tiny value-domain interpreter: every op is a tagged enum
// crossing the boundary as a Rust enum by POINTER (never dereferenced C-
// side — the runtime owns the type), keeping the C ABI fixed while the
// op set grows.
// ===========================================================================

/// One dispatch op. `Value` args/results ride as ap-obj values (the fn
/// signature takes raw `*const Value` / writes `*mut Value`); the runtime
/// owns every dereference. Frame-chain ops carry their payloads inline.
#[derive(Debug, Clone)]
pub enum ErrTbOp {
    /// (frame index from top) -> Value: the runtime's `<frame>` stub value
    /// (the s016 py_frame_stub shape — carries f_code/f_lineno/f_locals).
    MakeFrame(usize),
    /// (qualname, filename, lineno) -> Value: a `<code>` Namespace stub.
    MakeCode(String, String, i64),
    /// (frame rows outermost-first) -> Value: a `<tb>` snapshot value.
    MakeTb(Vec<(String, String, usize)>),
    /// (frame value, attr) -> Option<Value>: f_locals/f_globals/f_lineno
    /// live reads.
    FrameAttr(Value, String),
    /// (frame value, name) -> Option<Value>: a live local by name.
    FrameGetVar(Value, String),
    /// (generator value) -> Option<Value>: gi_code.
    GeneratorCode(Value),
    /// (code value, attr) -> Option<Value>: co_code/co_varnames/... reads.
    CodeAttr(Value, String),
    /// (frame value) -> Option<usize>: the py_frames index a `<frame>`
    /// value represents (None when the value is not a frame).
    FrameIndex(Value),
    /// (exception instance value) -> (class, str(exc)).
    ExcDescribe(Value),
    /// (class name) -> Option<Value>: a real exception instance the way
    /// Python's `except` binding materializes one.
    MakeExceptionInstance(String, String),
    /// (value) -> Option<String>: the CLASS name behind a class-or-
    /// instance value (exception_instance's class or the class itself).
    ClassNameOf(Value),
    /// (value, attr) -> Option<Value>: the runtime attr protocol (the
    /// PyObject_GetAttrString route; user properties resolve).
    GetAttr(Value, String),
}

/// The entry ABI: (op-by-move) -> Result. Success writes the Value result
/// (or Unit for None) into `out` and returns 0; failure returns -1 with
/// the runtime's error state NOT touched (ops are total — they answer
/// Option semantics).
pub type ErrTbFn = unsafe extern "C" fn(op: *const ErrTbOp, out: *mut ErrTbOut) -> c_int;

/// The op result carrier. `Value` for Some(Value); `None_` for None;
/// `Idx` for Option<usize>.
#[derive(Debug, Clone)]
pub enum ErrTbOut {
    Value(Value),
    None_,
    Idx(usize),
    NoIdx,
}

static ERRTB_FN: OnceLock<ErrTbFn> = OnceLock::new();

/// Install the value-level entry (idempotent, first wins).
pub fn install_errtb(f: ErrTbFn) -> bool {
    ERRTB_FN.set(f).is_ok()
}

fn errtb_fn() -> Option<ErrTbFn> {
    ERRTB_FN.get().copied()
}

/// Run one op; Some(Value) back. Shared unwrapper for every projection.
fn op_value(op: ErrTbOp) -> Option<Value> {
    let f = errtb_fn()?;
    let mut out = ErrTbOut::None_;
    // SAFETY: f is the fn pointer ap-run installed via the install_* OnceLock
    // seam (first-wins, never uninstalled), called with pointers of the exact
    // ABI types, valid for the call's duration; the callee treats op as total.
    let rc = unsafe { (f)(&op, &mut out) };
    if rc != 0 {
        return None;
    }
    match out {
        ErrTbOut::Value(v) => Some(v),
        _ => None,
    }
}

fn op_idx(op: ErrTbOp) -> Option<usize> {
    let f = errtb_fn()?;
    let mut out = ErrTbOut::NoIdx;
    // SAFETY: f is the fn pointer ap-run installed via the install_* OnceLock
    // seam (first-wins, never uninstalled), called with pointers of the exact
    // ABI types, valid for the call's duration; the callee treats op as total.
    let rc = unsafe { (f)(&op, &mut out) };
    if rc != 0 {
        return None;
    }
    match out {
        ErrTbOut::Idx(i) => Some(i),
        _ => None,
    }
}

// --- thin wrappers the exported symbols call ------------------------------

pub fn make_frame_value(idx: usize) -> Option<Value> {
    op_value(ErrTbOp::MakeFrame(idx))
}
pub fn make_code_value(name: &str, file: &str, lineno: i64) -> Option<Value> {
    op_value(ErrTbOp::MakeCode(
        name.to_string(),
        file.to_string(),
        lineno,
    ))
}
pub fn make_tb_value(rows: &[(String, String, usize)]) -> Option<Value> {
    op_value(ErrTbOp::MakeTb(rows.to_vec()))
}
pub fn frame_attr(v: &Value, attr: &str) -> Option<Value> {
    op_value(ErrTbOp::FrameAttr(v.clone(), attr.to_string()))
}
pub fn frame_get_var(v: &Value, name: &str) -> Option<Value> {
    op_value(ErrTbOp::FrameGetVar(v.clone(), name.to_string()))
}
pub fn generator_code(v: &Value) -> Option<Value> {
    op_value(ErrTbOp::GeneratorCode(v.clone()))
}
pub fn code_attr(v: &Value, attr: &str) -> Option<Value> {
    op_value(ErrTbOp::CodeAttr(v.clone(), attr.to_string()))
}
pub fn frame_index(v: &Value) -> Option<usize> {
    op_idx(ErrTbOp::FrameIndex(v.clone()))
}

/// (class, str(exc)) for an exception instance value, via the standard
/// single-Value carrier (a 2-tuple). Falls back to (type name, str) when
/// no runtime is attached.
pub fn exc_describe(v: &Value) -> (String, String) {
    match op_value(ErrTbOp::ExcDescribe(v.clone())) {
        Some(Value::Tuple(t)) => {
            let c = match t.0.first() {
                Some(Value::Str(s)) => s.as_str().to_string(),
                _ => String::new(),
            };
            let x = match t.0.get(1) {
                Some(Value::Str(s)) => s.as_str().to_string(),
                _ => String::new(),
            };
            (c, x)
        }
        _ => {
            let cls = match v {
                Value::Obj(o) => o.name.clone(),
                other => other.type_name().to_string(),
            };
            let text = ap_obj::ops::to_str(v).unwrap_or_default();
            (cls, text)
        }
    }
}

pub fn make_exception_instance(class: &str, text: &str) -> Option<Value> {
    op_value(ErrTbOp::MakeExceptionInstance(
        class.to_string(),
        text.to_string(),
    ))
}

pub fn class_name_of(v: &Value) -> Option<String> {
    match op_value(ErrTbOp::ClassNameOf(v.clone())) {
        Some(Value::Str(s)) => Some(s.as_str().to_string()),
        _ => None,
    }
}

pub fn get_attr(v: &Value, name: &str) -> Option<Value> {
    op_value(ErrTbOp::GetAttr(v.clone(), name.to_string()))
}

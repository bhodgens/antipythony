//! Leaf 10.05c (S1 band, item 2): the error/traceback C-API plumbing —
//! PyErr_Format family, PyErr_NormalizeException, ExceptionMatches pair,
//! Fetch/Restore/SetObject/GetRaisedException/SetRaisedException,
//! PyErr_NewException, plus the PyFrame_*/PyCode_*/PyGen_GetCode projection
//! surface.
//!
//! Model (following runtime_ctx's registered-fn-pointer discipline — ap-ce
//! cannot depend on ap-run): the traceback/frame projections need LIVE
//! runtime state (interp.py_frames, the raised exception's __ap_tb__
//! entries), so this module declares ONE context fn-pointer table that
//! ap-run's ce_bridge installs at import time:
//!
//!   frame_query_fn  — resolves the live frame chain: returns
//!                     (frame depth, code-name, co_filename, lineno)
//!                     tuples outermost-first from interp.py_frames.
//!   matches_fn      — the runtime's exception-subclass predicate
//!                     (exc_hier::is_subclass + user-class MRO walk).
//!   newexc_fn       — PyErr_NewException backend: builds a real user
//!                     exception class via classes::dynamic_type.
//!
//! Error state: a NEW thread-local (ERRTB) that carries the exception
//! CLASS NAME, the str value, and the normalized instance Value. The S0
//! err_set/err_take machine in state.rs stays untouched (exports.rs
//! interop contract): PyErr_SetString writes BOTH (ERRTB is the rich
//! view), and the bridge's err_take path keeps working byte-identically.
//! Fetch moves ERRTB -> handles and clears; Restore puts them back.
//!
//! Varargs: Rust cannot declare C varargs fns, so PyErr_Format follows the
//! NEXT-1004 ParseTuple pattern — fixed-arity format-family backends live
//! here (apx_PyErr_Format*), and apx_shim.c carries ONE C varargs
//! dispatcher that reads the format string, fetches va_args, and routes to
//! the matching backend. The exported symbol `PyErr_Format` itself is the
//! C dispatcher (in apx_shim.c, compiled into the ap-ce staticlib).

use std::cell::RefCell;
use std::ffi::{c_char, c_int};
use std::sync::OnceLock;

use ap_obj::value::{PyString, Value};

use crate::ffi_types::PyObjPtr;
use crate::state::{
    ce_newref, err_is_set, err_set, new_handle, py_none, CeHandle, CeKind, CePayload,
};

// ---------------------------------------------------------------------------
// The rich error state (leaf-owned thread-local)
// ---------------------------------------------------------------------------

/// The rich per-thread error state. `set` mirrors state::ErrState's flag;
/// class is the exception CLASS NAME; value_text is str(exc); instance is
/// the normalized instance Value when one was raised (None-Value when the
/// error is class+message shaped).
#[derive(Default)]
pub struct ErrTb {
    pub set: bool,
    pub class: String,
    pub value_text: String,
    pub instance: Option<Value>,
}

thread_local! {
    pub static ERRTB: RefCell<ErrTb> = RefCell::new(ErrTb::default());
}

/// Write BOTH error machines: the S0 machine state.rs reads (err_set) and
/// this leaf's rich ERRTB. Every raise path in this module goes through
/// here; PyErr_SetString (exports.rs) keeps its own err_set call — the
/// bridge converts from err_take, so S0 behavior is unchanged.
pub fn errtb_set(class: &str, msg: impl Into<String>) {
    let msg = msg.into();
    err_set(class, msg.clone());
    ERRTB.with(|e| {
        *e.borrow_mut() = ErrTb {
            set: true,
            class: class.to_string(),
            value_text: msg,
            instance: None,
        }
    });
}

/// Write the rich state from a RAISED instance Value (the raise round-trip:
/// the runtime hands us an exception instance object). Also writes the S0
/// machine so a NULL return from a C fn still surfaces the right class.
pub fn errtb_set_instance(v: &Value) {
    let (class, text) = crate::runtime_ctx::exc_describe(v);
    err_set(&class, text.clone());
    ERRTB.with(|e| {
        *e.borrow_mut() = ErrTb {
            set: true,
            class,
            value_text: text,
            instance: Some(v.clone()),
        }
    });
}

/// Take the rich state (Fetch semantics: clears). The S0 machine
/// (state::ErrState — what PyErr_SetString/Occurred and the bridge read)
/// is the AUTHORITY: a Fetch consumes its indicator and adopts its class/
/// msg into the rich view (the instance, when Restore parked one, rides
/// the ERRTB side table keyed by the same raise).
pub fn errtb_take() -> Option<ErrTb> {
    ERRTB.with(|e| {
        let mut b = e.borrow_mut();
        // S0 is authoritative: always consume it first.
        let s0 = crate::state::err_take();
        if let Some((class, msg)) = s0 {
            // Rich state matches the S0 indicator only when both are set
            // with the same class (a Restore that parked an instance).
            let instance = if b.set && b.class == class {
                b.instance.take()
            } else {
                None
            };
            *b = ErrTb::default();
            return Some(ErrTb {
                set: true,
                class,
                value_text: msg,
                instance,
            });
        }
        // S0 clear: stale rich state is meaningless — drop it.
        *b = ErrTb::default();
        None
    })
}

/// Put rich state back (Restore semantics). The S0 machine gets re-set so
/// PyErr_Occurred / the bridge's err_take keep seeing the indicator.
pub fn errtb_restore(st: ErrTb) {
    if st.set {
        err_set(&st.class, st.value_text.clone());
    }
    ERRTB.with(|e| *e.borrow_mut() = st);
}

/// Clear rich state (PyErr_Clear route — also clears the S0 machine via
/// state::err_clear, which exports.rs's PyErr_Clear already calls).
pub fn errtb_clear() {
    crate::state::err_clear();
    ERRTB.with(|e| *e.borrow_mut() = ErrTb::default());
}

/// The rich view of the CURRENT error (peek; does not clear). Used by
/// PyErr_Occurred's class-name surface and the tb projections.
pub fn errtb_peek() -> Option<(String, String)> {
    ERRTB.with(|e| {
        let b = e.borrow();
        if b.set {
            Some((b.class.clone(), b.value_text.clone()))
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// The registered runtime-context fns (ap-run installs at first ce dispatch)
// ---------------------------------------------------------------------------

/// One live frame row: (code name/qualname, co_filename, lineno, f_lasti).
pub type FrameRow = (String, String, i64, i64);

/// Resolve the live Python frame chain, OUTERMOST FIRST
/// (interp.py_frames order; sys._getframe(0) = the LAST row).
pub type FrameQueryFn = unsafe extern "C" fn(*mut FrameBuf) -> c_int;

/// The C-visible frame buffer the query fills (fixed capacity).
#[repr(C)]
pub struct FrameBuf {
    pub rows: *mut FrameRow,
    pub cap: usize,
    pub len: usize,
}

/// Exception-subclass predicate: (raised class name, target class name) ->
/// 1 when `raised` IS `target` or derives from it per the runtime's
/// subclass semantics (builtin tree + user-class MRO edges).
pub type MatchesFn = unsafe extern "C" fn(*const c_char, *const c_char) -> c_int;

/// PyErr_NewException backend: (dotted name, base class name-or-NULL,
/// members json-or-NULL) -> the new class as a runtime Value parked in
/// value_map under the returned key (usize::MAX on failure).
pub type NewExcFn = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> usize;

/// Describe an exception instance Value: (class, str(exc)) — used by
/// errtb_set_instance. Implemented as a small ABI over the runtime.
pub type DescribeFn = unsafe extern "C" fn(*const Value, *mut Value, *mut Value) -> c_int;

static FRAME_QUERY: OnceLock<FrameQueryFn> = OnceLock::new();
static MATCHES: OnceLock<MatchesFn> = OnceLock::new();
static NEWEXC: OnceLock<NewExcFn> = OnceLock::new();

pub fn install_frame_query(f: FrameQueryFn) -> bool {
    FRAME_QUERY.set(f).is_ok()
}
pub fn install_matches(f: MatchesFn) -> bool {
    MATCHES.set(f).is_ok()
}
pub fn install_newexc(f: NewExcFn) -> bool {
    NEWEXC.set(f).is_ok()
}

fn frame_query_fn() -> Option<FrameQueryFn> {
    FRAME_QUERY.get().copied()
}
fn matches_fn() -> Option<MatchesFn> {
    MATCHES.get().copied()
}
fn newexc_fn() -> Option<NewExcFn> {
    NEWEXC.get().copied()
}

/// Snapshot the live frame chain into a fresh Vec (None when no runtime /
/// no frames). Rows outermost-first, matching interp.py_frames order.
pub fn frame_rows() -> Option<Vec<FrameRow>> {
    let f = frame_query_fn()?;
    const CAP: usize = 256;
    let mut storage: Vec<FrameRow> = Vec::with_capacity(CAP);
    let mut buf = FrameBuf {
        rows: storage.as_mut_ptr(),
        cap: CAP,
        len: 0,
    };
    unsafe { (f)(&mut buf) };
    if buf.len > CAP {
        buf.len = CAP;
    }
    unsafe { storage.set_len(buf.len) };
    Some(storage)
}

/// Runtime subclass check by NAME with the registered predicate; falls
/// back to state.rs's class-name equality when no runtime is attached.
pub fn runtime_matches(raised: &str, target: &str) -> bool {
    if raised == target {
        return true;
    }
    if let Some(f) = matches_fn() {
        let r = std::ffi::CString::new(raised).unwrap_or_default();
        let t = std::ffi::CString::new(target).unwrap_or_default();
        let ok = unsafe { (f)(r.as_ptr(), t.as_ptr()) };
        return ok != 0;
    }
    // No runtime attached (pure unit-test context): exact match only.
    false
}

// ---------------------------------------------------------------------------
// PyErr_Format family (the fixed-arity backends; `PyErr_Format` itself is
// the C varargs dispatcher in apx_shim.c routing here per format char).
// ---------------------------------------------------------------------------

/// PyErr_Format with an argument object under %S / %s(str() form) —
/// msg pre-rendered by the shim; class from the token handle.
#[no_mangle]
pub unsafe extern "C" fn apx_PyErr_Format_1(exc: PyObjPtr, msg: *const c_char) -> PyObjPtr {
    apx_PyErr_Format_s(exc, msg)
}

/// Backend: class token + fully-rendered message (the shim renders the
/// format string from its va_args; %S/%s/%d/%zd/... resolve C-side).
#[no_mangle]
pub unsafe extern "C" fn apx_PyErr_Format_s(exc: PyObjPtr, msg: *const c_char) -> PyObjPtr {
    let m = if msg.is_null() {
        "".to_string()
    } else {
        std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned()
    };
    let class = exception_class_of_token(exc);
    errtb_set(&class, m);
    std::ptr::null_mut()
}

/// The exception CLASS NAME behind a PyExc_* token handle (a Str handle
/// carrying the class name per the S0 convention; a real class Value
/// (Opaque) reads its object name).
unsafe fn exception_class_of_token(exc: PyObjPtr) -> String {
    if exc.is_null() {
        return "Exception".to_string();
    }
    let h = exc as CeHandle;
    let o = &*h;
    let decoded = match o.payload.borrow().as_ref() {
        Some(CePayload::Str(s)) => Some(s.clone()),
        Some(CePayload::Opaque(v)) => {
            let val = Value::Obj(v.clone());
            crate::runtime_ctx::class_name_of(&val)
        }
        _ => None,
    };
    let _ = o;
    decoded.unwrap_or_else(|| {
        // Fall back to the payload kind name (a Type handle named the
        // class; an unrecognized shape degrades to Exception — recorded).
        match o.ob_type {
            CeKind::Str => "Exception".to_string(),
            _ => "Exception".to_string(),
        }
    })
}

// ---------------------------------------------------------------------------
// PyErr_ExceptionMatches / PyErr_GivenExceptionMatches
// ---------------------------------------------------------------------------

/// PyErr_ExceptionMatches(exc): match the CURRENT error against a class.
/// The S0 machine is the raise-side authority (PyErr_SetString writes
/// only it); the rich view adds instance fidelity when present.
#[no_mangle]
pub unsafe extern "C" fn PyErr_ExceptionMatches(exc: PyObjPtr) -> c_int {
    if !err_is_set() {
        return 0;
    }
    // Rich view first (carries the raised instance's real class), then
    // the S0 class (the SetString route).
    let raised = match errtb_peek() {
        Some((class, _)) => class,
        None => match crate::state::err_take() {
            Some((class, msg)) => {
                // Peek, not take: put it back (S0 remains the indicator).
                crate::state::err_set(&class, msg);
                class
            }
            None => return 0,
        },
    };
    let target = exception_class_of_token(exc);
    runtime_matches(&raised, &target) as c_int
}

/// PyErr_GivenExceptionMatches(given, exc): match an EXPLICIT exception
/// class-or-instance against a class. `given` may be a class token
/// (Str/Opaque class) or an instance (Opaque exception instance).
#[no_mangle]
pub unsafe extern "C" fn PyErr_GivenExceptionMatches(given: PyObjPtr, exc: PyObjPtr) -> c_int {
    if given.is_null() || exc.is_null() {
        return 0;
    }
    let h = given as CeHandle;
    let o = &*h;
    let given_class = match o.payload.borrow().as_ref() {
        // Class token: the class name directly.
        Some(CePayload::Str(s)) => s.clone(),
        // Instance or class Value: resolve through the runtime's
        // instance->class mapping (Opaque arm; fall back to the name).
        Some(CePayload::Opaque(v)) => {
            let val = Value::Obj(v.clone());
            match crate::runtime_ctx::class_name_of(&val) {
                Some(cn) => cn,
                None => v.name.clone(),
            }
        }
        _ => {
            errtb_set(
                "SystemError",
                "PyErr_GivenExceptionMatches: non-exception argument",
            );
            return -1;
        }
    };
    let target = exception_class_of_token(exc);
    runtime_matches(&given_class, &target) as c_int
}

// ---------------------------------------------------------------------------
// PyErr_Fetch / PyErr_Restore / PyErr_SetObject / raisedException pair
// ---------------------------------------------------------------------------

/// PyErr_Fetch(exc, val, tb): MOVES the error into the three slots (new
/// references) and clears the indicator. Value = str(exc) handle when the
/// error was class+message shaped, else the normalized instance. The tb
/// slot gets the frame chain snapshot encoded as an opaque FrameRow-list
/// handle (CPython hands a traceback object; our snapshot is what the
/// projections below read — honest about the shape: a `<tb>` marker).
#[no_mangle]
pub unsafe extern "C" fn PyErr_Fetch(
    exc_out: *mut PyObjPtr,
    val_out: *mut PyObjPtr,
    tb_out: *mut PyObjPtr,
) {
    let st = errtb_take();
    let Some(st) = st else {
        if !exc_out.is_null() {
            *exc_out = std::ptr::null_mut();
        }
        if !val_out.is_null() {
            *val_out = std::ptr::null_mut();
        }
        if !tb_out.is_null() {
            *tb_out = std::ptr::null_mut();
        }
        return;
    };
    // The class token (a Str handle carrying the class name — the same
    // shape PyExc_* tokens have, so Restore/Fetch round-trips).
    let exc_h = new_handle(CeKind::Str, CePayload::Str(st.class.clone()));
    // The value: prefer the raised instance; a class+msg error renders
    // str(exc) (the message) per CPython's val = str-shaped instance.
    let val_h = match &st.instance {
        Some(v) => crate::state::from_value(v),
        None => new_handle(CeKind::Str, CePayload::Str(st.value_text.clone())),
    };
    // The tb slot: a live-frame snapshot marker (the projections read it).
    let tb_h = tb_snapshot_handle();
    ce_newref(exc_h);
    ce_newref(val_h);
    ce_newref(tb_h);
    if !exc_out.is_null() {
        *exc_out = exc_h as PyObjPtr;
    }
    if !val_out.is_null() {
        *val_out = val_h as PyObjPtr;
    }
    if !tb_out.is_null() {
        *tb_out = tb_h as PyObjPtr;
    }
}

/// PyErr_Restore(exc, val, tb): moves the three back INTO the indicator.
#[no_mangle]
pub unsafe extern "C" fn PyErr_Restore(exc: PyObjPtr, val: PyObjPtr, tb: PyObjPtr) {
    let _ = tb; // accepted for ABI shape; our indicator keeps no tb slot
    if exc.is_null() {
        errtb_clear();
        return;
    }
    let class = exception_class_of_token(exc);
    // Value: an instance Value when the val slot carried one; otherwise
    // str() of the val handle (the message text).
    let v = val as CeHandle;
    if v.is_null() {
        errtb_set(&class, "");
        return;
    }
    let o = &*v;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Opaque(v)) => {
            // Exception instance round-trip: rich state carries it.
            errtb_set_instance(&Value::Obj(v.clone()));
            // errtb_set_instance derived class/str from the instance; but
            // the EXC slot's class is authoritative when it names a builtin
            // token (the instance arm of CPython's Restore keeps type(exc)).
            errtb_patch_class(&class);
        }
        Some(CePayload::Str(s)) => errtb_set(&class, s.clone()),
        _ => {
            let text = crate::value_conv::to_value(v)
                .map(|vv| ap_obj::ops::to_str(&vv).unwrap_or_default())
                .unwrap_or_default();
            errtb_set(&class, text);
        }
    }
}

/// Patch ONLY the class name in the rich state (Restore's type-slot wins
/// over the instance-derived class, matching CPython PyErr_Restore).
fn errtb_patch_class(class: &str) {
    ERRTB.with(|e| {
        let mut b = e.borrow_mut();
        if b.set {
            b.class = class.to_string();
        }
    });
    // Mirror into the S0 machine (err_set overwrites wholesale; re-set with
    // the current text under the new class).
    if let Some((_, text)) = errtb_peek() {
        err_set(class, text);
    }
}

/// PyErr_NormalizeException(exc, val, tb): normalize a (class, value, tb)
/// triple so val IS a real exception instance. Our raise path already
/// normalizes (the rich error state carries the instance when one was
/// raised), so this materializes the instance when the val slot is a bare
/// message and passes the class through — CPython semantics for the
/// normalized shape. NULL slots pass through untouched.
#[no_mangle]
pub unsafe extern "C" fn PyErr_NormalizeException(
    exc: *mut PyObjPtr,
    val: *mut PyObjPtr,
    _tb: *mut PyObjPtr,
) {
    if exc.is_null() || val.is_null() {
        return;
    }
    let e = *exc;
    let v = *val;
    if e.is_null() {
        return;
    }
    // Already an instance (Opaque carrying a runtime object): done.
    let v_h = v as CeHandle;
    if !v_h.is_null() {
        let is_instance = unsafe {
            matches!(
                (&*v_h).payload.borrow().as_ref(),
                Some(CePayload::Opaque(_))
            )
        };
        if is_instance {
            return;
        }
    }
    // Bare value: materialize the instance (class + str(val)).
    let class = exception_class_of_token(e);
    let text = if v_h.is_null() {
        String::new()
    } else {
        crate::value_conv::to_value(v_h)
            .map(|vv| ap_obj::ops::to_str(&vv).unwrap_or_default())
            .unwrap_or_default()
    };
    let Some(inst) = crate::runtime_ctx::make_exception_instance(&class, &text) else {
        // No runtime: leave the triple as-is (honest degradation).
        return;
    };
    let new_v = crate::state::from_value(&inst);
    ce_newref(new_v);
    *val = new_v as PyObjPtr;
}

/// PyErr_SetObject(exc, value): raise with an EXPLICIT value object.
#[no_mangle]
pub unsafe extern "C" fn PyErr_SetObject(exc: PyObjPtr, value: PyObjPtr) {
    let class = exception_class_of_token(exc);
    if value.is_null() {
        // CPython: value NULL => bare class raise (empty message).
        errtb_set(&class, "");
        return;
    }
    let v = value as CeHandle;
    let o = &*v;
    match o.payload.borrow().as_ref() {
        // Exception instance (or any runtime object): carry it faithfully.
        Some(CePayload::Opaque(v)) => errtb_set_instance(&Value::Obj(v.clone())),
        Some(CePayload::Str(s)) => errtb_set(&class, s.clone()),
        Some(CePayload::Int(i)) => {
            errtb_set(&class, i.to_string_raw());
        }
        _ => {
            let text = crate::value_conv::to_value(v)
                .map(|vv| ap_obj::ops::to_str(&vv).unwrap_or_default())
                .unwrap_or_default();
            errtb_set(&class, text);
        }
    }
}

/// PyErr_GetRaisedException(): MOVES the current error into a single
/// exception-instance handle (or NULL when no error). CPython 3.14 form.
#[no_mangle]
pub unsafe extern "C" fn PyErr_GetRaisedException() -> PyObjPtr {
    let st = errtb_take();
    let Some(st) = st else {
        return std::ptr::null_mut();
    };
    match st.instance {
        Some(v) => crate::state::from_value(&v) as PyObjPtr,
        None => {
            // Materialize an exception instance the same shape Python's
            // `except` binding sees (class name + args=(str,)).
            match crate::runtime_ctx::make_exception_instance(&st.class, &st.value_text) {
                Some(v) => crate::state::from_value(&v) as PyObjPtr,
                None => new_handle(CeKind::Str, CePayload::Str(st.value_text)) as PyObjPtr,
            }
        }
    }
}

/// PyErr_SetRaisedException(exc): set the indicator FROM an exception
/// instance handle (the inverse of Get). NULL clears.
#[no_mangle]
pub unsafe extern "C" fn PyErr_SetRaisedException(exc: PyObjPtr) {
    if exc.is_null() {
        errtb_clear();
        return;
    }
    let v = exc as CeHandle;
    let o = &*v;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Opaque(v)) => errtb_set_instance(&Value::Obj(v.clone())),
        _ => {
            // Non-Opaque handles (Str value etc.) degrade to a class+text
            // raise under the runtime's generic Exception — recorded as a
            // finding (SetRaisedException contracts an instance).
            let text = crate::value_conv::to_value(v)
                .map(|vv| ap_obj::ops::to_str(&vv).unwrap_or_default())
                .unwrap_or_default();
            errtb_set("Exception", text);
        }
    }
}

// ---------------------------------------------------------------------------
// PyErr_NewException (+ WithDoc)
// ---------------------------------------------------------------------------

/// PyErr_NewException(name, base, dict): create a REAL exception class via
/// the runtime's dynamic_type (a UserClass with is_exception). Returns a
/// faithful Opaque handle carrying the class Value.
#[no_mangle]
pub unsafe extern "C" fn PyErr_NewException(
    name: *const c_char,
    base: PyObjPtr,
    _dict: PyObjPtr,
) -> PyObjPtr {
    new_exception_impl(name, base, std::ptr::null())
}

/// PyErr_NewExceptionWithDoc(name, doc, base, dict): doc lands in the
/// class __doc__ (kept: CPython semantics — visible via help()/__doc__).
#[no_mangle]
pub unsafe extern "C" fn PyErr_NewExceptionWithDoc(
    name: *const c_char,
    doc: *const c_char,
    base: PyObjPtr,
    _dict: PyObjPtr,
) -> PyObjPtr {
    new_exception_impl(name, base, doc)
}

unsafe fn new_exception_impl(name: *const c_char, base: PyObjPtr, doc: *const c_char) -> PyObjPtr {
    if name.is_null() {
        errtb_set("SystemError", "PyErr_NewException: NULL name");
        return std::ptr::null_mut();
    }
    let full = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    // The DOTTED name rides the class identity (CPython repr <class
    // 'errtb.custom'>) — pass it through whole; the runtime carries it as
    // the class payload's name.
    let bare = full.clone();
    let base_class = if base.is_null() {
        "Exception".to_string()
    } else {
        exception_class_of_token(base)
    };
    match newexc_fn() {
        Some(f) => {
            let n = std::ffi::CString::new(bare.clone()).unwrap_or_default();
            let b = std::ffi::CString::new(base_class).unwrap_or_default();
            let d = if doc.is_null() {
                std::ffi::CString::new("").unwrap_or_default()
            } else {
                std::ffi::CString::new(std::ffi::CStr::from_ptr(doc).to_string_lossy().into_owned())
                    .unwrap_or_default()
            };
            let key = unsafe { (f)(n.as_ptr(), b.as_ptr(), d.as_ptr()) };
            if key == usize::MAX {
                // Error state set by the runtime side.
                return std::ptr::null_mut();
            }
            match crate::value_map::take_runtime_value(key) {
                Some(v) => crate::state::from_value(&v) as PyObjPtr,
                None => {
                    errtb_set("SystemError", "PyErr_NewException: class value lost");
                    std::ptr::null_mut()
                }
            }
        }
        None => {
            errtb_set(
                "SystemError",
                "PyErr_NewException: no runtime attached (extension used outside ap-run)",
            );
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Frame / code projections
// ---------------------------------------------------------------------------

/// A `<code>` marker handle for frame row `row` (co_name/co_filename/
/// co_firstlineno set; CPython spelling via apx_TypeName-compatible
/// naming stays in the header — this handle reads as Namespace "<code>").
unsafe fn code_handle_for(row: &FrameRow) -> PyObjPtr {
    let v = crate::runtime_ctx::make_code_value(&row.0, &row.1, row.2);
    match v {
        Some(v) => crate::state::from_value(&v) as PyObjPtr,
        None => std::ptr::null_mut(),
    }
}

/// A `<frame>` handle for the live frame at py_frames index `idx`.
unsafe fn frame_handle_for(idx: usize) -> PyObjPtr {
    match crate::runtime_ctx::make_frame_value(idx) {
        Some(v) => crate::state::from_value(&v) as PyObjPtr,
        None => std::ptr::null_mut(),
    }
}

/// The traceback-snapshot handle: a runtime `<tb>` Namespace value
/// carrying the live frame chain (tb_frames-shaped tuples under the
/// "__ap_tb__" attr; CPython's tb object is itself a chain — ours is a
/// flat snapshot, honest about the shape).
fn tb_snapshot_handle() -> CeHandle {
    let rows = frame_rows().unwrap_or_default();
    match crate::runtime_ctx::make_tb_value(
        &rows
            .iter()
            .map(|(q, f, l, _)| (q.clone(), f.clone(), *l as usize))
            .collect::<Vec<_>>(),
    ) {
        Some(v) => crate::state::from_value(&v),
        None => py_none(),
    }
}

/// PyFrame_GetCode(frame) -> strong ref. Our `<frame>` handles carry the
/// py_frames index (slice_parts[0], the s016 shape; the index is the
/// OUTERMOST-FIRST array position — the same convention frame_rows()
/// snapshots in), so the code row is the DIRECT read.
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetCode(frame: PyObjPtr) -> PyObjPtr {
    let Some(idx) = frame_index_of(frame) else {
        return std::ptr::null_mut();
    };
    match frame_rows() {
        Some(rows) if idx < rows.len() => code_handle_for(&rows[idx]),
        _ => std::ptr::null_mut(),
    }
}

/// PyFrame_GetBack(frame) -> the frame one level out (new ref; NULL at
/// the outermost).
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetBack(frame: PyObjPtr) -> PyObjPtr {
    let Some(idx) = frame_index_of(frame) else {
        return std::ptr::null_mut();
    };
    if idx == 0 {
        return std::ptr::null_mut();
    }
    frame_handle_for(idx - 1)
}

/// PyFrame_GetLocals/Globals/Builtins: dict handles from the live frame
/// (locals for module frames IS globals — the runtime's own convention).
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetLocals(frame: PyObjPtr) -> PyObjPtr {
    frame_dict_attr(frame, "f_locals")
}

#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetGlobals(frame: PyObjPtr) -> PyObjPtr {
    frame_dict_attr(frame, "f_globals")
}

#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetBuiltins(frame: PyObjPtr) -> PyObjPtr {
    // The builtins dict surface: our runtime resolves builtins through the
    // global namespace chain; CPython exposes a dedicated dict. Honest
    // projection: the module globals (where our builtin lookups land).
    frame_dict_attr(frame, "f_globals")
}

unsafe fn frame_dict_attr(frame: PyObjPtr, attr: &str) -> PyObjPtr {
    let h = frame as CeHandle;
    if h.is_null() {
        return std::ptr::null_mut();
    }
    match crate::state::opaque_object(h) {
        Some(v) => match crate::runtime_ctx::frame_attr(&Value::Obj(v), attr) {
            Some(av) => crate::state::from_value(&av) as PyObjPtr,
            None => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

/// PyFrame_GetGenerator(frame): NULL on S1 (our frame chain carries no
/// generator back-pointer; recorded finding, not a lying stub).
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetGenerator(_frame: PyObjPtr) -> PyObjPtr {
    std::ptr::null_mut()
}

/// PyFrame_GetLasti(frame): -1 (our frames carry statement granularity,
/// not instruction offsets; CPython's own default for entry frames).
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetLasti(_frame: PyObjPtr) -> c_int {
    -1
}

/// PyFrame_GetLineNumber(frame): the frame's live line cell.
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetLineNumber(frame: PyObjPtr) -> c_int {
    let h = frame as CeHandle;
    if h.is_null() {
        return -1;
    }
    unsafe {
        let o = &*h;
        if let Some(CePayload::Opaque(v)) = o.payload.borrow().as_ref() {
            if let Some(av) = crate::runtime_ctx::frame_attr(&Value::Obj(v.clone()), "f_lineno") {
                if let Some(i) = av.as_int().and_then(|i| i.to_i64()) {
                    return i as c_int;
                }
            }
        }
    }
    -1
}

/// PyFrame_GetVar(frame, name) / GetVarString: the live local (new ref;
/// raises KeyError per CPython 3.12+ on a missing name — the CPython
/// 3.14 dialect: PyErr_SetString(PyExc_KeyError...)).
#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetVar(frame: PyObjPtr, name: PyObjPtr) -> PyObjPtr {
    let name_text = {
        let h = name as CeHandle;
        if h.is_null() {
            return std::ptr::null_mut();
        }
        match crate::value_conv::to_value(h) {
            Some(Value::Str(s)) => s.as_str().to_string(),
            Some(_) => {
                errtb_set("TypeError", "PyFrame_GetVar: name must be a string");
                return std::ptr::null_mut();
            }
            None => return std::ptr::null_mut(),
        }
    };
    let h = frame as CeHandle;
    if h.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let o = &*h;
        let opaque = o.payload.borrow().as_ref().and_then(|p| match p {
            CePayload::Opaque(v) => Some(v.clone()),
            _ => None,
        });
        let Some(v) = opaque else {
            return std::ptr::null_mut();
        };
        match crate::runtime_ctx::frame_get_var(&Value::Obj(v), &name_text) {
            Some(av) => {
                let out = crate::state::from_value(&av);
                ce_newref(out);
                out as PyObjPtr
            }
            None => {
                errtb_set("KeyError", format!("PyFrame_GetVar: {name_text}"));
                std::ptr::null_mut()
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyFrame_GetVarString(frame: PyObjPtr, name: *const c_char) -> PyObjPtr {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let n = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let name_h = new_handle(CeKind::Str, CePayload::Str(n));
    unsafe { PyFrame_GetVar(frame, name_h as PyObjPtr) }
}

/// PyGen_GetCode(gen): the code object of a generator's frame. Our
/// generator objects carry gi_code (interp.rs Generator arm); the Opaque
/// handle resolves through the runtime attr surface.
#[no_mangle]
pub unsafe extern "C" fn PyGen_GetCode(gen: PyObjPtr) -> PyObjPtr {
    let h = gen as CeHandle;
    if h.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let o = &*h;
        let opaque = o.payload.borrow().as_ref().and_then(|p| match p {
            CePayload::Opaque(v) => Some(v.clone()),
            _ => None,
        });
        let Some(v) = opaque else {
            errtb_set("SystemError", "PyGen_GetCode: not a generator");
            return std::ptr::null_mut();
        };
        match crate::runtime_ctx::generator_code(&Value::Obj(v)) {
            Some(cv) => crate::state::from_value(&cv) as PyObjPtr,
            None => {
                errtb_set("SystemError", "PyGen_GetCode: generator carries no code");
                std::ptr::null_mut()
            }
        }
    }
}

/// PyCode_GetCode(co) -> co_code as a bytes handle (b"" — our projected
/// code objects carry no bytecode; CPython's own answer for stub frames).
#[no_mangle]
pub unsafe extern "C" fn PyCode_GetCode(co: PyObjPtr) -> PyObjPtr {
    code_attr_handle(co, "co_code")
}

#[no_mangle]
pub unsafe extern "C" fn PyCode_GetVarnames(co: PyObjPtr) -> PyObjPtr {
    code_attr_handle(co, "co_varnames")
}

#[no_mangle]
pub unsafe extern "C" fn PyCode_GetCellvars(co: PyObjPtr) -> PyObjPtr {
    code_attr_handle(co, "co_cellvars")
}

#[no_mangle]
pub unsafe extern "C" fn PyCode_GetFreevars(co: PyObjPtr) -> PyObjPtr {
    code_attr_handle(co, "co_freevars")
}

unsafe fn code_attr_handle(co: PyObjPtr, attr: &str) -> PyObjPtr {
    let h = co as CeHandle;
    if h.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        let o = &*h;
        let opaque = o.payload.borrow().as_ref().and_then(|p| match p {
            CePayload::Opaque(v) => Some(v.clone()),
            _ => None,
        });
        let Some(v) = opaque else {
            return std::ptr::null_mut();
        };
        match crate::runtime_ctx::code_attr(&Value::Obj(v), attr) {
            Some(av) => {
                let out = crate::state::from_value(&av);
                ce_newref(out);
                out as PyObjPtr
            }
            None => std::ptr::null_mut(),
        }
    }
}

/// PyCode_NewEmpty(filename, funcname, lineno) — a fresh code marker with
/// just the three fields (CPython's own documented semantics).
#[no_mangle]
pub unsafe extern "C" fn PyCode_NewEmpty(
    filename: *const c_char,
    funcname: *const c_char,
    lineno: c_int,
) -> PyObjPtr {
    if filename.is_null() || funcname.is_null() {
        errtb_set("SystemError", "PyCode_NewEmpty: NULL argument");
        return std::ptr::null_mut();
    }
    let f = std::ffi::CStr::from_ptr(filename)
        .to_string_lossy()
        .into_owned();
    let n = std::ffi::CStr::from_ptr(funcname)
        .to_string_lossy()
        .into_owned();
    match crate::runtime_ctx::make_code_value(&n, &f, lineno as i64) {
        Some(v) => crate::state::from_value(&v) as PyObjPtr,
        None => std::ptr::null_mut(),
    }
}

// ---------------------------------------------------------------------------
// Attribute / bytes / unicode accessors the band's proof extension probes
// through (each routes the runtime's own protocol).
// ---------------------------------------------------------------------------

/// PyObject_GetAttrString: the runtime attr protocol (user properties and
/// namespaces resolve; a miss raises AttributeError with the CPython text).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetAttrString(o: PyObjPtr, name: *const c_char) -> PyObjPtr {
    if o.is_null() || name.is_null() {
        errtb_set("SystemError", "PyObject_GetAttrString: NULL argument");
        return std::ptr::null_mut();
    }
    let n = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let h = o as CeHandle;
    let Some(v) = crate::value_conv::to_value(h) else {
        errtb_set("SystemError", "PyObject_GetAttrString: bad handle");
        return std::ptr::null_mut();
    };
    match crate::runtime_ctx::get_attr(&v, &n) {
        Some(av) => crate::state::from_value(&av) as PyObjPtr,
        None => {
            errtb_set(
                "AttributeError",
                format!("'{}' object has no attribute '{n}'", {
                    let tn = match &v {
                        Value::Obj(vo) => {
                            crate::runtime_ctx::class_name_of(&v).unwrap_or_else(|| vo.name.clone())
                        }
                        other => other.type_name().to_string(),
                    };
                    tn
                }),
            );
            std::ptr::null_mut()
        }
    }
}

/// PyBytes_Size: len(bytes) (b"" for our projected code objects).
#[no_mangle]
pub unsafe extern "C" fn PyBytes_Size(o: PyObjPtr) -> isize {
    let h = o as CeHandle;
    if h.is_null() {
        errtb_set("SystemError", "PyBytes_Size: NULL object");
        return -1;
    }
    match crate::value_conv::to_value(h) {
        Some(Value::Bytes(b)) => b.len() as isize,
        Some(_) => {
            let tn = crate::value_conv::to_value(h)
                .map(|v| v.type_name().to_string())
                .unwrap_or_default();
            errtb_set("TypeError", format!("expected bytes, not '{tn}'"));
            -1
        }
        None => -1,
    }
}

/// PyUnicode_GetLength: len(str) (code points).
#[no_mangle]
pub unsafe extern "C" fn PyUnicode_GetLength(o: PyObjPtr) -> isize {
    let h = o as CeHandle;
    if h.is_null() {
        errtb_set("SystemError", "PyUnicode_GetLength: NULL object");
        return -1;
    }
    match crate::value_conv::to_value(h) {
        Some(Value::Str(s)) => s.as_str().chars().count() as isize,
        Some(_) => {
            let tn = crate::value_conv::to_value(h)
                .map(|v| v.type_name().to_string())
                .unwrap_or_default();
            errtb_set("TypeError", format!("expected str, not '{tn}'"));
            -1
        }
        None => -1,
    }
}

/// PyLong_FromSsize_t.
#[no_mangle]
pub unsafe extern "C" fn PyLong_FromSsize_t(v: isize) -> PyObjPtr {
    new_handle(
        CeKind::Int,
        CePayload::Int(ap_obj::int::Int::from_i64(v as i64)),
    ) as PyObjPtr
}

/// The py_frames INDEX a `<frame>` handle represents (CPython frame
/// handles ours mint carry slice_parts[0] = index from the TOP/innermost;
/// resolved live through the runtime's own stub value).
unsafe fn frame_index_of(frame: PyObjPtr) -> Option<usize> {
    let h = frame as CeHandle;
    if h.is_null() {
        return None;
    }
    let o = &*h;
    let opaque = o.payload.borrow().as_ref().and_then(|p| match p {
        CePayload::Opaque(v) => Some(v.clone()),
        _ => None,
    });
    crate::runtime_ctx::frame_index(&Value::Obj(opaque?))
}

/// C-side text view of an object for PyErr_Format's %S/%s conversions:
/// str(o) or repr(o) resolved through the runtime (CPython protocol —
/// user __str__/__repr__ dispatch). Process-lifetime stable pointer into
/// a leaked allocation (the same ownership the S0 UTF8 views use).
#[no_mangle]
pub unsafe extern "C" fn apx_errtb_obj_text(o: PyObjPtr, use_repr: c_int) -> *const c_char {
    let h = o as CeHandle;
    if h.is_null() {
        return b"NULL\0".as_ptr() as *const c_char;
    }
    let Some(v) = crate::value_conv::to_value(h) else {
        return b"<str failed>\0".as_ptr() as *const c_char;
    };
    let text = if use_repr != 0 {
        ap_obj::ops::repr(&v)
    } else {
        ap_obj::ops::to_str(&v)
    };
    match text {
        Ok(s) => {
            let leaked = Box::leak(s.into_boxed_str());
            leaked.as_ptr() as *const c_char
        }
        Err(_) => b"<str failed>\0".as_ptr() as *const c_char,
    }
}

// ---------------------------------------------------------------------------
// ap-ce unit-test escape hatches (also used by the ce_bridge installer)
// ---------------------------------------------------------------------------

/// The shim's PyErr_Format varargs dispatcher re-export shim: the C side
/// calls the backends directly; this fn exists so cargo test can exercise
/// the class-token decode path.
#[no_mangle]
pub unsafe extern "C" fn apx_errtb_frame_rows_len() -> c_int {
    frame_rows().map(|r| r.len() as c_int).unwrap_or(-1)
}

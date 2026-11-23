//! Leaf 1005b (S1 band 2): the object-protocol C-API surface.
//!
//! Exports (CPython-spelled, unprefixed `#[no_mangle] extern "C"`, per the
//! 10.04 unprefixed-export gotcha — extensions reference the CPython symbol
//! names and dlopen resolves them against the running binary):
//!
//! - Attr: PyObject_Get/Set/DelAttr + Get/Set/DelAttrString + the 3.13+
//!   Optional variants (out-param + 1/0/-1 int contract).
//! - Length: PyObject_Length + PyObject_LengthHint (CPython's
//!   Objects/abstract.c protocol: __length_hint__ first — negative =>
//!   ValueError "__length_hint__() should return >= 0", non-int =>
//!   TypeError "__length_hint__ must be an integer, not '<T>'"; hint
//!   errors other than miss/NotImplemented PROPAGATE — probed live
//!   3.14.7), then len(o), then the default.
//! - Compare: PyObject_RichCompare/RichCompareBool (full user-slot
//!   reflectance + proper-subclass priority + exact op-spelled TypeError
//!   dialect) + PyObject_IsTrue/Not (user __bool__/__len__ honored).
//!   RichCompareBool carries CPython's identity shortcut (o1 is o2 =>
//!   Py_EQ answers 1 / Py_NE answers 0 WITHOUT dispatching).
//! - Iter: PyObject_GetIter/IterNext bridging the EXISTING iterator
//!   registry (runtime side routes through builtin_iter/pull_iter_pub —
//!   the OOM-campaign lifecycle; NO second registry here). IterNext
//!   returns NULL with the error state CLEAR at exhaustion (CPython
//!   clears the StopIteration before returning; PyO3 loops key on
//!   PyErr_Occurred() == NULL).
//! - Hash/Dir: PyObject_Hash (ap-obj py_hash — CPython-exact values incl.
//!   the -1 => -2 sentinel) + PyObject_Dir (the sorted builtin surface).
//!
//! Dispatch model: ap-ce cannot depend on ap-run (workspace layering:
//! ap-run depends on ap-ce), so — exactly like 10.04's PycallFn — the
//! runtime protocol implementations arrive as a REGISTERED VTABLE
//! (`install_objproto_vtable`, called by ap-run's ce_bridge at first ce
//! import through the shared `ensure_pycall_installed` seam). Every
//! exported fn checks the vtable; a missing runtime surfaces as
//! SystemError ("extension used outside ap-run"), never a panic.
//!
//! Error protocol: runtime Flow errors come back as (class, msg) strings
//! inside OpRes and are surfaced through `err_set` — the same thread-local
//! state PyErr_Occurred reads, byte-identical round-trip with 10.04.

use std::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;

use ap_obj::value::Value;

use crate::ffi_types::PyObjPtr;
use crate::state::{
    ce_newref, err_clear, err_is_set, err_set, from_value, new_handle, py_none, CeHandle, CeKind,
    CePayload,
};

// ---------------------------------------------------------------------------
// The registered runtime vtable (installed by ap-run's ce_bridge)
// ---------------------------------------------------------------------------

/// One protocol-call result. `ok` = the call completed (success OR a clean
/// miss/exhaustion — check the flags); `ok == false` => err_class/err_msg
/// carry the raised exception.
pub struct OpRes {
    pub ok: bool,
    /// Attr miss (GetOptionalAttr contract: no error set).
    pub missing: bool,
    /// The AttributeError message the runtime's own protocol raised on a
    /// miss (byte-exact dialect — GetAttr re-raises it verbatim).
    pub miss_text: String,
    /// Iterator exhaustion (no error set).
    pub exhausted: bool,
    /// The result Value (len => Int, truth/rich => Bool, hash => Int,
    /// iter => iterator Obj, dir => List, next => the item).
    pub val: Option<Value>,
    pub err_class: String,
    pub err_msg: String,
}

impl OpRes {
    pub fn val(v: Value) -> Self {
        OpRes {
            ok: true,
            missing: false,
            miss_text: String::new(),
            exhausted: false,
            val: Some(v),
            err_class: String::new(),
            err_msg: String::new(),
        }
    }

    pub fn missing_with_text(text: impl Into<String>) -> Self {
        OpRes {
            ok: true,
            missing: true,
            miss_text: text.into(),
            exhausted: false,
            val: None,
            err_class: String::new(),
            err_msg: String::new(),
        }
    }

    pub fn exhausted() -> Self {
        OpRes {
            ok: true,
            missing: false,
            miss_text: String::new(),
            exhausted: true,
            val: None,
            err_class: String::new(),
            err_msg: String::new(),
        }
    }

    pub fn unit() -> Self {
        OpRes::val(Value::None)
    }

    pub fn err(class: impl Into<String>, msg: impl Into<String>) -> Self {
        OpRes {
            ok: false,
            missing: false,
            miss_text: String::new(),
            exhausted: false,
            val: None,
            err_class: class.into(),
            err_msg: msg.into(),
        }
    }
}

/// The runtime's object-protocol implementations (function pointers —
/// the layering contract; ap-run installs once at first ce import).
pub struct ObjProtoVTable {
    pub attr_get: fn(obj: &Value, name: &str) -> OpRes,
    pub attr_set: fn(obj: &Value, name: &str, v: Value) -> OpRes,
    pub attr_del: fn(obj: &Value, name: &str) -> OpRes,
    pub len_of: fn(obj: &Value) -> OpRes,
    pub length_hint: fn(obj: &Value, default: i64) -> OpRes,
    pub truth_of: fn(obj: &Value) -> OpRes,
    pub rich_compare: fn(a: &Value, b: &Value, op: u8, bool_form: bool) -> OpRes,
    pub get_iter: fn(obj: &Value) -> OpRes,
    pub iter_next: fn(obj: &Value) -> OpRes,
    pub hash_of: fn(obj: &Value) -> OpRes,
    pub dir_of: fn(obj: Option<&Value>) -> OpRes,
    pub display_name: fn(v: &Value) -> String,
}

static VTABLE: OnceLock<ObjProtoVTable> = OnceLock::new();

/// Install the runtime vtable (idempotent; first installer wins — one
/// runtime per process). Called by ap-run's ce_bridge.
pub fn install_objproto_vtable(t: ObjProtoVTable) -> bool {
    VTABLE.set(t).is_ok()
}

/// The installed vtable, or None when no runtime is attached (the caller
/// raises SystemError — never panic across the C boundary).
fn vt() -> Option<&'static ObjProtoVTable> {
    VTABLE.get()
}

/// Clone-out accessor for sibling bands (fn pointers are Copy — cheap).
/// Sibling modules route their protocol-dependent members through the
/// SAME table (one runtime, one vtable).
pub fn installed_vtable() -> Option<&'static ObjProtoVTable> {
    vt()
}

fn err_no_runtime(api: &str) {
    err_set(
        "SystemError",
        format!("{api}: no runtime attached (extension used outside ap-run)"),
    );
}

// ---------------------------------------------------------------------------
// Handle <-> Value plumbing (module-private)
// ---------------------------------------------------------------------------

/// C handle -> runtime Value. None for NULL.
fn value_of(o: PyObjPtr) -> Option<Value> {
    if o.is_null() {
        return None;
    }
    crate::value_conv::to_value(o as CeHandle)
}

/// Mint a NEW-reference handle for a runtime Value.
fn new_ref(v: &Value) -> PyObjPtr {
    let h = from_value(v);
    ce_newref(h);
    h as PyObjPtr
}

fn sys_err_null(api: &str) -> PyObjPtr {
    err_set("SystemError", format!("{api}: NULL object"));
    std::ptr::null_mut()
}

// ---------------------------------------------------------------------------
// Attribute protocol
// ---------------------------------------------------------------------------

/// The shared getattr core. CPython checks the name type BEFORE dispatching
/// (probed 3.14.7: getattr(obj, 42) => TypeError "attribute name must be
/// string, not 'int'" even when the object would answer). `quiet` selects
/// the GetOptionalAttr contract (NULL + NO error on miss).
unsafe fn get_attr_impl(o: PyObjPtr, name: PyObjPtr, quiet: bool) -> PyObjPtr {
    if o.is_null() {
        return sys_err_null("PyObject_GetAttr");
    }
    let name_v = value_of(name);
    let Some(Value::Str(s)) = &name_v else {
        let tn = name_v
            .as_ref()
            .map(|v| match vt() {
                Some(t) => (t.display_name)(v),
                None => v.type_name().to_string(),
            })
            .unwrap_or_else(|| "NULL".to_string());
        err_set(
            "TypeError",
            format!("attribute name must be string, not '{tn}'"),
        );
        return std::ptr::null_mut();
    };
    let Some(v) = value_of(o) else {
        return sys_err_null("PyObject_GetAttr");
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_GetAttr");
        return std::ptr::null_mut();
    };
    let attr = s.as_str().to_string();
    match (t.attr_get)(&v, &attr) {
        r if !r.ok => {
            err_set(&r.err_class, &r.err_msg);
            std::ptr::null_mut()
        }
        r if r.missing => {
            if quiet {
                std::ptr::null_mut() // NO error set (the Optional contract)
            } else {
                err_set("AttributeError", &r.miss_text);
                std::ptr::null_mut()
            }
        }
        r => match r.val {
            Some(out) => new_ref(&out),
            None => {
                err_set("SystemError", "PyObject_GetAttr: runtime returned no value");
                std::ptr::null_mut()
            }
        },
    }
}

/// PyObject_GetAttr — NEW reference or NULL with AttributeError set.
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetAttr(o: PyObjPtr, name: PyObjPtr) -> PyObjPtr {
    get_attr_impl(o, name, false)
}

/// PyObject_GetOptionalAttr (3.13+ ABI): (obj, name, *result) -> 1 found /
/// 0 miss (*result = NULL, no error) / -1 error (*result = NULL).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetOptionalAttr(
    o: PyObjPtr,
    name: PyObjPtr,
    result: *mut PyObjPtr,
) -> c_int {
    if !result.is_null() {
        *result = std::ptr::null_mut();
    }
    let r = get_attr_impl(o, name, true);
    if r.is_null() {
        return if err_is_set() { -1 } else { 0 };
    }
    if !result.is_null() {
        *result = r;
    }
    1
}

/// PyObject_GetAttrString.
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetAttrString(o: PyObjPtr, name: *const c_char) -> PyObjPtr {
    if name.is_null() {
        err_set("SystemError", "PyObject_GetAttrString: NULL name");
        return std::ptr::null_mut();
    }
    let owned = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let s = new_handle(CeKind::Str, CePayload::Str(owned));
    ce_newref(s);
    get_attr_impl(o, s as PyObjPtr, false)
}

/// PyObject_GetOptionalAttrString (3.13+ ABI): (obj, name, *result).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetOptionalAttrString(
    o: PyObjPtr,
    name: *const c_char,
    result: *mut PyObjPtr,
) -> c_int {
    if name.is_null() {
        err_set("SystemError", "PyObject_GetOptionalAttrString: NULL name");
        if !result.is_null() {
            *result = std::ptr::null_mut();
        }
        return -1;
    }
    let owned = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let s = new_handle(CeKind::Str, CePayload::Str(owned));
    ce_newref(s);
    PyObject_GetOptionalAttr(o, s as PyObjPtr, result)
}

/// The name-must-be-string gate shared by set/del (probed 3.14.7:
/// setattr/delattr raise the same text as getattr). None => error set.
unsafe fn name_string(name: PyObjPtr, api: &str) -> Option<String> {
    let name_v = value_of(name);
    match name_v {
        Some(Value::Str(s)) => Some(s.as_str().to_string()),
        other => {
            let tn = other
                .as_ref()
                .map(|v| match vt() {
                    Some(t) => (t.display_name)(v),
                    None => v.type_name().to_string(),
                })
                .unwrap_or_else(|| "NULL".to_string());
            err_set(
                "TypeError",
                format!("attribute name must be string, not '{tn}'"),
            );
            let _ = api;
            None
        }
    }
}

/// PyObject_SetAttr — 0 ok / -1 error. NULL value means DELETE (the
/// CPython C-API contract PyObject_SetAttr documents).
#[no_mangle]
pub unsafe extern "C" fn PyObject_SetAttr(o: PyObjPtr, name: PyObjPtr, v: PyObjPtr) -> c_int {
    if o.is_null() {
        err_set("SystemError", "PyObject_SetAttr: NULL object");
        return -1;
    }
    let Some(attr) = name_string(name, "PyObject_SetAttr") else {
        return -1;
    };
    let Some(obj) = value_of(o) else {
        err_set("SystemError", "PyObject_SetAttr: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_SetAttr");
        return -1;
    };
    let r = if v.is_null() {
        (t.attr_del)(&obj, &attr)
    } else {
        match value_of(v) {
            Some(val) => (t.attr_set)(&obj, &attr, val),
            None => OpRes::err("SystemError", "PyObject_SetAttr: NULL value"),
        }
    };
    if r.ok {
        0
    } else {
        err_set(&r.err_class, &r.err_msg);
        -1
    }
}

/// PyObject_SetAttrString.
#[no_mangle]
pub unsafe extern "C" fn PyObject_SetAttrString(
    o: PyObjPtr,
    name: *const c_char,
    v: PyObjPtr,
) -> c_int {
    if name.is_null() {
        err_set("SystemError", "PyObject_SetAttrString: NULL name");
        return -1;
    }
    let owned = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let s = new_handle(CeKind::Str, CePayload::Str(owned));
    ce_newref(s);
    PyObject_SetAttr(o, s as PyObjPtr, v)
}

/// PyObject_DelAttr.
#[no_mangle]
pub unsafe extern "C" fn PyObject_DelAttr(o: PyObjPtr, name: PyObjPtr) -> c_int {
    if o.is_null() {
        err_set("SystemError", "PyObject_DelAttr: NULL object");
        return -1;
    }
    let Some(attr) = name_string(name, "PyObject_DelAttr") else {
        return -1;
    };
    let Some(obj) = value_of(o) else {
        err_set("SystemError", "PyObject_DelAttr: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_DelAttr");
        return -1;
    };
    let r = (t.attr_del)(&obj, &attr);
    if r.ok {
        0
    } else {
        err_set(&r.err_class, &r.err_msg);
        -1
    }
}

/// PyObject_DelAttrString.
#[no_mangle]
pub unsafe extern "C" fn PyObject_DelAttrString(o: PyObjPtr, name: *const c_char) -> c_int {
    if name.is_null() {
        err_set("SystemError", "PyObject_DelAttrString: NULL name");
        return -1;
    }
    let owned = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let s = new_handle(CeKind::Str, CePayload::Str(owned));
    ce_newref(s);
    PyObject_DelAttr(o, s as PyObjPtr)
}

// ---------------------------------------------------------------------------
// Length protocol
// ---------------------------------------------------------------------------

/// PyObject_Length (len()) — -1 with error on "has no len()".
#[no_mangle]
pub unsafe extern "C" fn PyObject_Length(o: PyObjPtr) -> isize {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_Length: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_Length");
        return -1;
    };
    let r = (t.len_of)(&v);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return -1;
    }
    match r.val.and_then(|v| v.as_int().and_then(|i| i.to_i64())) {
        Some(n) => n as isize,
        None => {
            err_set("SystemError", "PyObject_Length: runtime returned non-int");
            -1
        }
    }
}

/// PyObject_LengthHint — the CPython abstract.c protocol (see module doc:
/// hint first with its two exact error dialects, then len, then default;
/// hint miss/NotImplemented falls through, other hint errors propagate).
#[no_mangle]
pub unsafe extern "C" fn PyObject_LengthHint(o: PyObjPtr, defaultvalue: isize) -> isize {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_LengthHint: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_LengthHint");
        return -1;
    };
    let r = (t.length_hint)(&v, defaultvalue as i64);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return -1;
    }
    match r.val.and_then(|v| v.as_int().and_then(|i| i.to_i64())) {
        Some(n) => n as isize,
        None => {
            err_set(
                "SystemError",
                "PyObject_LengthHint: runtime returned non-int",
            );
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Rich comparison + truthiness
// ---------------------------------------------------------------------------

/// Py_LT/Py_LE/Py_EQ/Py_NE/Py_GT/Py_GE (CPython object.h values).
pub const PY_LT: c_int = 0;
pub const PY_LE: c_int = 1;
pub const PY_EQ: c_int = 2;
pub const PY_NE: c_int = 3;
pub const PY_GT: c_int = 4;
pub const PY_GE: c_int = 5;

unsafe fn rich_impl(o1: PyObjPtr, o2: PyObjPtr, op: c_int, bool_form: bool) -> PyObjPtr {
    let (Some(a), Some(b)) = (value_of(o1), value_of(o2)) else {
        err_set("SystemError", "PyObject_RichCompare: NULL argument");
        return std::ptr::null_mut();
    };
    if !(PY_LT..=PY_GE).contains(&op) {
        err_set("SystemError", "PyObject_RichCompare: bad op");
        return std::ptr::null_mut();
    }
    let Some(t) = vt() else {
        err_no_runtime("PyObject_RichCompare");
        return std::ptr::null_mut();
    };
    let r = (t.rich_compare)(&a, &b, op as u8, bool_form);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return std::ptr::null_mut();
    }
    match r.val {
        Some(out) => new_ref(&out),
        None => {
            err_set(
                "SystemError",
                "PyObject_RichCompare: runtime returned no value",
            );
            std::ptr::null_mut()
        }
    }
}

/// PyObject_RichCompare — returns a Bool (new reference) or NULL + error.
#[no_mangle]
pub unsafe extern "C" fn PyObject_RichCompare(o1: PyObjPtr, o2: PyObjPtr, op: c_int) -> PyObjPtr {
    rich_impl(o1, o2, op, false)
}

/// PyObject_RichCompareBool — same protocol + CPython's identity shortcut
/// (o1 is o2: Py_EQ => 1, Py_NE => 0, WITHOUT dispatching __eq__; probed
/// nan != nan semantics unaffected — the shortcut only fires on identity).
#[no_mangle]
pub unsafe extern "C" fn PyObject_RichCompareBool(o1: PyObjPtr, o2: PyObjPtr, op: c_int) -> c_int {
    if !o1.is_null() && !o2.is_null() && std::ptr::eq(o1, o2) {
        if op == PY_EQ {
            return 1;
        }
        if op == PY_NE {
            return 0;
        }
    }
    let r = rich_impl(o1, o2, op, true);
    if r.is_null() {
        return -1;
    }
    crate::value_conv::as_i64(r as CeHandle)
        .map(|b| (b != 0) as c_int)
        .unwrap_or(-1)
}

/// PyObject_IsTrue — user __bool__/__len__ honored; -1 on error.
#[no_mangle]
pub unsafe extern "C" fn PyObject_IsTrue(o: PyObjPtr) -> c_int {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_IsTrue: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_IsTrue");
        return -1;
    };
    let r = (t.truth_of)(&v);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return -1;
    }
    match r.val {
        Some(Value::Bool(b)) => b as c_int,
        _ => {
            err_set("SystemError", "PyObject_IsTrue: runtime returned non-bool");
            -1
        }
    }
}

/// PyObject_Not — !IsTrue.
#[no_mangle]
pub unsafe extern "C" fn PyObject_Not(o: PyObjPtr) -> c_int {
    let r = PyObject_IsTrue(o);
    if r < 0 {
        -1
    } else {
        (r == 0) as c_int
    }
}

// ---------------------------------------------------------------------------
// Iterator protocol (bridge to the EXISTING registry)
// ---------------------------------------------------------------------------

/// PyObject_GetIter — NEW-reference iterator handle (runtime side routes
/// through builtin_iter: register_shared_iter lifecycle + generator lazy
/// anchors; iter(it) re-fetches the SAME state).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GetIter(o: PyObjPtr) -> PyObjPtr {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_GetIter: NULL object");
        return std::ptr::null_mut();
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_GetIter");
        return std::ptr::null_mut();
    };
    let r = (t.get_iter)(&v);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return std::ptr::null_mut();
    }
    match r.val {
        Some(out) => new_ref(&out),
        None => {
            err_set("SystemError", "PyObject_GetIter: runtime returned no value");
            std::ptr::null_mut()
        }
    }
}

/// PyObject_IterNext — next value (NEW reference) or NULL:
///   - NULL with the error state CLEAR at exhaustion (guaranteed here:
///     a pull path may transiently set StopIteration; CPython clears it),
///   - NULL with the error set on a real mid-iteration error.
#[no_mangle]
pub unsafe extern "C" fn PyObject_IterNext(o: PyObjPtr) -> PyObjPtr {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_IterNext: NULL object");
        return std::ptr::null_mut();
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_IterNext");
        return std::ptr::null_mut();
    };
    let r = (t.iter_next)(&v);
    if !r.ok {
        // A StopIteration surfacing at the C boundary IS exhaustion
        // (gen_pull maps clean generator exhaustion to a StopIteration
        // Flow in the Python-visible protocol; the C contract wants NULL +
        // cleared error — the PyO3 loop-breaker).
        if r.err_class == "StopIteration" {
            if err_is_set() {
                err_clear();
            }
            return std::ptr::null_mut();
        }
        err_set(&r.err_class, &r.err_msg);
        return std::ptr::null_mut();
    }
    if r.exhausted {
        if err_is_set() {
            err_clear();
        }
        return std::ptr::null_mut();
    }
    match r.val {
        Some(item) => new_ref(&item),
        None => {
            err_set(
                "SystemError",
                "PyObject_IterNext: runtime returned no value",
            );
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Hash / Dir
// ---------------------------------------------------------------------------

/// PyObject_Hash — CPython-exact hash values (ap-obj py_hash incl. the
/// -1 => -2 sentinel); -1 with TypeError set on unhashable.
#[no_mangle]
pub unsafe extern "C" fn PyObject_Hash(o: PyObjPtr) -> isize {
    let Some(v) = value_of(o) else {
        err_set("SystemError", "PyObject_Hash: NULL object");
        return -1;
    };
    let Some(t) = vt() else {
        err_no_runtime("PyObject_Hash");
        return -1;
    };
    let r = (t.hash_of)(&v);
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return -1;
    }
    match r.val.and_then(|v| v.as_int().and_then(|i| i.to_i64())) {
        Some(h) => {
            if h == -1 {
                // The C contract: -1 signals error — never a valid hash.
                -2
            } else {
                h as isize
            }
        }
        None => {
            err_set("SystemError", "PyObject_Hash: runtime returned non-int");
            -1
        }
    }
}

/// PyObject_Dir — the sorted builtin dir() surface (module ns / MRO dicts
/// / instance attrs). NULL object => SystemError (never calls the runtime).
#[no_mangle]
pub unsafe extern "C" fn PyObject_Dir(o: PyObjPtr) -> PyObjPtr {
    if o.is_null() {
        return sys_err_null("PyObject_Dir");
    }
    let v = value_of(o);
    let Some(t) = vt() else {
        err_no_runtime("PyObject_Dir");
        return std::ptr::null_mut();
    };
    let r = (t.dir_of)(v.as_ref());
    if !r.ok {
        err_set(&r.err_class, &r.err_msg);
        return std::ptr::null_mut();
    }
    match r.val {
        Some(list) => new_ref(&list),
        None => {
            err_set("SystemError", "PyObject_Dir: runtime returned no value");
            std::ptr::null_mut()
        }
    }
}

// ---------------------------------------------------------------------------
// Test helpers (private to the unit gates; no C surface)
// ---------------------------------------------------------------------------

#[allow(unused)]
fn _unused(_: *mut c_void) {}

/// None mint kept module-local for unit tests of the attr layer.
#[doc(hidden)]
pub fn test_str_value(s: &str) -> Value {
    Value::Str(ap_obj::value::PyString::new(s))
}

/// apx_PyIter_Check — the PyIter_Check seam (CPython parity: PyIter_Next
/// takes "is an iterator" as a CALLER precondition — bpo-19005; extensions
/// guard through this before calling PyObject_IterNext, mirroring how
/// CPython's builtin next() guards). Recognizes the runtime's Iterator /
/// Generator kinds through the faithful handle.
#[no_mangle]
pub unsafe extern "C" fn apx_PyIter_Check(o: PyObjPtr) -> c_int {
    let Some(Value::Obj(ob)) = value_of(o) else {
        return 0;
    };
    let it = matches!(
        ob.kind,
        ap_obj::ObjKind::Iterator | ap_obj::ObjKind::Generator
    );
    it as c_int
}

/// apx_DisplayTypeName — the runtime-side display type name (class name
/// for user instances/exceptions; the builtin kind otherwise). The
/// PyIter_Check-guard dialect and extension diagnostics route through it
/// ('int' for an int handle — the apx_TypeName static-table spelling says
/// 'object' for Opaque handles).
#[no_mangle]
pub unsafe extern "C" fn apx_DisplayTypeName(o: PyObjPtr) -> *const c_char {
    static NAMES: std::sync::Mutex<Vec<std::ffi::CString>> = std::sync::Mutex::new(Vec::new());
    let name = match value_of(o) {
        Some(v) => match vt() {
            Some(t) => (t.display_name)(&v),
            None => v.type_name().to_string(),
        },
        None => "NULL".to_string(),
    };
    let c = std::ffi::CString::new(name).unwrap_or_default();
    let mut g = NAMES.lock().unwrap_or_else(|p| p.into_inner());
    g.push(c);
    g.last().unwrap().as_ptr()
}

// PyLong_FromSsize_t: defined once in exports_containers.rs (leaf 1005a);
// the object-protocol band (leaf 1005b) consumes the same C symbol.

/// Unit-test seam: a no-op vtable (never installed in production; the
/// tests install their own probes and CANNOT uninstall — install order
/// matters, hence each test family owns a distinct fn-pointer identity).
#[doc(hidden)]
pub fn test_vtable_installed() -> bool {
    vt().is_some()
}

/// py_none re-export for the tests.
#[doc(hidden)]
pub fn test_none_handle() -> CeHandle {
    let h = py_none();
    ce_newref(h);
    h
}

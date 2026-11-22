//! The exported S0 C-API symbols (`#[no_mangle] extern "C"`) — the surface
//! extensions link against. Thin wrappers over the api/state modules.
//!
//! Symbol list = the spike's S0 subset (survey §2 S0 rows the spike
//! exercises; every symbol added here counts toward the density number).

use std::ffi::{c_char, c_int, c_long, c_void};

use crate::ffi_types::PyObjPtr;
use crate::state::{
    ce_newref, ce_xdecref, err_set, from_value, new_handle, py_false, py_none, py_true, CeHandle,
    CeKind, CePayload,
};

// ---------------------------------------------------------------------------
// Refcount protocol
// ---------------------------------------------------------------------------

/// Py_INCREF.
#[no_mangle]
pub extern "C" fn Py_IncRef(o: PyObjPtr) {
    if !o.is_null() {
        ce_newref(o as CeHandle);
    }
}

/// Py_DECREF.
#[no_mangle]
pub extern "C" fn Py_DecRef(o: PyObjPtr) {
    if !o.is_null() {
        ce_xdecref(o as CeHandle);
    }
}

/// Py_REFCNT (debug/diagnostic surface).
#[no_mangle]
pub extern "C" fn Py_Refcnt(o: PyObjPtr) -> isize {
    if o.is_null() {
        0
    } else {
        unsafe { (*(o as CeHandle)).ob_refcnt }
    }
}

// ---------------------------------------------------------------------------
// Singletons (Py_None / Py_True / Py_False live as FUNCTIONS in v0 — the
// header shim binds `#define Py_None (apx_Py_None())` so source compat
// holds without exporting data symbols).
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn apx_Py_None() -> PyObjPtr {
    py_none() as PyObjPtr
}

#[no_mangle]
pub extern "C" fn apx_Py_True() -> PyObjPtr {
    py_true() as PyObjPtr
}

#[no_mangle]
pub extern "C" fn apx_Py_False() -> PyObjPtr {
    py_false() as PyObjPtr
}

// ---------------------------------------------------------------------------
// Constructors (PyLong/PyFloat/PyUnicode/PyBool/PyTuple/PyList/PyDict)
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn PyLong_FromLong(v: std::ffi::c_long) -> PyObjPtr {
    new_handle(
        CeKind::Int,
        CePayload::Int(ap_obj::int::Int::from_i64(v as i64)),
    ) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_FromLongLong(v: i64) -> PyObjPtr {
    new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(v))) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_FromUnsignedLong(v: std::ffi::c_ulong) -> PyObjPtr {
    new_handle(
        CeKind::Int,
        CePayload::Int(ap_obj::int::Int::from_i64(v as i64)),
    ) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_AsLong(o: PyObjPtr) -> std::ffi::c_long {
    let h = o as CeHandle;
    match crate::value_conv::as_i64(h) {
        Some(v) => v as std::ffi::c_long,
        None => {
            let tn = unsafe { type_name_of(h) };
            err_set(
                "TypeError",
                format!("'{}' object cannot be interpreted as an integer", tn),
            );
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyLong_AsLongLong(o: PyObjPtr) -> i64 {
    let h = o as CeHandle;
    match crate::value_conv::as_i64(h) {
        Some(v) => v,
        None => {
            let tn = unsafe { type_name_of(h) };
            err_set(
                "TypeError",
                format!("'{}' object cannot be interpreted as an integer", tn),
            );
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyFloat_FromDouble(v: f64) -> PyObjPtr {
    new_handle(CeKind::Float, CePayload::Float(v)) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyFloat_AsDouble(o: PyObjPtr) -> f64 {
    let h = o as CeHandle;
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::Float(x)) => *x,
            Some(CePayload::Int(i)) => i.to_f64().unwrap_or(f64::NAN),
            Some(CePayload::Bool(b)) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            _ => {
                let tn = type_name_of(h);
                err_set("TypeError", format!("must be real number, not '{tn}'"));
                f64::NAN
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyBool_FromLong(v: std::ffi::c_long) -> PyObjPtr {
    if v != 0 {
        py_true() as PyObjPtr
    } else {
        py_false() as PyObjPtr
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyUnicode_FromString(s: *const c_char) -> PyObjPtr {
    if s.is_null() {
        err_set("SystemError", "PyUnicode_FromString: NULL input");
        return std::ptr::null_mut();
    }
    let owned = std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned();
    new_handle(CeKind::Str, CePayload::Str(owned)) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyUnicode_FromStringAndSize(s: *const c_char, n: isize) -> PyObjPtr {
    if s.is_null() {
        err_set("SystemError", "PyUnicode_FromStringAndSize: NULL input");
        return std::ptr::null_mut();
    }
    let bytes = std::slice::from_raw_parts(s as *const u8, n as usize);
    let owned = String::from_utf8_lossy(bytes).into_owned();
    new_handle(CeKind::Str, CePayload::Str(owned)) as PyObjPtr
}

/// PyUnicode_AsUTF8AndSize: hands out a pointer INTO the handle's string
/// (stable: the payload String lives as long as the process — we never
/// free, tracing-GC ownership).
#[no_mangle]
pub unsafe extern "C" fn PyUnicode_AsUTF8AndSize(
    o: PyObjPtr,
    size_out: *mut isize,
) -> *const c_char {
    let h = o as CeHandle;
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::Str(s)) => {
                if !size_out.is_null() {
                    *size_out = s.len() as isize;
                }
                s.as_ptr() as *const c_char
            }
            _ => {
                let tn = type_name_of(h);
                /* CPython 3.14 ParseTuple-'s' dialect: "argument N must be
                   str, not int" (older PyUnicode_FromString surfaces said
                   "expected str, not 'int'" — measured-divergent 10.03). */
                err_set("TypeError", format!("argument 1 must be str, not {tn}"));
                std::ptr::null()
            }
        }
    }
}

/// PyObject_Str fallback for the S0 dispatch surface.
#[no_mangle]
pub unsafe extern "C" fn PyObject_Str(o: PyObjPtr) -> PyObjPtr {
    let h = o as CeHandle;
    match crate::value_conv::to_value(h) {
        Some(v) => {
            let s = match &v {
                ap_obj::value::Value::Str(sv) => sv.as_str().to_string(),
                other => ap_obj::ops::repr(other).unwrap_or_else(|_| "<str failed>".to_string()),
            };
            new_handle(CeKind::Str, CePayload::Str(s)) as PyObjPtr
        }
        None => {
            err_set("SystemError", "PyObject_Str: NULL input");
            std::ptr::null_mut()
        }
    }
}

/// PyObject_Repr.
#[no_mangle]
pub unsafe extern "C" fn PyObject_Repr(o: PyObjPtr) -> PyObjPtr {
    let h = o as CeHandle;
    match crate::value_conv::to_value(h) {
        Some(v) => match ap_obj::ops::repr(&v) {
            Ok(s) => new_handle(CeKind::Str, CePayload::Str(s)) as PyObjPtr,
            Err(e) => {
                err_set("SystemError", format!("PyObject_Repr failed: {e:?}"));
                std::ptr::null_mut()
            }
        },
        None => {
            err_set("SystemError", "PyObject_Repr: NULL input");
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyTuple_New(n: isize) -> PyObjPtr {
    new_handle(
        CeKind::Tuple,
        CePayload::Tuple(Vec::with_capacity(n.max(0) as usize)),
    ) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyTuple_Size(o: PyObjPtr) -> isize {
    let h = o as CeHandle;
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::Tuple(items)) => items.len() as isize,
            _ => {
                let tn = type_name_of(h);
                err_set("TypeError", format!("expected tuple, not '{tn}'"));
                -1
            }
        }
    }
}

/// PyTuple_SetItem: STEALS a reference on success (CPython contract).
#[no_mangle]
pub unsafe extern "C" fn PyTuple_SetItem(o: PyObjPtr, i: isize, v: PyObjPtr) -> c_int {
    let h = o as CeHandle;
    unsafe {
        let ok = matches!((&*h).payload.borrow().as_ref(), Some(CePayload::Tuple(_)));
        if !ok {
            let tn = type_name_of(h);
            err_set("SystemError", format!("expected tuple, not '{tn}'"));
            return -1;
        }
        if let Some(CePayload::Tuple(items)) = &mut *(&*h).payload.borrow_mut() {
            let idx = i as usize;
            while items.len() <= idx {
                items.push(py_none());
            }
            items[idx] = v as CeHandle;
        }
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyTuple_GetItem(o: PyObjPtr, i: isize) -> PyObjPtr {
    let h = o as CeHandle;
    unsafe {
        if let Some(CePayload::Tuple(items)) = (&*h).payload.borrow().as_ref() {
            if i >= 0 && (i as usize) < items.len() {
                let r = items[i as usize];
                ce_newref(r); // borrowed -> new ref
                return r as PyObjPtr;
            }
        }
        err_set("IndexError", "tuple index out of range");
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyList_New(n: isize) -> PyObjPtr {
    new_handle(
        CeKind::List,
        CePayload::List(vec![py_none(); n.max(0) as usize]),
    ) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyList_Append(o: PyObjPtr, v: PyObjPtr) -> c_int {
    let h = o as CeHandle;
    unsafe {
        if let Some(CePayload::List(items)) = &mut *(&*h).payload.borrow_mut() {
            items.push(v as CeHandle);
            0
        } else {
            let tn = type_name_of(h);
            err_set("SystemError", format!("expected list, not '{tn}'"));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_New() -> PyObjPtr {
    new_handle(CeKind::Dict, CePayload::Dict(ap_obj::dict::PyDict::new())) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_SetItemString(o: PyObjPtr, k: *const c_char, v: PyObjPtr) -> c_int {
    if k.is_null() {
        err_set("SystemError", "PyDict_SetItemString: NULL key");
        return -1;
    }
    let key = std::ffi::CStr::from_ptr(k).to_string_lossy().into_owned();
    let h = o as CeHandle;
    unsafe {
        if let Some(CePayload::Dict(d)) = &mut *(&*h).payload.borrow_mut() {
            let _ = d.set(
                ap_obj::value::Value::Str(ap_obj::value::PyString::new(key)),
                crate::value_conv::to_value(v as CeHandle).unwrap_or(ap_obj::value::Value::None),
            );
            0
        } else {
            let tn = type_name_of(h);
            err_set("SystemError", format!("expected dict, not '{tn}'"));
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Module API
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn PyModule_Create2(def_base: PyObjPtr, _api_version: c_int) -> PyObjPtr {
    crate::api::module_create(def_base) as PyObjPtr
}

/// PyModule_AddObject (steals on success).
#[no_mangle]
pub unsafe extern "C" fn PyModule_AddObject(
    m: PyObjPtr,
    name: *const c_char,
    v: PyObjPtr,
) -> c_int {
    if name.is_null() {
        return -1;
    }
    let key = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    let h = m as CeHandle;
    unsafe {
        if let Some(CePayload::Module { ns, .. }) = &mut *(&*h).payload.borrow_mut() {
            let _ = ns.set(
                ap_obj::value::Value::Str(ap_obj::value::PyString::new(key)),
                crate::value_conv::to_value(v as CeHandle).unwrap_or(ap_obj::value::Value::None),
            );
            0
        } else {
            -1
        }
    }
}

// ---------------------------------------------------------------------------
// Type API (PyType_FromSpec — the survey's measured PyO3 path)
// ---------------------------------------------------------------------------

/// PyType_Ready: a no-op on S0 (heap types only; static PyTypeObject
/// layouts are OURS and don't need CPython's ready sequence).
#[no_mangle]
pub unsafe extern "C" fn PyType_Ready(_t: PyObjPtr) -> c_int {
    0
}

/// PyObject_TypeCheck-ish helper the header shim uses for Py_TYPE.
#[no_mangle]
pub unsafe extern "C" fn apx_TypeName(o: PyObjPtr) -> *const c_char {
    let h = o as CeHandle;
    if h.is_null() {
        return b"?\0".as_ptr() as *const c_char;
    }
    // Names are static; a per-kind static table keeps the pointer valid.
    unsafe { type_name_cstr(type_name_of(h)) }
}

unsafe fn type_name_of(h: CeHandle) -> String {
    if h.is_null() {
        return "<null>".to_string();
    }
    let o = &*h;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Type { spec_index }) => crate::slots::record_clone(*spec_index)
            .map(|s| s.name)
            .unwrap_or_else(|| o.type_name().to_string()),
        Some(CePayload::Instance { class_spec, .. }) => crate::slots::record_clone(*class_spec)
            .map(|s| s.name)
            .unwrap_or_else(|| "object".to_string()),
        _ => o.type_name().to_string(),
    }
}

unsafe fn type_name_cstr(_n: String) -> *const c_char {
    // Static names only for the S0 error paths (int/str/tuple/...); heap
    // type names route through PyErr_Format messages instead.
    b"object\0".as_ptr() as *const c_char
}

// ---------------------------------------------------------------------------
// Error protocol
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn PyErr_SetString(exc: PyObjPtr, msg: *const c_char) {
    // S0: exc is accepted for API shape; the CLASS name is carried by the
    // msg convention on S0 (extension passes PyExc_ValueError etc. — the
    // header shim maps those to apx exception tokens carrying class names).
    let m = if msg.is_null() {
        "".to_string()
    } else {
        std::ffi::CStr::from_ptr(msg).to_string_lossy().into_owned()
    };
    // The exception token is a Str handle carrying the class name (header
    // shim: apx_PyExc_<Name>() returns those).
    let class = unsafe {
        if exc.is_null() {
            "Exception".to_string()
        } else if let Some(CePayload::Str(s)) = (&*(exc as CeHandle)).payload.borrow().as_ref() {
            s.clone()
        } else {
            "Exception".to_string()
        }
    };
    err_set(&class, m);
}

#[no_mangle]
pub unsafe extern "C" fn PyErr_Clear() {
    crate::state::err_clear();
}

#[no_mangle]
pub unsafe extern "C" fn PyErr_Occurred() -> PyObjPtr {
    if crate::state::err_is_set() {
        // Truthy token (CPython returns the exception handle).
        py_true() as PyObjPtr
    } else {
        std::ptr::null_mut()
    }
}

/// apx exception tokens: the header shim's PyExc_<Name> macros call these;
/// each returns a Str handle carrying the class name (interned per call —
/// cheap and identity-stable for the S0 raise path).
macro_rules! apx_exc {
    ($fn_name:ident, $class:expr) => {
        #[no_mangle]
        pub extern "C" fn $fn_name() -> PyObjPtr {
            new_handle(CeKind::Str, CePayload::Str($class.to_string())) as PyObjPtr
        }
    };
}

apx_exc!(apx_PyExc_ValueError, "ValueError");
apx_exc!(apx_PyExc_TypeError, "TypeError");
apx_exc!(apx_PyExc_RuntimeError, "RuntimeError");
apx_exc!(apx_PyExc_IndexError, "IndexError");
apx_exc!(apx_PyExc_KeyError, "KeyError");
apx_exc!(apx_PyExc_AttributeError, "AttributeError");
apx_exc!(apx_PyExc_NotImplementedError, "NotImplementedError");
apx_exc!(apx_PyExc_OverflowError, "OverflowError");
apx_exc!(apx_PyExc_ZeroDivisionError, "ZeroDivisionError");
apx_exc!(apx_PyExc_MemoryError, "MemoryError");
apx_exc!(apx_PyExc_SystemError, "SystemError");

/// PyObject_Call (S0: no-arg + tuple-arg forms; kwargs arrive S1).
#[no_mangle]
pub unsafe extern "C" fn apx_PyObject_Call(
    _callable: PyObjPtr,
    _args: PyObjPtr,
    _kwds: PyObjPtr,
) -> PyObjPtr {
    // S0 spike: extensions in scope never call back into Python values
    // (the PyO3-shaped module uses slots + methods only). Full call-back
    // surface is S1 (bridge may need the interpreter handle threaded).
    err_set(
        "NotImplementedError",
        "PyObject_Call back into the runtime is S1 surface (S0 spike boundary)",
    );
    std::ptr::null_mut()
}

// Keep c_void referenced (the header shim uses it through this module).
#[allow(unused)]
fn _unused(_: *mut c_void) {}

/// from_value re-export used by the bridge (public for ap-run).
pub fn handle_from_value(v: &ap_obj::value::Value) -> CeHandle {
    from_value(v)
}

// ---------------------------------------------------------------------------
// Shim-backend registrations (called from apx_shim.c)
// ---------------------------------------------------------------------------

/// apx_PyModule_AddFunctions backend: register + install the ns marker.
#[no_mangle]
pub unsafe extern "C" fn apx_method_register(
    m: PyObjPtr,
    name: *const c_char,
    flags: c_int,
    meth: crate::ffi_types::PyCFunctionRaw,
) -> c_int {
    if name.is_null() || m.is_null() {
        return -1;
    }
    let n = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
    let h = m as CeHandle;
    unsafe {
        if let Some(CePayload::Module { name: mod_name, ns }) =
            &mut *(&*h).payload.borrow_mut()
        {
            let mn = mod_name.clone();
            let _ = ns.set(
                ap_obj::value::Value::Str(ap_obj::value::PyString::new(n.clone())),
                crate::state::marker_for(&mn, &n),
            );
        } else {
            return -1;
        }
    }
    crate::api::method_registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(crate::api::MethodRecord {
            module: {
                // module name from the handle payload
                let o = &*(m as CeHandle);
                match o.payload.borrow().as_ref() {
                    Some(CePayload::Module { name, .. }) => name.clone(),
                    _ => String::new(),
                }
            },
            name: n,
            flags: flags as i32,
            f: meth,
        });
    0
}

/// apx_PyType_FromSpec backend: normalized registration.
#[no_mangle]
pub unsafe extern "C" fn apx_type_register(
    name: *const c_char,
    tp_new: *mut c_void,
    tp_init: *mut c_void,
    tp_dealloc: *mut c_void,
    methods: *mut c_void,
    getsets: *mut c_void,
) -> c_long {
    use crate::ffi_types::{GetterRaw, InitRaw, PyCFunctionRaw, SetterRaw};
    if name.is_null() {
        err_set("SystemError", "PyType_FromSpec: NULL name");
        return -1;
    }
    let n = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
    // Walk the raw C tables (the shim passed the ORIGINAL pointers).
    let mut meths: Vec<(String, i32, PyCFunctionRaw)> = Vec::new();
    if !methods.is_null() {
        let mut md = methods as *const ApxCMethodDef;
        while !(*md).ml_name.is_null() {
            let mn = std::ffi::CStr::from_ptr((*md).ml_name)
                .to_string_lossy()
                .into_owned();
            meths.push((
                mn,
                (*md).ml_flags,
                (*md).ml_meth.expect("ml_meth required on S0"),
            ));
            md = md.add(1);
        }
    }
    let mut gs: Vec<(String, Option<crate::ffi_types::GetterRaw>, Option<crate::ffi_types::SetterRaw>)> =
        Vec::new();
    if !getsets.is_null() {
        let mut gd = getsets as *const ApxCGetSetDef;
        while !(*gd).name.is_null() {
            let gn = std::ffi::CStr::from_ptr((*gd).name)
                .to_string_lossy()
                .into_owned();
            gs.push((gn, (*gd).get, (*gd).set));
            gd = gd.add(1);
        }
    }
    let newf: Option<PyCFunctionRaw> =
        if tp_new.is_null() { None } else { Some(unsafe { std::mem::transmute(tp_new) }) };
    let initf: Option<InitRaw> =
        if tp_init.is_null() { None } else { Some(unsafe { std::mem::transmute(tp_init) }) };
    let delf: Option<PyCFunctionRaw> =
        if tp_dealloc.is_null() { None } else { Some(unsafe { std::mem::transmute(tp_dealloc) }) };
    match crate::api::type_from_spec_normalized(
        &n,
        newf,
        initf,
        delf,
        meths,
        gs,
        Vec::new(),
    ) {
        Ok(idx) => idx as c_long,
        Err(msg) => {
            err_set("SystemError", msg);
            -1
        }
    }
}

/// The raw C table layouts (mirror apx_python.h; kept local so this module
/// does not include C headers).
#[repr(C)]
struct ApxCMethodDef {
    ml_name: *const c_char,
    ml_meth: Option<crate::ffi_types::PyCFunctionRaw>,
    ml_flags: c_int,
    ml_doc: *const c_char,
}

#[repr(C)]
struct ApxCGetSetDef {
    name: *const c_char,
    get: Option<crate::ffi_types::GetterRaw>,
    set: Option<crate::ffi_types::SetterRaw>,
    doc: *mut c_void,
    closure: *mut c_void,
}

/// Spec-index escape for tests/bridge: the CLASS Value of a FromSpec type.
#[no_mangle]
pub unsafe extern "C" fn apx_spec_class_value(spec_index: c_long) -> PyObjPtr {
    match crate::slots::class_value_for_spec(spec_index as usize) {
        Some(v) => from_value(&v) as PyObjPtr,
        None => std::ptr::null_mut(),
    }
}

/// The shim's module-init runner re-export (C lives in apx_shim.c).
#[no_mangle]
pub extern "C" fn apx_init_count() -> c_int {
    // apx_run_module_init is defined in C; this count fn exists so the
    // runtime can enumerate before probing. Backed by the C static table
    // through a tiny C helper below.
    extern "C" {
        fn apx_c_init_count() -> c_int;
    }
    unsafe { apx_c_init_count() }
}

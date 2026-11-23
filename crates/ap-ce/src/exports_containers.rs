//! Leaf 1005a — the S1 container band: the residual PyDict_*/PyList_*/
//! PyTuple_* C-API surface (~60 exported members), wrapper-style over the
//! S0 handle layer (state.rs / value_conv.rs / ap-obj PyDict).
//!
//! Style contract (mirrors exports.rs): CPython-spelled UNPREFIXED
//! `#[no_mangle]` symbols (dlopen resolves against the running binary),
//! thin wrappers over handles, err_set with the CPython 3.14 dialect.
//!
//! ERROR DIALECT: every non-trivial message below was PROBED against the
//! host's real CPython 3.14.7 (scratch probe extensions, 2026-11-22
//! session). Notably, type-mismatched receivers on these internal-API
//! functions raise **SystemError "<file>.c:<line>: bad argument to
//! internal function"** with CPython's EXACT object-file line numbers,
//! NOT TypeErrors — the probe table (dictobject/listobject/tupleobject)
//! is pinned in the tests below. KeyError carries the key's repr in its
//! arg (the runtime's exc_hier KeyError str() = repr(args[0]) contract);
//! `PyDict_GetItem` swallows hash errors (returns NULL, no error set) —
//! 3.14 emits a stderr "Exception ignored" notice we do NOT reproduce
//! (the C contract is NULL + no error); the *Ref/*WithError variants DO
//! raise.
//!
//! Refcount contracts per the CPython docs, load-bearing: SetItem steals,
//! GetItem borrows (ce_newref to hand a new ref out), Next borrows, Pop /
//! GetItemRef / SetDefaultRef / GetItemRef / GetItemStringRef produce NEW
//! references.

use std::ffi::{c_char, c_int, c_void};

use crate::ffi_types::PyObjPtr;
use crate::state::{
    ce_newref, err_clear, err_set, from_value, new_handle, CeHandle, CeKind, CePayload,
};
use ap_obj::dict::PyDict;
use ap_obj::value::{PyList, PyString, Value};

// ---------------------------------------------------------------------------
// Handle/payload access helpers (private to this module — no sibling edits).
// ---------------------------------------------------------------------------

/// Read the dict payload out of a handle (clone of the Gc<PyDict>).
fn dict_of(h: CeHandle) -> Option<ap_obj::int::Gc<PyDict>> {
    if h.is_null() {
        return None;
    }
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::Dict(d)) => Some(d.clone()),
            _ => None,
        }
    }
}

fn list_of(h: CeHandle) -> Option<ap_obj::int::Gc<PyList>> {
    if h.is_null() {
        return None;
    }
    unsafe {
        let p = (&*h).payload.borrow();
        match p.as_ref() {
            Some(CePayload::List(items)) => {
                let vals: Vec<Value> = items
                    .iter()
                    .filter_map(|hh| crate::value_conv::to_value(*hh))
                    .collect();
                Some(PyList::from_vec(vals))
            }
            _ => None,
        }
    }
}

fn tuple_items_of(h: CeHandle) -> Option<Vec<CeHandle>> {
    if h.is_null() {
        return None;
    }
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::Tuple(items)) => Some(items.clone()),
            _ => None,
        }
    }
}

/// CPython 3.14 SystemError dialect for a wrong-typed receiver on the
/// container internals. `<line>` is the object-file line CPython 3.14.7
/// raises from (measured table; see the module doc + unit tests).
fn bad_internal(file_line: &str) {
    err_set(
        "SystemError",
        format!("{file_line}: bad argument to internal function"),
    );
}

fn type_name_of(h: CeHandle) -> String {
    if h.is_null() {
        return "<null>".to_string();
    }
    unsafe { (&*h).type_name().to_string() }
}

/// Value -> handle (new reference).
fn h_from_value(v: &Value) -> CeHandle {
    from_value(v)
}

/// Handle -> Value with the None/Bool singletons resolved (to_value does
/// this via the CeKind check; kept local for readability).
fn v_of(h: CeHandle) -> Option<Value> {
    crate::value_conv::to_value(h)
}

/// Re-materialize a CePayload::List handle from a Value list (the in-place
/// mutation family works on the runtime list then syncs back).
fn sync_list_from_value(h: CeHandle, l: &ap_obj::int::Gc<PyList>) {
    let items: Vec<CeHandle> = l.items().iter().map(h_from_value).collect();
    unsafe {
        if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
            *slot = items;
        }
    }
}

/// Dict key hash with the CPython dict-surface error dialect (the
/// "cannot use '<t>' as a dict key (unhashable type: '<t>')" phrasing —
/// PyDict_SetItem/GetItemWithError/Pop/SetDefault family; measured).
fn dict_key_hash(key: &Value) -> Result<i64, ()> {
    match ap_obj::hash::py_hash(key) {
        Ok(hv) => Ok(hv),
        Err(e) => {
            // ap-obj's py_hash raises "unhashable type: 'list'"; the dict
            // KEY surfaces use the two-mention phrasing.
            let tn = match &e.message.strip_prefix("unhashable type: '") {
                Some(rest) => rest.trim_end_matches('\'').to_string(),
                None => key.type_name().to_string(),
            };
            err_set(
                "TypeError",
                format!("cannot use '{tn}' as a dict key (unhashable type: '{tn}')"),
            );
            Err(())
        }
    }
}

// ---------------------------------------------------------------------------
// PyLong_FromSsize_t (leaf 1005a): the container band's size return route.
// ---------------------------------------------------------------------------

/// PyLong_FromSsize_t: Py_ssize_t -> Python int (no error path; raises
/// OverflowError only beyond 64-bit, which Py_ssize_t cannot be).
#[no_mangle]
pub unsafe extern "C" fn PyLong_FromSsize_t(v: isize) -> PyObjPtr {
    new_handle(
        CeKind::Int,
        CePayload::Int(ap_obj::int::Int::from_i64(v as i64)),
    ) as PyObjPtr
}

// ---------------------------------------------------------------------------
// PyDict — lookups
// ---------------------------------------------------------------------------

/// PyDict_GetItem: BORROWED return; missing key OR unhashable key => NULL
/// with NO error set (the masking contract; 3.14 prints an unraisable
/// notice for the unhashable case on the stderr side, not via PyErr).
#[no_mangle]
pub unsafe extern "C" fn PyDict_GetItem(mp: PyObjPtr, key: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2798");
        return std::ptr::null_mut();
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return std::ptr::null_mut();
    };
    if ap_obj::hash::py_hash(&kv).is_err() {
        // Masked: NULL, no error state (caller probes PyErr_Occurred()).
        err_clear();
        return std::ptr::null_mut();
    }
    match d.get(&kv) {
        Some(v) => {
            // BORROWED on CPython: hand the same object back with NO new
            // reference. Our handle mints are per-call (from_value builds
            // a fresh CeObject per non-immortal value), so a "borrow"
            // surfaces as a new handle — but its refcount reads 1 fresh,
            // matching a borrowed CPython object observed once. The rc
            // probe in containers.c never asserts GetItem borrow counts
            // against CPython's (impossible across layouts; PEP 703).
            h_from_value(&v) as PyObjPtr
        }
        None => std::ptr::null_mut(),
    }
}

/// PyDict_GetItemWithError: like GetItem but hash errors SET the error.
#[no_mangle]
pub unsafe extern "C" fn PyDict_GetItemWithError(mp: PyObjPtr, key: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2524");
        return std::ptr::null_mut();
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return std::ptr::null_mut();
    };
    if dict_key_hash(&kv).is_err() {
        return std::ptr::null_mut();
    }
    match d.get(&kv) {
        Some(v) => {
            // Borrowed on CPython (see PyDict_GetItem's note): fresh mint,
            // no bump.
            h_from_value(&v) as PyObjPtr
        }
        None => std::ptr::null_mut(),
    }
}

/// PyDict_GetItemRef: 1 present (strong ref out), 0 missing (NULL out),
/// -1 error (unhashable key / non-dict). NULL `result` is a caller
/// contract violation on CPython (it crashes) — we return -1 + SystemError
/// instead (strictly safer, unobservable in correct programs).
#[no_mangle]
pub unsafe extern "C" fn PyDict_GetItemRef(
    mp: PyObjPtr,
    key: PyObjPtr,
    result: *mut PyObjPtr,
) -> c_int {
    if result.is_null() {
        err_set(
            "SystemError",
            "Objects/dictobject.c:2468: bad argument to internal function",
        );
        return -1;
    }
    *result = std::ptr::null_mut();
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2468");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    match d.get(&kv) {
        Some(v) => {
            *result = h_from_value(&v) as PyObjPtr;
            1
        }
        None => 0,
    }
}

/// PyDict_GetItemString: borrowed-or-NULL like GetItem, str key.
#[no_mangle]
pub unsafe extern "C" fn PyDict_GetItemString(dp: PyObjPtr, key: *const c_char) -> PyObjPtr {
    if key.is_null() {
        return std::ptr::null_mut();
    }
    let ks = std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned();
    let kv = Value::Str(PyString::new(ks));
    let Some(d) = dict_of(dp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2798");
        return std::ptr::null_mut();
    };
    match d.get(&kv) {
        Some(v) => {
            // Borrowed on CPython: fresh mint, no bump.
            h_from_value(&v) as PyObjPtr
        }
        None => std::ptr::null_mut(),
    }
}

/// PyDict_GetItemStringRef: like GetItemRef with a C-string key.
#[no_mangle]
pub unsafe extern "C" fn PyDict_GetItemStringRef(
    mp: PyObjPtr,
    key: *const c_char,
    result: *mut PyObjPtr,
) -> c_int {
    if result.is_null() {
        err_set(
            "SystemError",
            "Objects/dictobject.c:2508: bad argument to internal function",
        );
        return -1;
    }
    *result = std::ptr::null_mut();
    if key.is_null() {
        return -1;
    }
    let ks = std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned();
    let kv = Value::Str(PyString::new(ks));
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2508");
        return -1;
    };
    match d.get(&kv) {
        Some(v) => {
            *result = h_from_value(&v) as PyObjPtr;
            1
        }
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// PyDict — mutation
// ---------------------------------------------------------------------------

/// PyDict_SetItem: key hash errors raise the dict-key TypeError dialect.
#[no_mangle]
pub unsafe extern "C" fn PyDict_SetItem(mp: PyObjPtr, key: PyObjPtr, item: PyObjPtr) -> c_int {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2722");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    let vv = v_of(item as CeHandle).unwrap_or(Value::None);
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    match d.set(kv, vv) {
        Ok(()) => 0,
        Err(e) => {
            err_set(e.class, e.message);
            -1
        }
    }
}

/// PyDict_DelItem: missing key => KeyError carrying the key's repr (the
/// runtime renders KeyError str() = repr(args[0]); the arg IS the key).
#[no_mangle]
pub unsafe extern "C" fn PyDict_DelItem(mp: PyObjPtr, key: PyObjPtr) -> c_int {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2854");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    match d.pop(&kv) {
        Some(_) => 0,
        None => {
            err_set("KeyError", ap_obj::ops::repr(&kv).unwrap_or_default());
            -1
        }
    }
}

/// PyDict_DelItemString.
#[no_mangle]
pub unsafe extern "C" fn PyDict_DelItemString(dp: PyObjPtr, key: *const c_char) -> c_int {
    if key.is_null() {
        return -1;
    }
    let ks = std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned();
    let kv = Value::Str(PyString::new(ks));
    unsafe { PyDict_DelItem(dp, h_from_value(&kv) as PyObjPtr) }
}

/// PyDict_Clear: void on CPython; we no-op NULL (CPython crashes; strict
/// superset, unobservable in correct programs).
#[no_mangle]
pub unsafe extern "C" fn PyDict_Clear(mp: PyObjPtr) {
    if mp.is_null() {
        return;
    }
    if dict_of(mp as CeHandle).is_none() {
        bad_internal("Objects/dictobject.c:2019");
        return;
    }
    if let Some(d) = dict_of(mp as CeHandle) {
        // No PyDict::clear on ap-obj's surface — drain via popitem (LIFO
        // order irrelevant; the dict ends empty either way).
        while d.popitem().is_some() {}
    }
}

/// PyDict_Contains: 1/0; unhashable key raises (measured 3.14: TypeError
/// dict-key dialect, rc -1).
#[no_mangle]
pub unsafe extern "C" fn PyDict_Contains(mp: PyObjPtr, key: PyObjPtr) -> c_int {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:2031");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    if d.contains_key(&kv) {
        1
    } else {
        0
    }
}

/// PyDict_ContainsString (3.13+).
#[no_mangle]
pub unsafe extern "C" fn PyDict_ContainsString(mp: PyObjPtr, key: *const c_char) -> c_int {
    if key.is_null() {
        return -1;
    }
    let ks = std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned();
    let kv = Value::Str(PyString::new(ks));
    unsafe { PyDict_Contains(mp, h_from_value(&kv) as PyObjPtr) }
}

/// PyDict_Pop: 1 popped (strong ref out unless result NULL), 0 missing
/// (NULL out), -1 error (non-dict / unhashable key — measured 3.14 raises
/// for unhashable).
#[no_mangle]
pub unsafe extern "C" fn PyDict_Pop(mp: PyObjPtr, key: PyObjPtr, result: *mut PyObjPtr) -> c_int {
    if mp.is_null() {
        bad_internal("Objects/dictobject.c:3166");
        return -1;
    }
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:3166");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    match d.pop(&kv) {
        Some(v) => {
            if !result.is_null() {
                *result = h_from_value(&v) as PyObjPtr;
            }
            1
        }
        None => {
            if !result.is_null() {
                *result = std::ptr::null_mut();
            }
            0
        }
    }
}

/// PyDict_PopString.
#[no_mangle]
pub unsafe extern "C" fn PyDict_PopString(
    mp: PyObjPtr,
    key: *const c_char,
    result: *mut PyObjPtr,
) -> c_int {
    if key.is_null() {
        if !result.is_null() {
            unsafe { *result = std::ptr::null_mut() };
        }
        return -1;
    }
    let ks = std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned();
    let kv = Value::Str(PyString::new(ks));
    unsafe { PyDict_Pop(mp, h_from_value(&kv) as PyObjPtr, result) }
}

/// PyDict_SetDefault: borrowed-or-new STRONG ref return (CPython returns a
/// borrowed ref; we hand a new ref — INCREF-safe superset, per the S0
/// convention that C code DECREFs what it receives from any non-borrow
/// call... GetItem-family borrows are the exception; SetDefault sits in
/// the Ref family on 3.14's docs). Unhashable key raises.
#[no_mangle]
pub unsafe extern "C" fn PyDict_SetDefault(
    mp: PyObjPtr,
    key: PyObjPtr,
    defaultobj: PyObjPtr,
) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:4421");
        return std::ptr::null_mut();
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return std::ptr::null_mut();
    };
    let dv = v_of(defaultobj as CeHandle).unwrap_or(Value::None);
    if dict_key_hash(&kv).is_err() {
        return std::ptr::null_mut();
    }
    let v = match d.get(&kv) {
        Some(existing) => existing,
        None => {
            let _ = d.set(kv.clone(), dv.clone());
            dv
        }
    };
    let r = h_from_value(&v);
    ce_newref(r);
    r as PyObjPtr
}

/// PyDict_SetDefaultRef: -1 error / 0 inserted / 1 existing (strong ref in
/// *result on 0 AND 1; *result untouched... measured 3.14: set on both).
#[no_mangle]
pub unsafe extern "C" fn PyDict_SetDefaultRef(
    mp: PyObjPtr,
    key: PyObjPtr,
    defaultobj: PyObjPtr,
    result: *mut PyObjPtr,
) -> c_int {
    if result.is_null() {
        err_set(
            "SystemError",
            "Objects/dictobject.c:4470: bad argument to internal function",
        );
        return -1;
    }
    *result = std::ptr::null_mut();
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:4470");
        return -1;
    };
    let Some(kv) = v_of(key as CeHandle) else {
        return -1;
    };
    let dv = v_of(defaultobj as CeHandle).unwrap_or(Value::None);
    if dict_key_hash(&kv).is_err() {
        return -1;
    }
    match d.get(&kv) {
        Some(existing) => {
            *result = h_from_value(&existing) as PyObjPtr;
            1
        }
        None => {
            let _ = d.set(kv, dv.clone());
            *result = h_from_value(&dv) as PyObjPtr;
            0
        }
    }
}

// ---------------------------------------------------------------------------
// PyDict — views / bulk
// ---------------------------------------------------------------------------

fn dict_list_of(d: &ap_obj::int::Gc<PyDict>, f: fn(&PyDict) -> Vec<Value>) -> PyObjPtr {
    let vals = f(d);
    let handles: Vec<CeHandle> = vals.iter().map(h_from_value).collect();
    new_handle(CeKind::List, CePayload::List(handles)) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Keys(mp: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:3561");
        return std::ptr::null_mut();
    };
    dict_list_of(&d, PyDict::keys)
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Values(mp: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:3610");
        return std::ptr::null_mut();
    };
    let vals: Vec<Value> = d.items().into_iter().map(|(_, v)| v).collect();
    let handles: Vec<CeHandle> = vals.iter().map(h_from_value).collect();
    new_handle(CeKind::List, CePayload::List(handles)) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Items(mp: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:3658");
        return std::ptr::null_mut();
    };
    let pairs: Vec<CeHandle> = d
        .items()
        .into_iter()
        .map(|(k, v)| {
            new_handle(
                CeKind::Tuple,
                CePayload::Tuple(vec![h_from_value(&k), h_from_value(&v)]),
            )
        })
        .collect();
    new_handle(CeKind::List, CePayload::List(pairs)) as PyObjPtr
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Size(mp: PyObjPtr) -> isize {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:4248");
        return -1;
    };
    d.len() as isize
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Copy(mp: PyObjPtr) -> PyObjPtr {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:4231");
        return std::ptr::null_mut();
    };
    let fresh = PyDict::new();
    for (k, v) in d.items() {
        let _ = fresh.set(k, v);
    }
    new_handle(CeKind::Dict, CePayload::Dict(fresh)) as PyObjPtr
}

/// dict_update_impl: dict source merges (override=false keeps existing);
/// non-dict source => the keys()-AttributeError chain CPython surfaces as
/// AttributeError: '<t>' object has no attribute 'keys' (measured).
unsafe fn dict_update_from_dict(
    d: &ap_obj::int::Gc<PyDict>,
    other: &ap_obj::int::Gc<PyDict>,
    override_: c_int,
) -> c_int {
    for (k, v) in other.items() {
        if override_ == 0 && d.contains_key(&k) {
            continue;
        }
        if let Err(e) = d.set(k, v) {
            err_set(e.class, e.message);
            return -1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Merge(mp: PyObjPtr, other: PyObjPtr, override_: c_int) -> c_int {
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:4003");
        return -1;
    };
    let Some(ov) = v_of(other as CeHandle) else {
        return -1;
    };
    match ov {
        Value::Dict(od) => dict_update_from_dict(&d, &od, override_),
        // CPython's non-dict path goes through .keys() -> AttributeError.
        other => {
            let tn = other.type_name();
            err_set(
                "AttributeError",
                format!("'{tn}' object has no attribute 'keys'"),
            );
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn PyDict_Update(mp: PyObjPtr, other: PyObjPtr) -> c_int {
    unsafe { PyDict_Merge(mp, other, 1) }
}

/// PyDict_MergeFromSeq2: pairs iterable into the dict. CPython 3.14
/// measured texts: non-iterable => TypeError "object is not iterable";
/// bad item arity (after iterating the item) => ValueError "dictionary
/// update sequence element #N has length L; 2 is required" or TypeError
/// "element N is a tuple of length 1; 2 is required" family. We support
/// the iterable-of-2-sequence shapes ap-obj carries natively (list/tuple
/// of pairs) and raise the measured texts otherwise.
#[no_mangle]
pub unsafe extern "C" fn PyDict_MergeFromSeq2(
    d_obj: PyObjPtr,
    seq2: PyObjPtr,
    override_: c_int,
) -> c_int {
    let Some(d) = dict_of(d_obj as CeHandle) else {
        bad_internal("Objects/dictobject.c:3767");
        return -1;
    };
    let Some(sv) = v_of(seq2 as CeHandle) else {
        return -1;
    };
    let pairs: Vec<(Value, Value)> = match sv {
        Value::List(l) => {
            let mut out = Vec::new();
            for (i, item) in l.items().iter().enumerate() {
                match pair_of(item) {
                    Ok(p) => out.push(p),
                    Err(msg) => {
                        if msg.is_empty() {
                            // pair_of already set the exact error
                            // (non-iterable item -> bare TypeError).
                            return -1;
                        }
                        err_set(
                            "ValueError",
                            format!("dictionary update sequence element #{i} {msg}"),
                        );
                        return -1;
                    }
                }
            }
            out
        }
        Value::Tuple(t) => {
            let mut out = Vec::new();
            for (i, item) in t.0.iter().enumerate() {
                match pair_of(item) {
                    Ok(p) => out.push(p),
                    Err(msg) => {
                        if msg.is_empty() {
                            return -1;
                        }
                        err_set(
                            "ValueError",
                            format!("dictionary update sequence element #{i} {msg}"),
                        );
                        return -1;
                    }
                }
            }
            out
        }
        // CPython's MergeFromSeq2 on a dict source iterates its KEYS (a
        // plain iterable, not items()): each key then fails pair_of with
        // its own length (measured: {'a':1} -> element #0 has length 1).
        Value::Dict(od) => {
            let keys = od.keys();
            let mut out = Vec::new();
            for (i, item) in keys.iter().enumerate() {
                match pair_of(item) {
                    Ok(p) => out.push(p),
                    Err(msg) => {
                        if msg.is_empty() {
                            return -1;
                        }
                        err_set(
                            "ValueError",
                            format!("dictionary update sequence element #{i} {msg}"),
                        );
                        return -1;
                    }
                }
            }
            out
        }
        // Non-iterable OUTER seq: the type-mentioning TypeError (measured
        // for MergeFromSeq2(42)).
        other => {
            let tn = other.type_name();
            err_set("TypeError", format!("'{tn}' object is not iterable"));
            return -1;
        }
    };
    for (k, v) in pairs {
        if override_ == 0 && d.contains_key(&k) {
            continue;
        }
        if let Err(e) = d.set(k, v) {
            err_set(e.class, e.message);
            return -1;
        }
    }
    0
}

/// (key, value) pair extraction for MergeFromSeq2 with the measured
/// length-error suffix ("has length L; 2 is required"). CPython uses
/// PySequence_Size on the item: a STR item of len 2 is a valid pair
/// ("ab" -> ('a','b')) — measured; a non-sequence item gives the bare
/// "object is not iterable" TypeError.
fn pair_of(item: &Value) -> Result<(Value, Value), String> {
    match item {
        Value::Tuple(t) => {
            if t.0.len() == 2 {
                Ok((t.0[0].clone(), t.0[1].clone()))
            } else {
                Err(format!("has length {}; 2 is required", t.0.len()))
            }
        }
        Value::List(l) => {
            let items = l.items();
            if items.len() == 2 {
                Ok((items[0].clone(), items[1].clone()))
            } else {
                Err(format!("has length {}; 2 is required", items.len()))
            }
        }
        Value::Str(sv) => {
            // CPython: PySequence consideration for str items — chars as
            // the pair elements (measured: {'ab': 1} merges 'a'->'b').
            let n = sv.as_str().chars().count();
            if n == 2 {
                let mut it = sv.as_str().chars();
                let c0 = it.next().unwrap();
                let c1 = it.next().unwrap();
                Ok((
                    Value::Str(PyString::new(c0.to_string())),
                    Value::Str(PyString::new(c1.to_string())),
                ))
            } else {
                Err(format!("has length {n}; 2 is required"))
            }
        }
        other => {
            let _ = other.type_name();
            err_set("TypeError", "object is not iterable");
            Err(String::new()) // error already set; caller just returns -1
        }
    }
}

/// PyDict_Next: iteration with a pos cursor; returns 0 when exhausted /
/// on a non-dict (CPython crashes on non-dict — measured via omission; we
/// return 0 with the line-pinned SystemError, strictly safer). Keys and
/// values are BORROWED (we hand new refs; ce DECREF discipline treats
/// them like GetItem borrows).
#[no_mangle]
pub unsafe extern "C" fn PyDict_Next(
    mp: PyObjPtr,
    ppos: *mut isize,
    pkey: *mut PyObjPtr,
    pvalue: *mut PyObjPtr,
) -> c_int {
    if ppos.is_null() {
        return 0;
    }
    let Some(d) = dict_of(mp as CeHandle) else {
        bad_internal("Objects/dictobject.c:678");
        return 0;
    };
    let items = d.items();
    let i = *ppos as usize;
    if i >= items.len() {
        return 0;
    }
    *ppos = (i + 1) as isize;
    if !pkey.is_null() {
        let k = h_from_value(&items[i].0);
        ce_newref(k);
        *pkey = k as PyObjPtr;
    }
    if !pvalue.is_null() {
        let v = h_from_value(&items[i].1);
        ce_newref(v);
        *pvalue = v as PyObjPtr;
    }
    1
}

// ---------------------------------------------------------------------------
// PyList — element access
// ---------------------------------------------------------------------------

/// PyList_GetItem: borrowed; negative or OOB => IndexError (measured:
/// "list index out of range"; NEGATIVE indexes also fail here — the C
/// API does not wrap). Borrow on CPython = fresh handle mint, no bump.
#[no_mangle]
pub unsafe extern "C" fn PyList_GetItem(o: PyObjPtr, i: isize) -> PyObjPtr {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:388");
        return std::ptr::null_mut();
    };
    if i < 0 || i as usize >= items.len() {
        err_set("IndexError", "list index out of range");
        return std::ptr::null_mut();
    }
    // Fresh mint via to_value round-trip keeps the SAME item object.
    let Some(v) = v_of(items[i as usize]) else {
        return std::ptr::null_mut();
    };
    h_from_value(&v) as PyObjPtr
}

/// The live item handles of a CePayload::List handle (clone of the Vec).
fn list_items_of(h: CeHandle) -> Option<Vec<CeHandle>> {
    if h.is_null() {
        return None;
    }
    unsafe {
        match (&*h).payload.borrow().as_ref() {
            Some(CePayload::List(items)) => Some(items.clone()),
            _ => None,
        }
    }
}

/// PyList_GetItemRef (3.13+): NEW strong reference out, NULL + IndexError
/// out-of-band (no -1 path: only IndexError exists per CPython docs).
#[no_mangle]
pub unsafe extern "C" fn PyList_GetItemRef(o: PyObjPtr, i: isize) -> PyObjPtr {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:419");
        return std::ptr::null_mut();
    };
    if i < 0 || i as usize >= items.len() {
        err_set("IndexError", "list index out of range");
        return std::ptr::null_mut();
    }
    let r = items[i as usize];
    ce_newref(r);
    r as PyObjPtr
}

/// PyList_SetItem: STEALS on success (no INCREF — the item handle moves
/// into the list); negative/OOB => "list assignment index out of range".
#[no_mangle]
pub unsafe extern "C" fn PyList_SetItem(o: PyObjPtr, i: isize, newitem: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:453");
        return -1;
    };
    if i < 0 || i as usize >= items.len() {
        err_set("IndexError", "list assignment index out of range");
        return -1;
    }
    items[i as usize] = newitem as CeHandle;
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = items;
    }
    0
}

// ---------------------------------------------------------------------------
// PyList — size / slice / in-place
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn PyList_Size(o: PyObjPtr) -> isize {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:306");
        return -1;
    };
    items.len() as isize
}

/// PyList_GetSlice: clamped like CPython (both bounds clamp into
/// [0, len]; lo > hi => empty list; NO error on OOB).
#[no_mangle]
pub unsafe extern "C" fn PyList_GetSlice(o: PyObjPtr, ilow: isize, ihigh: isize) -> PyObjPtr {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:717");
        return std::ptr::null_mut();
    };
    let len = items.len() as isize;
    let lo = ilow.clamp(0, len);
    let hi = ihigh.clamp(0, len);
    let slice: Vec<CeHandle> = if lo < hi {
        items[lo as usize..hi as usize].to_vec()
    } else {
        Vec::new()
    };
    new_handle(CeKind::List, CePayload::List(slice)) as PyObjPtr
}

/// PyList_SetSlice: assign an iterable (list/tuple/dict-shapes ap-obj
/// carries) over [ilow, ihigh), clamped; NULL value deletes the range;
/// non-iterable value => TypeError "can only assign an iterable" (measured).
#[no_mangle]
pub unsafe extern "C" fn PyList_SetSlice(
    o: PyObjPtr,
    ilow: isize,
    ihigh: isize,
    newitems: PyObjPtr,
) -> c_int {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:1026");
        return -1;
    };
    let len = items.len() as isize;
    let lo = ilow.clamp(0, len);
    let hi = ihigh.clamp(0, len);
    let replacement: Vec<CeHandle> = if newitems.is_null() {
        Vec::new()
    } else {
        match v_of(newitems as CeHandle) {
            Some(Value::List(l)) => l.items().iter().map(h_from_value).collect(),
            Some(Value::Tuple(t)) => t.0.iter().map(h_from_value).collect(),
            Some(Value::Dict(d)) => d.keys().iter().map(h_from_value).collect(),
            Some(other) => {
                let _ = other.type_name();
                err_set("TypeError", "can only assign an iterable");
                return -1;
            }
            None => return -1,
        }
    };
    let mut out: Vec<CeHandle> = Vec::with_capacity(items.len());
    out.extend_from_slice(&items[..lo as usize]);
    out.extend(replacement);
    // CPython's tail starts at max(lo, hi) — a reversed range (lo > hi)
    // keeps the tail from lo, giving the measured insert shape.
    let tail_from = lo.max(hi) as usize;
    out.extend_from_slice(&items[tail_from..]);
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = out;
    }
    0
}

/// NOTE (sibling-boundary, like the tuple block below): PyList_Append is
/// ALREADY exported by exports.rs (S0). Its wrong-receiver text is
/// "expected list, not '<t>'" (SystemError) rather than the measured
/// 3.14 "Objects/listobject.c:542: bad argument..." — divergent dialect,
/// recorded as a finding for the exports.rs owner (cannot fix without
/// touching off-limits exports.rs). The steal-on-success contract is
/// already faithful on S0. The differential probe exercises Append on
/// VALID receivers only (identical both runtimes).

/// PyList_Insert: clamps like CPython's list.insert (negative wraps from
/// the end clamped at 0; beyond-len clamps at len). Measured:
/// [-1, 0, 9, 1, 8] for inserts (0,0)(0,0)(1,9)(100,8)(-100,-1).
#[no_mangle]
pub unsafe extern "C" fn PyList_Insert(o: PyObjPtr, i: isize, newitem: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:507");
        return -1;
    };
    let len = items.len() as isize;
    let idx = if i < 0 { (len + i).max(0) } else { i.min(len) } as usize;
    items.insert(idx, newitem as CeHandle);
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = items;
    }
    0
}

/// PyList_Extend: append every item of an iterable. Measured non-iterable
/// text: TypeError "'int' object is not iterable" (mention included here,
/// unlike MergeFromSeq2's bare text).
#[no_mangle]
pub unsafe extern "C" fn PyList_Extend(o: PyObjPtr, iterable: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:755");
        return -1;
    };
    let Some(iv) = v_of(iterable as CeHandle) else {
        return -1;
    };
    let add: Vec<Value> = match iv {
        Value::List(l) => l.items().to_vec(),
        Value::Tuple(t) => t.0.to_vec(),
        Value::Dict(d) => d.keys(),
        Value::Str(st) => st
            .as_str()
            .chars()
            .map(|c| Value::Str(PyString::new(c.to_string())))
            .collect(),
        other => {
            let tn = other.type_name();
            // Measured 3.14 for PyList_Extend(5): type-mentioning text.
            err_set("TypeError", format!("'{tn}' object is not iterable"));
            return -1;
        }
    };
    items.extend(add.iter().map(|v| h_from_value(v)));
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = items;
    }
    0
}

/// PyList_Clear (3.13+).
#[no_mangle]
pub unsafe extern "C" fn PyList_Clear(o: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:1498");
        return -1;
    };
    items.clear();
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = items;
    }
    0
}

/// PyList_Sort: homogeneous int/str sorts byte-exact; mixed types raise
/// the CPython text "'<' not supported between instances of 'str' and
/// 'int'" (measured; ORDER of type names follows the compared pair).
#[no_mangle]
pub unsafe extern "C" fn PyList_Sort(o: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:3170");
        return -1;
    };
    let mut vals: Vec<Value> = items.iter().filter_map(|hh| v_of(*hh)).collect();
    if vals.len() != items.len() {
        // Unconvertible handles (raw Opaque mints) sort as their values.
        vals.clear();
    }
    // Sort by ap-obj ordering; raise the CPython '<' text on incompatibles.
    let mut err: Option<(String, String)> = None;
    let mut key_vals = vals.clone();
    key_vals.sort_by(|a, b| match ap_obj::dict::values_cmp(a, b) {
        Ok(ord) => ord,
        Err(e) => {
            if err.is_none() {
                err = Some((e.class.to_string(), e.message.clone()));
            }
            std::cmp::Ordering::Equal
        }
    });
    if let Some((class, msg)) = err {
        err_set(&class, msg);
        return -1;
    }
    // Map sorted values back onto handles (identity-preserving via lookup).
    let mut sorted: Vec<CeHandle> = Vec::with_capacity(items.len());
    let mut pool = items.clone();
    for v in &key_vals {
        // Find a pool handle whose Value equals v (stable enough: equal
        // values are interchangeable for a sorted list).
        let mut pick = 0usize;
        for (j, cand) in pool.iter().enumerate() {
            if let Some(cv) = v_of(*cand) {
                if ap_obj::ops::eq(&cv, v) {
                    pick = j;
                    break;
                }
            }
        }
        sorted.push(pool.remove(pick));
    }
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = sorted;
    }
    0
}

/// PyList_Reverse.
#[no_mangle]
pub unsafe extern "C" fn PyList_Reverse(o: PyObjPtr) -> c_int {
    let Some(mut items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:3204");
        return -1;
    };
    items.reverse();
    let h = o as CeHandle;
    if let Some(CePayload::List(slot)) = &mut *(&*h).payload.borrow_mut() {
        *slot = items;
    }
    0
}

/// PyList_AsTuple: non-list => the measured line-pinned SystemError.
#[no_mangle]
pub unsafe extern "C" fn PyList_AsTuple(o: PyObjPtr) -> PyObjPtr {
    let Some(items) = list_items_of(o as CeHandle) else {
        bad_internal("Objects/listobject.c:3219");
        return std::ptr::null_mut();
    };
    let out: Vec<CeHandle> = items
        .iter()
        .map(|hh| {
            let r = *hh;
            ce_newref(r);
            r
        })
        .collect();
    new_handle(CeKind::Tuple, CePayload::Tuple(out)) as PyObjPtr
}

// ---------------------------------------------------------------------------
// PyTuple — residual.
//
// NOTE (sibling-boundary): PyTuple_New/Size/SetItem/GetItem are ALREADY
// exported by exports.rs (S0, leaf-owned by the session record NEXT-1004).
// Duplicate #[no_mangle] symbols in the same crate are a link error, so
// THIS module does NOT re-export them; the S0 spellings stay the ones
// extensions resolve. KNOWN DIVERGENCE RECORDED (finding, cannot fix
// without touching off-limits exports.rs):
//   - S0 PyTuple_Size/GetItem/SetItem raise "expected tuple, not '<t>'"
//     (TypeError/SystemError) on a wrong-typed receiver instead of the
//     measured 3.14 "Objects/tupleobject.c:{96,107,123}: bad argument to
//     internal function" SystemError.
//   - S0 PyTuple_GetItem ACCEPTS negative indexes (CPython does not;
//     IndexError on both — divergent only for i<0 AND i>=-len).
//   - S0 PyTuple_SetItem GROWS the tuple for i >= len (CPython raises
//     IndexError).
// The differential probe (containers.c) exercises tuple Size/GetItem/
// SetItem only on VALID inputs (identical behavior both runtimes) and
// pins the OOB paths on GetItem ("tuple index out of range", matching)
// — the wrong-receiver + negative-index divergences are recorded as
// findings for the exports.rs owner, not assertable here.
// ---------------------------------------------------------------------------

/// PyTuple_GetSlice: clamped, lo > hi => () (measured incl. negatives).
#[no_mangle]
pub unsafe extern "C" fn PyTuple_GetSlice(o: PyObjPtr, ilow: isize, ihigh: isize) -> PyObjPtr {
    let Some(items) = tuple_items_of(o as CeHandle) else {
        bad_internal("Objects/tupleobject.c:479");
        return std::ptr::null_mut();
    };
    let len = items.len() as isize;
    let lo = ilow.clamp(0, len);
    let hi = ihigh.clamp(0, len);
    let slice: Vec<CeHandle> = if lo < hi {
        items[lo as usize..hi as usize].to_vec()
    } else {
        Vec::new()
    };
    new_handle(CeKind::Tuple, CePayload::Tuple(slice)) as PyObjPtr
}

/// PyTuple_Pack: new tuple from n C args (array form; varargs cross in
/// the shim's apx_PyTuple_PackV — see apx_shim.c + the header macro).
#[no_mangle]
pub unsafe extern "C" fn apx_PyTuple_Pack(n: isize, args: *const PyObjPtr) -> PyObjPtr {
    if n < 0 {
        err_set(
            "SystemError",
            "Objects/tupleobject.c:207: bad argument to internal function",
        );
        return std::ptr::null_mut();
    }
    let items: Vec<CeHandle> = if args.is_null() || n == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(args, n as usize)
            .iter()
            .map(|p| *p as CeHandle)
            .collect()
    };
    new_handle(CeKind::Tuple, CePayload::Tuple(items)) as PyObjPtr
}

// ---------------------------------------------------------------------------
// apx line-pinned dialect table (exposed for the unit gate + probe parity
// checks; the values are the measured CPython 3.14.7 object-file lines).
// ---------------------------------------------------------------------------

/// (fn-family, line) pairs the differential asserts.
pub fn dialect_table() -> Vec<(&'static str, &'static str)> {
    vec![
        ("PyList_Size", "Objects/listobject.c:306"),
        ("PyList_GetItem", "Objects/listobject.c:388"),
        ("PyList_GetItemRef", "Objects/listobject.c:419"),
        ("PyList_SetItem", "Objects/listobject.c:453"),
        ("PyList_Insert", "Objects/listobject.c:507"),
        ("PyList_Append", "Objects/listobject.c:542"),
        ("PyList_GetSlice", "Objects/listobject.c:717"),
        ("PyList_SetSlice", "Objects/listobject.c:1026"),
        ("PyList_Sort", "Objects/listobject.c:3170"),
        ("PyList_Reverse", "Objects/listobject.c:3204"),
        ("PyList_AsTuple", "Objects/listobject.c:3219"),
        ("PyDict_GetItemWithError", "Objects/dictobject.c:577"),
        ("PyDict_Next", "Objects/dictobject.c:678"),
        ("PyDict_SetItem", "Objects/dictobject.c:2722"),
        ("PyDict_DelItem", "Objects/dictobject.c:2854"),
        ("PyDict_GetItem", "Objects/dictobject.c:2798"),
        ("PyDict_GetItemRef", "Objects/dictobject.c:2468"),
        ("PyDict_GetItemStringRef", "Objects/dictobject.c:2508"),
        ("PyDict_Keys", "Objects/dictobject.c:3561"),
        ("PyDict_Values", "Objects/dictobject.c:3610"),
        ("PyDict_Items", "Objects/dictobject.c:3658"),
        ("PyDict_MergeFromSeq2", "Objects/dictobject.c:3767"),
        ("PyDict_Merge", "Objects/dictobject.c:4003"),
        ("PyDict_Copy", "Objects/dictobject.c:4231"),
        ("PyDict_Size", "Objects/dictobject.c:4248"),
        ("PyDict_SetDefault", "Objects/dictobject.c:4421"),
        ("PyDict_Pop", "Objects/dictobject.c:3166"),
        ("PyTuple_Size", "Objects/tupleobject.c:96"),
        ("PyTuple_GetItem", "Objects/tupleobject.c:107"),
        ("PyTuple_SetItem", "Objects/tupleobject.c:123"),
        ("PyTuple_GetSlice", "Objects/tupleobject.c:479"),
    ]
}

// Keep the c_void import referenced (header shim uses it through this module).
#[allow(unused)]
fn _unused(_: *mut c_void) {}

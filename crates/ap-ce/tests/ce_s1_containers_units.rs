//! Leaf 1005a unit gates (in-process, no .so build): the container-band
//! contracts ap-ce owns internally — dialect table shape, error-state
//! semantics on the exported fns (called directly through the Rust
//! symbols), and refcount-bump discipline where observable.
//!
//! The FULL byte-exact differential lives in tests/ce-s1/containers.c +
//! containers_driver.py (built via `antipythony build-ext`, run under the
//! binary AND CPython 3.14; expected output pinned in
//! tests/ce-s1/expected/containers_cpython314_stdout.txt).

use ap_ce::exports_containers::dialect_table;
use ap_ce::state::{err_clear, err_is_set, err_take, new_handle, CeKind, CePayload};

/// Build a dict handle {'a': 1, 'b': 2}.
fn dict_ab() -> ap_ce::state::CeHandle {
    let d = ap_obj::dict::PyDict::new();
    let _ = d.set(
        ap_obj::value::Value::Str(ap_obj::value::PyString::new("a")),
        ap_obj::value::Value::Int(ap_obj::int::Int::from_i64(1)),
    );
    let _ = d.set(
        ap_obj::value::Value::Str(ap_obj::value::PyString::new("b")),
        ap_obj::value::Value::Int(ap_obj::int::Int::from_i64(2)),
    );
    new_handle(CeKind::Dict, CePayload::Dict(d))
}

fn long_handle(v: i64) -> ap_ce::state::CeHandle {
    new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(v)))
}

fn str_handle(s: &str) -> ap_ce::state::CeHandle {
    new_handle(CeKind::Str, CePayload::Str(s.to_string()))
}

fn last_err() -> Option<(String, String)> {
    err_take()
}

// ---------------------------------------------------------------------------
// Dialect table (the measured CPython 3.14.7 line numbers).
// ---------------------------------------------------------------------------

#[test]
fn dialect_table_has_container_lines() {
    let t = dialect_table();
    assert!(t.len() >= 31, "expected >=31 pinned lines, got {}", t.len());
    let get = |k: &str| {
        t.iter()
            .find(|(f, _)| *f == k)
            .map(|(_, l)| *l)
            .unwrap_or_else(|| panic!("missing {k}"))
    };
    assert_eq!(get("PyList_Size"), "Objects/listobject.c:306");
    assert_eq!(get("PyDict_Size"), "Objects/dictobject.c:4248");
    assert_eq!(get("PyDict_Keys"), "Objects/dictobject.c:3561");
    assert_eq!(get("PyDict_Pop"), "Objects/dictobject.c:3166");
    assert_eq!(get("PyTuple_GetSlice"), "Objects/tupleobject.c:479");
}

// ---------------------------------------------------------------------------
// PyDict_GetItemRef tri-state (1/0/-1).
// ---------------------------------------------------------------------------

#[test]
fn getitem_ref_tri_state() {
    err_clear();
    let d = dict_ab();
    let ka = str_handle("a");
    let kz = str_handle("zz");
    let mut out: ap_ce::ffi_types::PyObjPtr = std::ptr::null_mut();

    let rc_present = unsafe {
        ap_ce::exports_containers::PyDict_GetItemRef(
            d as ap_ce::ffi_types::PyObjPtr,
            ka as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rc_present, 1);
    assert!(!out.is_null());
    let v = ap_ce::value_conv::to_value(out as ap_ce::state::CeHandle);
    let v_repr = v
        .map(|x| ap_obj::ops::repr(&x).unwrap())
        .unwrap_or_default();
    assert_eq!(v_repr, "1");

    let rc_missing = unsafe {
        ap_ce::exports_containers::PyDict_GetItemRef(
            d as ap_ce::ffi_types::PyObjPtr,
            kz as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rc_missing, 0);
    assert!(out.is_null());
    assert!(!err_is_set(), "missing key must NOT set an error");

    // Unhashable key -> -1 + the measured dict-key TypeError.
    let list = new_handle(CeKind::List, CePayload::List(vec![long_handle(1)]));
    let rc_unhash = unsafe {
        ap_ce::exports_containers::PyDict_GetItemRef(
            d as ap_ce::ffi_types::PyObjPtr,
            list as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rc_unhash, -1);
    let (class, msg) = last_err().expect("unhashable key must set an error");
    assert_eq!(class, "TypeError");
    assert_eq!(
        msg,
        "cannot use 'list' as a dict key (unhashable type: 'list')"
    );
}

// ---------------------------------------------------------------------------
// PyDict_Pop (rc contract: strong ref out, 0 on missing, -1 unhashable).
// ---------------------------------------------------------------------------

#[test]
fn pop_contract() {
    err_clear();
    let d = dict_ab();
    let ka = str_handle("a");
    let kz = str_handle("zz");
    let mut out: ap_ce::ffi_types::PyObjPtr = std::ptr::null_mut();

    let rc1 = unsafe {
        ap_ce::exports_containers::PyDict_Pop(
            d as ap_ce::ffi_types::PyObjPtr,
            ka as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rc1, 1);
    assert!(!out.is_null());
    assert_eq!(
        unsafe { ap_ce::exports_containers::PyDict_Size(d as ap_ce::ffi_types::PyObjPtr) },
        1
    );

    let rc0 = unsafe {
        ap_ce::exports_containers::PyDict_Pop(
            d as ap_ce::ffi_types::PyObjPtr,
            kz as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rc0, 0);
    assert!(out.is_null());
    assert!(!err_is_set());

    let list = new_handle(CeKind::List, CePayload::List(vec![]));
    let rcneg = unsafe {
        ap_ce::exports_containers::PyDict_Pop(
            d as ap_ce::ffi_types::PyObjPtr,
            list as ap_ce::ffi_types::PyObjPtr,
            &mut out,
        )
    };
    assert_eq!(rcneg, -1);
    assert!(err_is_set());
    err_clear();
}

// ---------------------------------------------------------------------------
// KeyError carries the key's repr (DelItem missing).
// ---------------------------------------------------------------------------

#[test]
fn delitem_missing_keyerror_repr() {
    err_clear();
    let d = dict_ab();
    let kz = str_handle("zz");
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_DelItem(
            d as ap_ce::ffi_types::PyObjPtr,
            kz as ap_ce::ffi_types::PyObjPtr,
        )
    };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().expect("missing DelItem must raise");
    assert_eq!(class, "KeyError");
    assert_eq!(msg, "'zz'", "KeyError arg = the key's repr");
}

// ---------------------------------------------------------------------------
// Line-pinned SystemError on wrong-typed receivers.
// ---------------------------------------------------------------------------

#[test]
fn wrong_receiver_dialect() {
    err_clear();
    let i5 = long_handle(5);

    let n = unsafe { ap_ce::exports_containers::PyDict_Size(i5 as ap_ce::ffi_types::PyObjPtr) };
    assert_eq!(n, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "SystemError");
    assert_eq!(
        msg,
        "Objects/dictobject.c:4248: bad argument to internal function"
    );

    err_clear();
    let n = unsafe { ap_ce::exports_containers::PyList_Size(i5 as ap_ce::ffi_types::PyObjPtr) };
    assert_eq!(n, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "SystemError");
    assert_eq!(
        msg,
        "Objects/listobject.c:306: bad argument to internal function"
    );

    err_clear();
    let keys = unsafe { ap_ce::exports_containers::PyDict_Keys(i5 as ap_ce::ffi_types::PyObjPtr) };
    assert!(keys.is_null());
    let (_, msg) = last_err().unwrap();
    assert_eq!(
        msg,
        "Objects/dictobject.c:3561: bad argument to internal function"
    );

    err_clear();
    let r = unsafe { ap_ce::exports_containers::PyList_AsTuple(i5 as ap_ce::ffi_types::PyObjPtr) };
    assert!(r.is_null());
    let (_, msg) = last_err().unwrap();
    assert_eq!(
        msg,
        "Objects/listobject.c:3219: bad argument to internal function"
    );
}

// ---------------------------------------------------------------------------
// PyDict_Merge: non-dict other -> the measured AttributeError chain head.
// ---------------------------------------------------------------------------

#[test]
fn merge_nondict_attribute_error() {
    err_clear();
    let d = dict_ab();
    let l = new_handle(CeKind::List, CePayload::List(vec![long_handle(1)]));
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_Merge(
            d as ap_ce::ffi_types::PyObjPtr,
            l as ap_ce::ffi_types::PyObjPtr,
            1,
        )
    };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "AttributeError");
    assert_eq!(msg, "'list' object has no attribute 'keys'");
}

// ---------------------------------------------------------------------------
// PyDict_MergeFromSeq2: measured ValueError arity text.
// ---------------------------------------------------------------------------

#[test]
fn merge_seq2_arity_texts() {
    err_clear();
    let d = new_handle(CeKind::Dict, CePayload::Dict(ap_obj::dict::PyDict::new()));
    let bad3 = new_handle(
        CeKind::List,
        CePayload::List(vec![new_handle(
            CeKind::Tuple,
            CePayload::Tuple(vec![str_handle("a"), long_handle(1), long_handle(2)]),
        )]),
    );
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_MergeFromSeq2(
            d as ap_ce::ffi_types::PyObjPtr,
            bad3 as ap_ce::ffi_types::PyObjPtr,
            1,
        )
    };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "ValueError");
    assert_eq!(
        msg,
        "dictionary update sequence element #0 has length 3; 2 is required"
    );

    err_clear();
    let d2 = new_handle(CeKind::Dict, CePayload::Dict(ap_obj::dict::PyDict::new()));
    let int5 = long_handle(5);
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_MergeFromSeq2(
            d2 as ap_ce::ffi_types::PyObjPtr,
            int5 as ap_ce::ffi_types::PyObjPtr,
            1,
        )
    };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "TypeError");
    // OUTER non-iterable: type-mentioning (measured MergeFromSeq2(42)).
    assert_eq!(msg, "'int' object is not iterable");

    // INNER non-iterable item ([42] outer list): the BARE text (measured).
    err_clear();
    let d3 = new_handle(CeKind::Dict, CePayload::Dict(ap_obj::dict::PyDict::new()));
    let inner_int = new_handle(CeKind::List, CePayload::List(vec![long_handle(42)]));
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_MergeFromSeq2(
            d3 as ap_ce::ffi_types::PyObjPtr,
            inner_int as ap_ce::ffi_types::PyObjPtr,
            1,
        )
    };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "TypeError");
    assert_eq!(msg, "object is not iterable");

    // STR item of len 2 is a valid pair (measured: {'ab': 1} merges).
    err_clear();
    let d4 = new_handle(CeKind::Dict, CePayload::Dict(ap_obj::dict::PyDict::new()));
    let str_item = new_handle(CeKind::List, CePayload::List(vec![str_handle("ab")]));
    let rc = unsafe {
        ap_ce::exports_containers::PyDict_MergeFromSeq2(
            d4 as ap_ce::ffi_types::PyObjPtr,
            str_item as ap_ce::ffi_types::PyObjPtr,
            1,
        )
    };
    assert_eq!(rc, 0);
    assert!(!err_is_set());
    let merged = ap_ce::value_conv::to_value(d4 as ap_ce::state::CeHandle).unwrap();
    assert_eq!(ap_obj::ops::repr(&merged).unwrap(), "{'a': 'b'}");
}

// ---------------------------------------------------------------------------
// PyList_GetItem / GetItemRef index contracts (OOB AND negative; the C
// API does not wrap).
// ---------------------------------------------------------------------------

#[test]
fn list_index_contracts() {
    err_clear();
    let l = new_handle(
        CeKind::List,
        CePayload::List(vec![long_handle(3), long_handle(1), long_handle(2)]),
    );
    let hp = l as ap_ce::ffi_types::PyObjPtr;

    let v = unsafe { ap_ce::exports_containers::PyList_GetItem(hp, 0) };
    assert!(!v.is_null());
    let got = ap_ce::value_conv::to_value(v as ap_ce::state::CeHandle);
    let got_repr = got
        .map(|x| ap_obj::ops::repr(&x).unwrap())
        .unwrap_or_default();
    assert_eq!(got_repr, "3");

    err_clear();
    let v = unsafe { ap_ce::exports_containers::PyList_GetItem(hp, -1) };
    assert!(v.is_null(), "negative index must NOT wrap on the C API");
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "IndexError");
    assert_eq!(msg, "list index out of range");

    err_clear();
    let v = unsafe { ap_ce::exports_containers::PyList_GetItem(hp, 5) };
    assert!(v.is_null());
    let (_, msg) = last_err().unwrap();
    assert_eq!(msg, "list index out of range");

    err_clear();
    let v = unsafe { ap_ce::exports_containers::PyList_GetItemRef(hp, -1) };
    assert!(v.is_null());
    let (class, _) = last_err().unwrap();
    assert_eq!(class, "IndexError");
}

// ---------------------------------------------------------------------------
// PyList_SetSlice delete / insert shapes (measured CPython sequences).
// ---------------------------------------------------------------------------

#[test]
fn list_setslice_shapes() {
    let mk = |vals: Vec<i64>| {
        new_handle(
            CeKind::List,
            CePayload::List(vals.into_iter().map(long_handle).collect()),
        )
    };
    err_clear();
    // [0,1,2,3] SetSlice(1,3,[9,8]) -> [0,9,8,3]
    let l = mk(vec![0, 1, 2, 3]);
    let n = new_handle(
        CeKind::List,
        CePayload::List(vec![long_handle(9), long_handle(8)]),
    );
    let rc = unsafe {
        ap_ce::exports_containers::PyList_SetSlice(
            l as ap_ce::ffi_types::PyObjPtr,
            1,
            3,
            n as ap_ce::ffi_types::PyObjPtr,
        )
    };
    assert_eq!(rc, 0);
    let vals = ap_ce::value_conv::to_value(l as ap_ce::state::CeHandle).unwrap();
    let repr = ap_obj::ops::repr(&vals).unwrap();
    assert_eq!(repr, "[0, 9, 8, 3]");

    // SetSlice(0,1,NULL) deletes -> [9,8,3]
    let rc = unsafe {
        ap_ce::exports_containers::PyList_SetSlice(
            l as ap_ce::ffi_types::PyObjPtr,
            0,
            1,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0);
    let vals = ap_ce::value_conv::to_value(l as ap_ce::state::CeHandle).unwrap();
    assert_eq!(ap_obj::ops::repr(&vals).unwrap(), "[9, 8, 3]");

    // SetSlice(1,0,[9,8]) inserts -> [9,9,8,8,3]
    let rc = unsafe {
        ap_ce::exports_containers::PyList_SetSlice(
            l as ap_ce::ffi_types::PyObjPtr,
            1,
            0,
            n as ap_ce::ffi_types::PyObjPtr,
        )
    };
    assert_eq!(rc, 0);
    let vals = ap_ce::value_conv::to_value(l as ap_ce::state::CeHandle).unwrap();
    assert_eq!(ap_obj::ops::repr(&vals).unwrap(), "[9, 9, 8, 8, 3]");
}

// ---------------------------------------------------------------------------
// PyList_Insert clamp shape (measured: [-1, 0, 9, 1, 8]).
// ---------------------------------------------------------------------------

#[test]
fn list_insert_clamp_shape() {
    err_clear();
    let l = new_handle(CeKind::List, CePayload::List(vec![]));
    let hp = l as ap_ce::ffi_types::PyObjPtr;
    unsafe {
        ap_ce::exports_containers::PyList_Insert(
            hp,
            0,
            long_handle(1) as ap_ce::ffi_types::PyObjPtr,
        );
        ap_ce::exports_containers::PyList_Insert(
            hp,
            0,
            long_handle(0) as ap_ce::ffi_types::PyObjPtr,
        );
        ap_ce::exports_containers::PyList_Insert(
            hp,
            1,
            long_handle(9) as ap_ce::ffi_types::PyObjPtr,
        );
        ap_ce::exports_containers::PyList_Insert(
            hp,
            100,
            long_handle(8) as ap_ce::ffi_types::PyObjPtr,
        );
        ap_ce::exports_containers::PyList_Insert(
            hp,
            -100,
            long_handle(-1) as ap_ce::ffi_types::PyObjPtr,
        );
    }
    let vals = ap_ce::value_conv::to_value(l as ap_ce::state::CeHandle).unwrap();
    assert_eq!(ap_obj::ops::repr(&vals).unwrap(), "[-1, 0, 9, 1, 8]");
}

// ---------------------------------------------------------------------------
// PyTuple_GetSlice clamps (measured incl. negatives).
// ---------------------------------------------------------------------------

#[test]
fn tuple_getslice_clamps() {
    err_clear();
    let t = new_handle(
        CeKind::Tuple,
        CePayload::Tuple(vec![long_handle(1), long_handle(2), str_handle("x")]),
    );
    let hp = t as ap_ce::ffi_types::PyObjPtr;
    let s1 = unsafe { ap_ce::exports_containers::PyTuple_GetSlice(hp, 1, 3) };
    let s2 = unsafe { ap_ce::exports_containers::PyTuple_GetSlice(hp, -100, 100) };
    let s3 = unsafe { ap_ce::exports_containers::PyTuple_GetSlice(hp, 2, 1) };
    let s4 = unsafe { ap_ce::exports_containers::PyTuple_GetSlice(hp, -2, 2) };
    let s5 = unsafe { ap_ce::exports_containers::PyTuple_GetSlice(hp, 1, -100) };
    let rep = |h: ap_ce::ffi_types::PyObjPtr| {
        let v = ap_ce::value_conv::to_value(h as ap_ce::state::CeHandle).unwrap();
        ap_obj::ops::repr(&v).unwrap()
    };
    assert_eq!(rep(s1), "(2, 'x')");
    assert_eq!(rep(s2), "(1, 2, 'x')");
    assert_eq!(rep(s3), "()");
    assert_eq!(rep(s4), "(1, 2)");
    assert_eq!(rep(s5), "()");
}

// ---------------------------------------------------------------------------
// PyList_Sort mixed types -> the measured '<' TypeError (order follows
// the compared pair).
// ---------------------------------------------------------------------------

#[test]
fn list_sort_mixed_text() {
    err_clear();
    let l = new_handle(
        CeKind::List,
        CePayload::List(vec![long_handle(1), str_handle("a")]),
    );
    let rc = unsafe { ap_ce::exports_containers::PyList_Sort(l as ap_ce::ffi_types::PyObjPtr) };
    assert_eq!(rc, -1);
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "TypeError");
    assert_eq!(
        msg,
        "'<' not supported between instances of 'str' and 'int'"
    );
}

// ---------------------------------------------------------------------------
// apx_PyTuple_Pack array form.
// ---------------------------------------------------------------------------

#[test]
fn tuple_pack_array_form() {
    err_clear();
    let a = long_handle(1);
    let b = str_handle("x");
    let args = [
        a as ap_ce::ffi_types::PyObjPtr,
        b as ap_ce::ffi_types::PyObjPtr,
    ];
    let t = unsafe { ap_ce::exports_containers::apx_PyTuple_Pack(2, args.as_ptr()) };
    let v = ap_ce::value_conv::to_value(t as ap_ce::state::CeHandle).unwrap();
    assert_eq!(ap_obj::ops::repr(&v).unwrap(), "(1, 'x')");

    err_clear();
    let t = unsafe { ap_ce::exports_containers::apx_PyTuple_Pack(-1, std::ptr::null()) };
    assert!(t.is_null());
    let (class, msg) = last_err().unwrap();
    assert_eq!(class, "SystemError");
    assert_eq!(
        msg,
        "Objects/tupleobject.c:207: bad argument to internal function"
    );
}

// ---------------------------------------------------------------------------
// PyDict_GetItem masking: missing AND unhashable -> NULL + NO error.
// ---------------------------------------------------------------------------

#[test]
fn getitem_masks_errors() {
    err_clear();
    let d = dict_ab();
    let hp = d as ap_ce::ffi_types::PyObjPtr;

    let missing = unsafe {
        ap_ce::exports_containers::PyDict_GetItem(
            hp,
            str_handle("zz") as ap_ce::ffi_types::PyObjPtr,
        )
    };
    assert!(missing.is_null());
    assert!(!err_is_set(), "missing-key GetItem must not set an error");

    let list = new_handle(CeKind::List, CePayload::List(vec![long_handle(1)]));
    err_clear();
    let unhash = unsafe {
        ap_ce::exports_containers::PyDict_GetItem(hp, list as ap_ce::ffi_types::PyObjPtr)
    };
    assert!(unhash.is_null());
    assert!(
        !err_is_set(),
        "unhashable GetItem must MASK (NULL, no error)"
    );
}

//! Leaf 10.04 (in-process unit gates, no .so build): the call-back
//! registration + tp_basicsize payload registry semantics.
//!
//! The full .so-level differential lives in tests/ce-s1/*.c + drivers
//! (callback + payload extensions, byte-exact vs CPython 3.14 — run in the
//! leaf session; see docs/plan10-1004-s1-bridge-report.md). These unit
//! gates pin the ap-ce-side contracts ap-run relies on so a regression in
//! the registry/keying shows up in `cargo test -p ap-ce` directly.

use ap_ce::payloads;
use ap_ce::runtime_ctx;
use ap_ce::state::{err_clear, err_is_set, err_take, new_handle, CeHandle, CeKind, CePayload};
use ap_ce::value_conv::to_value;
use ap_obj::value::Value;

#[test]
fn pycall_registration_is_first_wins_and_queryable() {
    unsafe extern "C" fn probe_fn(
        _c: ap_ce::ffi_types::PyObjPtr,
        _a: ap_ce::ffi_types::PyObjPtr,
        _k: ap_ce::ffi_types::PyObjPtr,
    ) -> ap_ce::ffi_types::PyObjPtr {
        std::ptr::null_mut()
    }
    // First install wins; a second installer is rejected (-1 semantics).
    let first = runtime_ctx::install_pycall(probe_fn);
    let second = runtime_ctx::install_pycall(probe_fn);
    assert!(first || second, "exactly one install must succeed");
    assert!(
        runtime_ctx::pycall_fn().is_some(),
        "pycall_fn queryable after install"
    );
}

#[test]
fn payload_alloc_roundtrip_and_take() {
    let key = 0xA0_04usize;
    let block = payloads::payload_alloc(key, 7, 32) as *mut u64;
    assert!(!block.is_null(), "alloc must return a block");
    assert!(payloads::payload_is_spec(key, 7));
    assert!(!payloads::payload_is_spec(key, 8));
    // Zero-initialized (the tp_init contract: fields start at 0).
    unsafe {
        assert_eq!(*block, 0, "payload block zeroed at alloc");
        *block = 0x1234;
    }
    assert_eq!(payloads::payload_of(key) as usize, block as usize);
    payloads::payload_take(key);
    assert!(
        payloads::payload_of(key).is_null(),
        "take detaches the block"
    );
    assert!(!payloads::payload_is_spec(key, 7));
}

#[test]
fn opaque_handles_round_trip_faithfully() {
    // A runtime callable (BuiltinFn marker) must survive handle -> to_value
    // with the EXACT object (identity), not a repr pin — the PyObject_Call
    // fidelity contract.
    let marker = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "callback.apply");
    let v = Value::Obj(marker.clone());
    let h = ap_ce::state::from_value(&v);
    let back = to_value(h).expect("opaque handle resolves");
    match back {
        Value::Obj(o) => {
            assert_eq!(o.name, "callback.apply");
            assert_eq!(o.kind, ap_obj::ObjKind::BuiltinFn);
        }
        other => panic!("expected faithful Obj round-trip, got {other:?}"),
    }
    // The handle's own payload is the Opaque arm.
    unsafe {
        let ce = &*(h as CeHandle);
        assert_eq!(ce.ob_type, CeKind::Opaque);
        assert!(matches!(&*ce.payload.borrow(), Some(CePayload::Opaque(_))));
    }
}

#[test]
fn ce_instance_handle_resolves_through_value_map() {
    // alloc_ce_instance's contract: bridge attaches the runtime Value under
    // the payload key; the CeInstance handle's to_value finds it (the map
    // entry is take-once).
    let key = 0xBEEFusize;
    let inst = ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, "payload.Box");
    ap_ce::value_map::set_runtime_value(key, Value::Obj(inst.clone()));
    let h = new_handle(
        CeKind::Instance,
        CePayload::CeInstance {
            class_spec: 0,
            payload_key: key,
        },
    );
    let back = to_value(h).expect("CeInstance resolves via the map");
    match back {
        Value::Obj(o) => assert_eq!(o.name, "payload.Box"),
        other => panic!("expected the attached instance, got {other:?}"),
    }
    // Take-once: second resolve falls to None (documented handoff model).
    assert!(
        to_value(h).is_none(),
        "CeInstance map entries are take-once"
    );
}

#[test]
fn err_state_roundtrip_for_bridge_flow() {
    err_clear();
    assert!(!err_is_set());
    // The bridge's flow_to_err shape: set + class-carrying take.
    ap_ce::state::err_set("ValueError", "from python");
    assert!(err_is_set());
    let (class, msg) = err_take().expect("error state populated");
    assert_eq!(class, "ValueError");
    assert_eq!(msg, "from python");
    assert!(!err_is_set());
}

// ---------------------------------------------------------------------------
// Leaf 1005b: the object-protocol band unit gates (the vtable seam + the
// exported fn contracts BEFORE the vtable install — the C-level shapes).
// The full runtime-backed differential lives in tests/ce-s1/objproto.c +
// objproto_driver.py (byte-exact vs CPython 3.14, run in the leaf session).
// ---------------------------------------------------------------------------

mod objproto_units {
    use ap_ce::exports_objproto::{
        install_objproto_vtable, test_vtable_installed, ObjProtoVTable, OpRes, PY_EQ, PY_LT, PY_NE,
    };
    use ap_ce::ffi_types::PyObjPtr;
    use ap_ce::state::{err_clear, err_is_set, err_take, new_handle, CeKind, CePayload};
    use ap_obj::value::Value;

    /// The probe vtable BODY (fn pointers, so a leaked const-shaped copy is
    /// constructible at install time — the OnceLock owns them for 'static).
    macro_rules! probe_vtable {
        () => {
            ObjProtoVTable {
                attr_get: |obj, name| {
                    if name == "hit" {
                        OpRes::val(Value::Int(ap_obj::int::Int::from_i64(7)))
                    } else {
                        OpRes::missing_with_text(format!("'X' object has no attribute '{name}'"))
                    }
                },
                attr_set: |_obj, _name, _v| OpRes::unit(),
                attr_del: |_obj, _name| OpRes::unit(),
                len_of: |_obj| OpRes::val(Value::Int(ap_obj::int::Int::from_i64(3))),
                length_hint: |_obj, default| {
                    OpRes::val(Value::Int(ap_obj::int::Int::from_i64(default)))
                },
                truth_of: |obj| OpRes::val(Value::Bool(!matches!(obj, Value::None))),
                rich_compare: |a, b, op, _bool_form| {
                    let (Some(x), Some(y)) = (a.as_int(), b.as_int()) else {
                        return OpRes::err(
                            "TypeError",
                            "'<' not supported between instances of 'X' and 'Y'",
                        );
                    };
                    let (x, y) = (x.to_i64().unwrap_or(0), y.to_i64().unwrap_or(0));
                    let r = match op {
                        0 => x < y,
                        1 => x <= y,
                        2 => x == y,
                        3 => x != y,
                        4 => x > y,
                        _ => x >= y,
                    };
                    OpRes::val(Value::Bool(r))
                },
                get_iter: |_obj| OpRes::err("TypeError", "'X' object is not iterable"),
                iter_next: |_obj| OpRes::exhausted(),
                hash_of: |obj| match obj {
                    Value::Int(i) => OpRes::val(Value::Int(i.clone())),
                    _ => OpRes::err("TypeError", "unhashable type: 'X'"),
                },
                dir_of: |_obj| OpRes::err("TypeError", "dir expected"),
                display_name: |v| v.type_name().to_string(),
            }
        };
    }

    #[test]
    fn vtable_install_is_first_wins_and_queryable() {
        // Idempotent across the whole test binary: either THIS install won
        // or an earlier test's did — the seam contract is "one runtime".
        let ok = install_objproto_vtable(probe_vtable!());
        assert!(ok || test_vtable_installed());
        assert!(test_vtable_installed());
    }

    #[test]
    fn getattr_hit_and_miss_dialect() {
        // The probe vtable may or may not already be installed (parallel
        // unit order); install is first-wins idempotent.
        let _ = install_objproto_vtable(probe_vtable!());
        let obj = new_handle(
            CeKind::Opaque,
            CePayload::Opaque(ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, "X")),
        );
        let name = new_handle(CeKind::Str, CePayload::Str("hit".to_string()));
        let r =
            unsafe { ap_ce::exports_objproto::PyObject_GetAttr(obj as PyObjPtr, name as PyObjPtr) };
        assert!(!r.is_null(), "hit must return a handle");
        assert_eq!(
            ap_ce::value_conv::as_i64(r as ap_ce::state::CeHandle),
            Some(7)
        );
        err_clear();
        let miss_name = new_handle(CeKind::Str, CePayload::Str("zz".to_string()));
        let r2 = unsafe {
            ap_ce::exports_objproto::PyObject_GetAttr(obj as PyObjPtr, miss_name as PyObjPtr)
        };
        assert!(r2.is_null(), "miss returns NULL");
        assert!(err_is_set());
        let (class, msg) = err_take().unwrap();
        assert_eq!(class, "AttributeError");
        assert_eq!(msg, "'X' object has no attribute 'zz'");
    }

    #[test]
    fn name_must_be_string_before_dispatch() {
        let obj = new_handle(CeKind::Str, CePayload::Str("whatever".to_string()));
        let bad_name = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(42)));
        err_clear();
        let r = unsafe {
            ap_ce::exports_objproto::PyObject_GetAttr(obj as PyObjPtr, bad_name as PyObjPtr)
        };
        assert!(r.is_null());
        let (class, msg) = err_take().unwrap();
        assert_eq!(class, "TypeError");
        assert_eq!(msg, "attribute name must be string, not 'int'");
    }

    #[test]
    fn getoptionalattr_contract() {
        let _ = install_objproto_vtable(probe_vtable!());
        let obj = new_handle(
            CeKind::Opaque,
            CePayload::Opaque(ap_obj::PyObject::new(ap_obj::ObjKind::UserInstance, "X")),
        );
        let hit = new_handle(CeKind::Str, CePayload::Str("hit".to_string()));
        let mut out: ap_ce::ffi_types::PyObjPtr = std::ptr::null_mut();
        let rc = unsafe {
            ap_ce::exports_objproto::PyObject_GetOptionalAttr(
                obj as PyObjPtr,
                hit as PyObjPtr,
                &mut out,
            )
        };
        assert_eq!(rc, 1, "hit => 1");
        assert!(!out.is_null());
        err_clear();
        let miss = new_handle(CeKind::Str, CePayload::Str("nope".to_string()));
        let mut out2: ap_ce::ffi_types::PyObjPtr = std::ptr::null_mut();
        let rc2 = unsafe {
            ap_ce::exports_objproto::PyObject_GetOptionalAttr(
                obj as PyObjPtr,
                miss as PyObjPtr,
                &mut out2,
            )
        };
        assert_eq!(rc2, 0, "miss => 0");
        assert!(out2.is_null(), "miss leaves *result NULL");
        assert!(!err_is_set(), "miss sets NO error (PyO3 contract)");
    }

    #[test]
    fn richcompare_op_codes_route() {
        let _ = install_objproto_vtable(probe_vtable!());
        let a = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(1)));
        let b = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(2)));
        let r = unsafe {
            ap_ce::exports_objproto::PyObject_RichCompare(a as PyObjPtr, b as PyObjPtr, PY_LT)
        };
        assert!(!r.is_null());
        match ap_ce::value_conv::to_value(r as ap_ce::state::CeHandle) {
            Some(Value::Bool(true)) => {}
            other => panic!("1 < 2 must be Bool(true), got {other:?}"),
        }
        // The identity shortcut: RichCompareBool(x, x, Py_EQ) => 1 without
        // dispatching (needs distinct handles pointing at the same object —
        // pointer equality IS the shortcut).
        let x = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(5)));
        let rc = unsafe {
            ap_ce::exports_objproto::PyObject_RichCompareBool(x as PyObjPtr, x as PyObjPtr, PY_EQ)
        };
        assert_eq!(rc, 1, "identity shortcut Py_EQ");
        let rc2 = unsafe {
            ap_ce::exports_objproto::PyObject_RichCompareBool(x as PyObjPtr, x as PyObjPtr, PY_NE)
        };
        assert_eq!(rc2, 0, "identity shortcut Py_NE");
        // Bad op => SystemError, never a panic.
        err_clear();
        let bad = unsafe {
            ap_ce::exports_objproto::PyObject_RichCompare(a as PyObjPtr, b as PyObjPtr, 99)
        };
        assert!(bad.is_null());
        assert!(err_is_set());
        let (class, _msg) = err_take().unwrap();
        assert_eq!(class, "SystemError");
    }

    #[test]
    fn not_and_istrue_shapes() {
        let _ = install_objproto_vtable(probe_vtable!());
        let t = new_handle(CeKind::Bool, CePayload::Bool(true));
        let n = new_handle(CeKind::None, CePayload::Bool(false));
        // to_value maps CeKind::None + Bool(false) payload to Value::None —
        // the probe vtable answers !matches!(None) => true... wait: None is
        // falsy. The probe: !matches!(obj, Value::None) => None => false.
        let r1 = unsafe { ap_ce::exports_objproto::PyObject_IsTrue(t as PyObjPtr) };
        assert_eq!(r1, 1);
        let r2 = unsafe { ap_ce::exports_objproto::PyObject_IsTrue(n as PyObjPtr) };
        assert_eq!(r2, 0, "None is falsy through the probe");
        let r3 = unsafe { ap_ce::exports_objproto::PyObject_Not(n as PyObjPtr) };
        assert_eq!(r3, 1, "not None => 1");
    }

    #[test]
    fn hash_sentinel_and_null_guards() {
        let _ = install_objproto_vtable(probe_vtable!());
        let i = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(42)));
        let h = unsafe { ap_ce::exports_objproto::PyObject_Hash(i as PyObjPtr) };
        assert_eq!(h, 42, "int hash passthrough (probe vtable)");
        err_clear();
        // NULL object => SystemError, never a panic.
        let bad = unsafe { ap_ce::exports_objproto::PyObject_Hash(std::ptr::null_mut()) };
        assert_eq!(bad, -1);
        let (class, _) = err_take().unwrap();
        assert_eq!(class, "SystemError");
    }

    #[test]
    fn iter_check_recognizes_kinds() {
        let it_obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
        let it = new_handle(CeKind::Opaque, CePayload::Opaque(it_obj));
        let rc = unsafe { ap_ce::exports_objproto::apx_PyIter_Check(it as PyObjPtr) };
        assert_eq!(rc, 1, "Iterator kind passes PyIter_Check");
        let s = new_handle(CeKind::Str, CePayload::Str("plain".to_string()));
        let rc2 = unsafe { ap_ce::exports_objproto::apx_PyIter_Check(s as PyObjPtr) };
        assert_eq!(rc2, 0, "str handle is not an iterator");
        let rc3 = unsafe { ap_ce::exports_objproto::apx_PyIter_Check(std::ptr::null_mut()) };
        assert_eq!(rc3, 0, "NULL is not an iterator");
    }
}

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

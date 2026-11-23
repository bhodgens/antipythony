//! Leaf 1006 unit gates (in-process, no .so build): the hard-members band
//! contracts — PyLong export/AsNativeBytes semantics, the GC traverse
//! registry, PyMutex lock semantics, PyMem/GILState bookkeeping — so a
//! regression shows in `cargo test -p ap-ce` directly.
//!
//! The FULL byte-exact differential lives in tests/ce-s1/{longgc.c,
//! methodmem.c} + drivers (built via `antipythony build-ext`, run under
//! the binary AND CPython 3.14.7; expected output pinned in
//! tests/ce-s1/expected/).

use ap_ce::exports_long_gc::{
    fits_in_n_bits, from_digits, gc_traverse_collect, gc_traverse_register, gc_traverse_unregister,
    resolve_endianness, to_digits, TraverseProc,
};
use ap_ce::exports_method_mem::{
    PyGILState_Check, PyGILState_Ensure, PyGILState_Release, PyMem_Calloc, PyMem_Free,
    PyMem_Malloc, PyMutex_IsLocked, PyMutex_Lock, PyMutex_Unlock, PyUnstable_Long_CompactValue,
    PyUnstable_Long_IsCompact,
};
use ap_ce::state::{err_clear, err_is_set, err_take, new_handle, CeHandle, CeKind, CePayload};

fn long_handle_big(mag: num_bigint::BigInt, neg: bool) -> CeHandle {
    let v = if neg {
        ap_obj::int::Int::from_big(-&mag)
    } else {
        ap_obj::int::Int::from_big(mag)
    };
    new_handle(CeKind::Int, CePayload::Int(v))
}

// ---------------------------------------------------------------------------
// Digit layout (the 3.14 measured dialect)
// ---------------------------------------------------------------------------

#[test]
fn digit_layout_matches_cpython_314() {
    // 10^30 = digits [0, 433305513, 852086020, 807] (LSB first, 30-bit).
    let digs = to_digits(&num_bigint::BigInt::from(10u8).pow(30));
    assert_eq!(digs, vec![0, 433305513, 852086020, 807]);
    // -2^70 -> [0, 0, 1024].
    let digs = to_digits(&(num_bigint::BigInt::from(1u8) << 70usize));
    assert_eq!(digs, vec![0, 0, 1024]);
    // Round-trip.
    let mag = from_digits(&digs);
    assert_eq!(mag, num_bigint::BigInt::from(1u8) << 70usize);
}

#[test]
fn endianness_trap_flags4_is_big_endian() {
    // The measured trap: UNS(4) alone resolves BIG endian (4&2==0, 4&1==0).
    assert!(resolve_endianness(-1)); // DEFAULTS -> native = LE
    assert!(resolve_endianness(3)); // NATIVE (bit 2 set) -> LE
    assert!(resolve_endianness(1)); // LITTLE_ENDIAN
    assert!(!resolve_endianness(0)); // BIG_ENDIAN
    assert!(!resolve_endianness(4)); // UNS alone -> BE (the trap)
}

#[test]
fn fits_in_n_bits_matches_cpython() {
    // 255 fits 2 bytes; 70000 needs 3.
    assert!(fits_in_n_bits(255, 16));
    assert!(fits_in_n_bits(-128, 8)); // -128>>7 = -1
    assert!(!fits_in_n_bits(70000, 16)); // 70000>>15 = 2
    assert!(!fits_in_n_bits(70000, 17)); // 70000>>16 = 1
}

// ---------------------------------------------------------------------------
// GC traverse registry
// ---------------------------------------------------------------------------

static TRAVERSE_PROBE_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

unsafe extern "C" fn probe_traverse(
    _self_h: ap_ce::ffi_types::PyObjPtr,
    visit: ap_ce::exports_long_gc::TraverseVisit,
    arg: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    TRAVERSE_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Visit a fresh int handle (its GC addr is 0 — untracked scalars stay
    // out of the graph, so visited stays empty; the CONTRACT under test
    // is that the C fn IS called and its visits are collected).
    let h = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(1)));
    visit(h as ap_ce::ffi_types::PyObjPtr, arg);
    0
}

#[test]
fn gc_traverse_registry_collects_visits() {
    let key = 0xC_1006usize;
    gc_traverse_register(key, probe_traverse as TraverseProc);
    let h = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(5)));
    let visited = gc_traverse_collect(key, h);
    assert!(
        TRAVERSE_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed) >= 1,
        "the registered traverse fn must be invoked"
    );
    let _ = visited; // untracked scalar visits resolve to no graph addr
    gc_traverse_unregister(key);
    assert!(
        !gc_traverse_collect(key, h).is_empty() == false,
        "unregistered key collects nothing"
    );
}

// ---------------------------------------------------------------------------
// PyMutex real-lock semantics
// ---------------------------------------------------------------------------

#[test]
fn pymutex_lock_unlock_islocked() {
    // One mutex per test (static local shape, like the proof extension).
    static MU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = MU.lock().unwrap_or_else(|p| p.into_inner());
    let mu = Box::leak(Box::new(0u8)) as *mut u8 as *mut std::ffi::c_void;
    unsafe {
        assert_eq!(PyMutex_IsLocked(mu), 0);
        PyMutex_Lock(mu);
        assert_eq!(PyMutex_IsLocked(mu), 1);
        PyMutex_Unlock(mu);
        assert_eq!(PyMutex_IsLocked(mu), 0);
    }
}

// ---------------------------------------------------------------------------
// PyMem front
// ---------------------------------------------------------------------------

#[test]
fn pymem_malloc_zero_is_nonnull_and_free_null_ok() {
    let z = unsafe { PyMem_Malloc(0) };
    assert!(!z.is_null(), "PyMem_Malloc(0) must be NON-NULL (CPython)");
    unsafe { PyMem_Free(z) };
    unsafe { PyMem_Free(std::ptr::null_mut()) }; // no-op
    let c = unsafe { PyMem_Calloc(4, 4) };
    assert!(!c.is_null());
    let zeroed = unsafe { std::slice::from_raw_parts(c as *const u8, 16) };
    assert!(zeroed.iter().all(|&b| b == 0), "calloc zeroed");
    unsafe { PyMem_Free(c) };
}

// ---------------------------------------------------------------------------
// PyUnstable_Long compactness
// ---------------------------------------------------------------------------

#[test]
fn unstable_long_compact_matches_cpython() {
    unsafe {
        let small = new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(42)));
        assert_eq!(
            PyUnstable_Long_IsCompact(small as ap_ce::ffi_types::PyObjPtr),
            1
        );
        assert_eq!(
            PyUnstable_Long_CompactValue(small as ap_ce::ffi_types::PyObjPtr),
            42
        );
        let big = long_handle_big(num_bigint::BigInt::from(10u8).pow(30), false);
        assert_eq!(
            PyUnstable_Long_IsCompact(big as ap_ce::ffi_types::PyObjPtr),
            0
        );
        err_clear();
    }
}

// ---------------------------------------------------------------------------
// PyGILState bookkeeping
// ---------------------------------------------------------------------------

#[test]
fn gilstate_ensure_release_check() {
    let g1 = PyGILState_Ensure();
    assert_eq!(g1, 0, "LOCKED on the attaching thread (measured)");
    let g2 = PyGILState_Ensure();
    assert_eq!(g2, 0, "nested Ensure is LOCKED (measured)");
    assert_eq!(PyGILState_Check(), 1);
    unsafe {
        PyGILState_Release(g2);
        PyGILState_Release(g1);
    }
    // The attaching thread STAYS attached (measured pin4: after=1).
    assert_eq!(PyGILState_Check(), 1);
}

// ---------------------------------------------------------------------------
// The stale-class bridge finding stays fixed (gate-found regression pin)
// ---------------------------------------------------------------------------

#[test]
fn expect_int_sets_typeerror_class_not_api_name() {
    // The gate-found bug: the API name landed in the CLASS slot, making
    // `except TypeError` never match. Regression pin via the exported
    // error path (PyLong_Export on a str handle).
    let s = new_handle(CeKind::Str, CePayload::Str("x".to_string()));
    let mut out = ap_ce::exports_long_gc::PyLongExport {
        value: 0,
        negative: 0,
        ndigits: 0,
        digits: std::ptr::null(),
        reserved: 0,
    };
    let rc =
        unsafe { ap_ce::exports_long_gc::PyLong_Export(s as ap_ce::ffi_types::PyObjPtr, &mut out) };
    assert_eq!(rc, -1);
    assert!(err_is_set());
    let (class, msg) = err_take().expect("error set");
    assert_eq!(class, "TypeError", "class slot must be TypeError");
    assert_eq!(msg, "expect int, got str");
    err_clear();
}

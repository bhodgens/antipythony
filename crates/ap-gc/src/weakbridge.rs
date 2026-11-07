//! weakbridge — the inverted-dependency seam between ap-gc (no ap-run dep)
//! and ap-run's interpreter state.
//!
//! ap-run registers a bridge at entry (`set_bridge`); the collector calls
//! through the function pointers for everything that needs interpreter
//! access: sys.modules enumeration, live-frame locals, `__del__` dispatch,
//! weakref-callback firing, gc-callback invocation.
//!
//! All calls are collector-context only: the bridge implementations run on
//! the interpreter thread (single-threaded v0) and never re-enter collect().

use ap_obj::value::Value;
use std::cell::RefCell;

/// Guard against re-entrant collection (a `__del__` that allocates).
///
/// C10 (memperf audit 2026-11-06): DEPTH counter, not a bool. The old flag
/// cleared unconditionally on Drop, so an inner guard exiting inside an
/// outer collection window re-enabled registration for the REST of the
/// outer pass — a callback's nested gc.collect() dropping its guard in
/// step 4 left steps 5/6 running with registration live: an object born
/// there inserted into REGISTRY after reachable was computed, and the
/// step-6 retain then dropped it (a latent false-collection path, not
/// reachable in the corpus only because no v1 case nests guards through a
/// callback that allocates). With depth, the flag clears only when the
/// OUTER guard exits.
pub(crate) struct CollectGuard;

thread_local! {
    static IN_COLLECT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl CollectGuard {
    pub(crate) fn new() -> CollectGuard {
        IN_COLLECT.with(|c| c.set(c.get() + 1));
        CollectGuard
    }
}

impl Drop for CollectGuard {
    fn drop(&mut self) {
        IN_COLLECT.with(|c| c.set(c.get().saturating_sub(1)));
    }
}

pub(crate) fn in_collect() -> bool {
    IN_COLLECT.with(|c| c.get() > 0)
}

/// True while the collector is firing finalizers/callbacks: ap-obj's
/// allocation gate skips REGISTRATION for objects created inside a
/// finalizer (they are born into the running mutator and will be seen by
/// the NEXT collect; registering them mid-sweep would corrupt the pass).
pub fn suppress_registration() -> bool {
    in_collect()
}

type SysModulesFn = fn() -> Vec<Value>;
type FrameValsFn = fn() -> Vec<Value>;
type DelFn = fn(&Value);
type CallbackFn = fn(&Value, &[Value]) -> Result<Value, String>;
type WeakFiredFn = fn();

#[derive(Default)]
struct Bridge {
    sys_modules: Option<SysModulesFn>,
    frame_values: Option<FrameValsFn>,
    call_del: Option<DelFn>,
    call_callback: Option<CallbackFn>,
    fire_weakrefs: Option<WeakFiredFn>,
}

thread_local! {
    static BRIDGE: RefCell<Bridge> = RefCell::new(Bridge::default());
}

/// Install the bridge (ap-run::run_module, once per run).
pub fn set_bridge(
    sys_modules: SysModulesFn,
    frame_values: FrameValsFn,
    call_del: DelFn,
    call_callback: CallbackFn,
    fire_weakrefs: WeakFiredFn,
) {
    BRIDGE.with(|b| {
        let mut b = b.borrow_mut();
        b.sys_modules = Some(sys_modules);
        b.frame_values = Some(frame_values);
        b.call_del = Some(call_del);
        b.call_callback = Some(call_callback);
        b.fire_weakrefs = Some(fire_weakrefs);
    });
}

pub(crate) fn sys_modules_values() -> Option<Vec<Value>> {
    BRIDGE.with(|b| b.borrow().sys_modules).map(|f| f())
}

pub(crate) fn frame_values_roots() -> Option<Vec<Value>> {
    BRIDGE.with(|b| b.borrow().frame_values).map(|f| f())
}

pub(crate) fn call_del_finalizer(v: &Value) {
    if let Some(f) = BRIDGE.with(|b| b.borrow().call_del) {
        f(v);
    }
}

pub(crate) fn call_callback(cb: &Value, args: &[Value]) -> Result<Value, String> {
    match BRIDGE.with(|b| b.borrow().call_callback) {
        Some(f) => f(cb, args),
        None => Ok(Value::None),
    }
}

pub(crate) fn fire_dead_weakrefs() {
    if let Some(f) = BRIDGE.with(|b| b.borrow().fire_weakrefs) {
        f();
    }
}

#[cfg(test)]
mod unit_tests {
    //! Leaf 01.03 (robustness-ci): pins the weakbridge global-state seam —
    //! the stated risk of this module is the thread_local BRIDGE + the
    //! re-entrancy guard. ap-gc has no ap-run dependency, so these are the
    //! only tests that can observe bridge call ORDERING without the
    //! interpreter.

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Referenced directly (not captured) inside the bridge fn pointers.
    static LOG_C: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn collect_guard_is_thread_local_and_restores_on_drop() {
        // INVARIANT: the re-entrancy guard flips on construction and clears
        // on drop, per-thread (a `__del__` that allocates must not re-enter
        // collect()). suppress_registration() mirrors it — ap-obj's gate
        // skips mid-sweep registration (weakbridge doc header).
        assert!(!in_collect());
        assert!(!suppress_registration());
        {
            let _g = CollectGuard::new();
            assert!(in_collect());
            assert!(suppress_registration());
        }
        assert!(!in_collect());
        let g = CollectGuard::new();
        assert!(in_collect());
        drop(g);
        assert!(!in_collect());
    }

    #[test]
    fn nested_guards_keep_the_flag_until_the_outer_drop() {
        // C10 (memperf audit 2026-11-06): the guard is a DEPTH counter now.
        // The old bool flag cleared on the INNER drop, re-enabling
        // registration inside the outer collection window (documented
        // footgun this replaced). Correct behavior: the flag stays set
        // until the LAST guard exits.
        let outer = CollectGuard::new();
        {
            let inner = CollectGuard::new();
            assert!(in_collect());
            drop(inner);
            assert!(
                in_collect(),
                "inner guard drop must NOT clear the OUTER window (depth counter)"
            );
        }
        assert!(in_collect());
        drop(outer);
        assert!(!in_collect());
    }

    #[test]
    fn bridge_install_and_dispatch_order() {
        // INVARIANT: set_bridge installs all five seams at once; every
        // accessor dispatches exactly the installed fn. The fire_dead_weakrefs
        // ordering (collector step 4, AFTER finalizers, BEFORE the registry
        // retain) is what makes weakref callbacks land in the SAME collect
        // that killed the target (gc004/gc005/gc008/gc015 red before the
        // GARBAGE_NOW fix) — asserted end-to-end in tests/gc_invariants.rs.
        let c = &LOG_C;
        c.store(0, Ordering::SeqCst);
        // Non-capturing closures coerce to the fn-pointer seam types:
        let sys: SysModulesFn = || {
            LOG_C.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        };
        let frames: FrameValsFn = || {
            LOG_C.fetch_add(10, Ordering::SeqCst);
            Vec::new()
        };
        let del: DelFn = |_v| {
            LOG_C.fetch_add(100, Ordering::SeqCst);
        };
        let cb: CallbackFn = |_c, _a| {
            LOG_C.fetch_add(1000, Ordering::SeqCst);
            Ok(Value::None)
        };
        let fired: WeakFiredFn = || {
            LOG_C.fetch_add(10000, Ordering::SeqCst);
        };
        set_bridge(sys, frames, del, cb, fired);
        assert_eq!(c.load(Ordering::SeqCst), 0);
        assert!(sys_modules_values().map(|v| v.is_empty()) == Some(true));
        assert_eq!(c.load(Ordering::SeqCst), 1);
        assert!(frame_values_roots().map(|v| v.is_empty()) == Some(true));
        assert_eq!(c.load(Ordering::SeqCst), 11);
        call_del_finalizer(&Value::None);
        assert_eq!(c.load(Ordering::SeqCst), 111);
        assert!(call_callback(&Value::None, &[]).is_ok());
        assert_eq!(c.load(Ordering::SeqCst), 1111);
        fire_dead_weakrefs();
        assert_eq!(c.load(Ordering::SeqCst), 11111);
    }

    #[test]
    fn bridge_accessors_are_reentrant_safe_and_return_owned() {
        // INVARIANT: the RefCell borrow of BRIDGE ends BEFORE the installed
        // fn runs (accessors map over the copied fn pointer), so a finalizer
        // that reads another bridge seam does not panic on a double borrow.
        // This is the one global-state deadlock the seam could hide.
        set_bridge(
            || Vec::new(),
            || Vec::new(),
            |_v| {},
            |_cb, _args| Ok(Value::None),
            || {
                // Re-enter OTHER accessors while running as a bridge callee:
                assert!(sys_modules_values().is_some());
                assert!(frame_values_roots().is_some());
            },
        );
        fire_dead_weakrefs(); // must not panic
    }
}

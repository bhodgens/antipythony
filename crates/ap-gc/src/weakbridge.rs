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
pub(crate) struct CollectGuard;

thread_local! {
    static IN_COLLECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl CollectGuard {
    pub(crate) fn new() -> CollectGuard {
        IN_COLLECT.with(|c| c.set(true));
        CollectGuard
    }
}

impl Drop for CollectGuard {
    fn drop(&mut self) {
        IN_COLLECT.with(|c| c.set(false));
    }
}

pub(crate) fn in_collect() -> bool {
    IN_COLLECT.with(|c| c.get())
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

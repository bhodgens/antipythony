//! gc_gate — allocation-site hook into the ap-gc registry (leaf 02.04).
//!
//! ap-obj has NO dependency on ap-gc (ap-gc depends on ap-obj), so the
//! collector's registration hook is injected here as a plain function
//! pointer (`set_hook`), installed by ap-run at entry. Container/object
//! constructors call [`gc_register_pub`] after building each new heap
//! value; when no hook is installed (pure value-layer tests, the
//! differential bins) registration is a no-op and behavior is unchanged.
//!
//! Which allocations track (the garbage the collector can actually miss):
//! containers that can participate in cycles or hold cycle members —
//! list/dict/set/frozenset/PyObject. NOT tracked: str/bytes/tuple
//! (immutable, cannot reference anything new after construction... tuple
//! MEMBERS can be cycle members, but a tuple only dies with its members
//! and never keeps a cycle alive by itself being alive: CPython tracks
//! tuples conservatively; the corpus pins post-collect `is_tracked` of
//! tuples as False, which the no-tracking answer gives directly).

use crate::value::Value;
use std::sync::atomic::{AtomicUsize, Ordering};

static HOOK: AtomicUsize = AtomicUsize::new(0);

/// Install the registration hook (ap-gc::gc_register, once at run entry).
pub fn set_hook(f: fn(&Value)) {
    let ptr = f as usize;
    HOOK.store(ptr, Ordering::Release);
}

/// True when a collector is installed (tests can assert inertness).
pub fn hook_installed() -> bool {
    HOOK.load(Ordering::Acquire) != 0
}

/// Register a fresh allocation with the collector (no-op without a hook).
pub fn gc_register_pub(v: &Value) {
    let f = HOOK.load(Ordering::Acquire);
    if f != 0 {
        let f: fn(&Value) = unsafe { std::mem::transmute(f) };
        f(v);
    }
}

/// Convenience: register then return the same value (builder chains).
pub fn tracked<T>(v: Value, _payload: &T) -> Value {
    gc_register_pub(&v);
    v
}

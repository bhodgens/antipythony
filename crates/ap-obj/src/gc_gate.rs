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

// Issue #23 real fix (inventory category c): was AtomicUsize + usize->fn
// reinterpretation; OnceLock stores the typed fn directly, so the call
// path is fully safe. Publication semantics are unchanged (OnceLock is
// atomic internally) and the hot-path load cost is the same acquire.
// First install wins; every in-repo caller installs the same hook for a
// process lifetime (gcsupport::install / install_hook_public both pass
// gc_register_routed; test/bench bins install once per mode before any
// allocation), so no overwriting behavior is lost.
static HOOK: std::sync::OnceLock<fn(&Value)> = std::sync::OnceLock::new();

/// Install the registration hook (ap-gc::gc_register, once at run entry).
pub fn set_hook(f: fn(&Value)) {
    let _ = HOOK.set(f);
}

/// True when a collector is installed (tests can assert inertness).
pub fn hook_installed() -> bool {
    HOOK.get().is_some()
}

/// Register a fresh allocation with the collector (no-op without a hook).
pub fn gc_register_pub(v: &Value) {
    if let Some(f) = HOOK.get() {
        f(v);
    }
}

/// Convenience: register then return the same value (builder chains).
pub fn tracked<T>(v: Value, _payload: &T) -> Value {
    gc_register_pub(&v);
    v
}

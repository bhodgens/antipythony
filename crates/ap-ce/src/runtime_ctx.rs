//! Leaf 10.04 (S1 item 1): the runtime-context registry — how a C extension
//! calls BACK into the running interpreter (PyObject_Call family).
//!
//! Model decision (recorded in docs/plan10-1004-s1-bridge-report.md):
//! ap-ce cannot depend on ap-run (workspace layering: ap-run depends on
//! ap-ce), so the call-back is a REGISTERED FUNCTION POINTER: ap-run's
//! ce_bridge installs one `PycallFn` at import time (`install_pycall`);
//! the exported PyObject_Call* symbols dispatch through it. The
//! interpreter handle itself rides the runtime's existing thread-local
//! (`interp::current_interp_handle`, the 02.09 per-worker model that
//! classes.rs metaclass hooks already use) — the explicit
//! runtime-context-parameter alternative was rejected because C slot fns
//! must keep CPython's exact ABI and the thread-local is already the
//! runtime's own convention.
//!
//! Re-entrancy is safe by construction: C fn -> registered pycall ->
//! call_value -> Python code -> C method -> registered pycall -> ... —
//! each hop is an ordinary Rust stack frame (no GIL to drop, no
//! thread-state swap; PEP 703 posture).

use std::sync::OnceLock;

use crate::ffi_types::PyObjPtr;

/// The call-back ABI: (callable, args tuple, kwargs dict-or-NULL) -> result
/// handle. NULL return == error state set (CPython convention).
pub type PycallFn = unsafe extern "C" fn(PyObjPtr, PyObjPtr, PyObjPtr) -> PyObjPtr;

static PYCALL: OnceLock<PycallFn> = OnceLock::new();

/// Install the runtime's call-back (idempotent; first installer wins —
/// one runtime per process).
pub fn install_pycall(f: PycallFn) -> bool {
    PYCALL.set(f).is_ok()
}

/// The registered call-back, if the runtime is attached.
pub fn pycall_fn() -> Option<PycallFn> {
    PYCALL.get().copied()
}

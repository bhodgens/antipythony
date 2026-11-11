//! 02.09a P4/P5: staged Send/Sync markers for the ap-obj object graph.
//!
//! Land ONLY after Decision 1 (`Gc<T> = Arc<T>`, int.rs) — the atomic
//! refcounts are what makes the marker-graph claims sound. See the contract
//! block in int.rs next to the alias: every cross-thread touch of these
//! types' RefCell/Cell interiors stays GIL/mutex-serialized until 02.09-b
// lands the per-object lock story (census punch-list P4). The markers cover
//! every payload type `Value` can hold, which unblocks the Send bounds of
//! the ap-run registry tables (LOCALS/EXCEPTHOOK/OBJ_MIRROR/WORKER_INTERP)
//! without any blanket wrapper.

use crate::classes::TypeObject;
use crate::dict::{PyDict, PySet};
use crate::object::PyObject;
use crate::value::{PyList, PyString, Tuple};

// PyString: interior state is two atomic idempotent caches (entry 27
// CONVERTED; entry 28 converted in this leaf after the hash-domain audit —
// both same-value races, benign by construction). Sync is actually PROVEN
// for this type, not just staged.
unsafe impl Send for PyString {}
unsafe impl Sync for PyString {}

// Tuple: immutable payload (Vec<Value> fixed at construction; audit entry
// 30 — "first candidate for unsafe impl Sync once the refcount story
// lands"). The Vec is never mutated; only the Arc counts move.
unsafe impl Send for Tuple {}
unsafe impl Sync for Tuple {}

// Staged (RefCell interiors — sound only under the GIL/mutex protocol):
// PyObject (18 RefCell fields), PyDict (state + epoch), PySet (state),
// PyList (items + allocated), TypeObject (mro/dict/metaclass/class_value).
unsafe impl Send for PyObject {}
unsafe impl Sync for PyObject {}
unsafe impl Send for PyDict {}
unsafe impl Sync for PyDict {}
unsafe impl Send for PySet {}
unsafe impl Sync for PySet {}
unsafe impl Send for PyList {}
unsafe impl Sync for PyList {}
unsafe impl Send for TypeObject {}
unsafe impl Sync for TypeObject {}

// The non-Gc Value payloads are plain data (Send+Sync by construction).
// `Value` itself derives them once every Gc payload is covered — the
// enum-level marker the registry tables' Send bounds resolve through.

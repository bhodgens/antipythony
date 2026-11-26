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
// CONVERTED; entry 28 converted in 02.09a P4 after the hash-domain audit —
// the -1 => -2 CPython-parity normalization keeps the 0-sentinel domain
// clean; both same-value races, benign by construction). Sync is actually
// PROVEN for this type, not just staged.
// SAFETY: interior state is two atomic idempotent caches (hash normalized -1=>-2 and the converted-entry cache) whose same-value races are benign by construction — Sync is proven for this type, not just staged (module docs / 02.09a P4 audit).
unsafe impl Send for PyString {}
// SAFETY: interior state is two atomic idempotent caches (hash normalized -1=>-2 and the converted-entry cache) whose same-value races are benign by construction — Sync is proven for this type, not just staged (module docs / 02.09a P4 audit).
unsafe impl Sync for PyString {}

// Tuple: immutable payload (Vec<Value> fixed at construction; audit entry
// 30 — "first candidate for unsafe impl Sync once the refcount story
// lands"). The Vec is never mutated; only the Arc counts move.
// SAFETY: the payload Vec<Value> is fixed at construction and never mutated afterward, so cross-thread shared access is read-only over immutable data (audit entry 30).
unsafe impl Send for Tuple {}
// SAFETY: the payload Vec<Value> is fixed at construction and never mutated afterward, so cross-thread shared access is read-only over immutable data (audit entry 30).
unsafe impl Sync for Tuple {}

// Staged (RefCell/Mutex interiors — sound under the GIL/mutex protocol):
// PyObject (18 RefCell fields), PyDict (state + epoch), PySet (state),
// PyList (items + allocated), TypeObject (mro[Mutex, write-once frozen in
// 02.09a P4]/dict/metaclass/class_value).
// SAFETY: all interior mutability lives in RefCell/Mutex fields that the no-GIL worker-serialization protocol keeps from concurrent borrows (02.09a P4/P5 staged markers).
unsafe impl Send for PyObject {}
// SAFETY: all interior mutability lives in RefCell/Mutex fields that the no-GIL worker-serialization protocol keeps from concurrent borrows (02.09a P4/P5 staged markers).
unsafe impl Sync for PyObject {}
// SAFETY: interior state (state + epoch cells) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Send for PyDict {}
// SAFETY: interior state (state + epoch cells) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Sync for PyDict {}
// SAFETY: interior state (state cell) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Send for PySet {}
// SAFETY: interior state (state cell) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Sync for PySet {}
// SAFETY: interior state (items + allocated flag) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Send for PyList {}
// SAFETY: interior state (items + allocated flag) is only mutated under the no-GIL worker-serialization protocol (02.09a P4/P5 staged markers).
unsafe impl Sync for PyList {}
// SAFETY: interior state (mro Mutex write-once frozen per 02.09a P4, plus dict/metaclass/class_value) is only mutated under the no-GIL worker-serialization protocol.
unsafe impl Send for TypeObject {}
// SAFETY: interior state (mro Mutex write-once frozen per 02.09a P4, plus dict/metaclass/class_value) is only mutated under the no-GIL worker-serialization protocol.
unsafe impl Sync for TypeObject {}

// The non-Gc Value payloads are plain data (Send+Sync by construction).
// `Value` itself derives them once every Gc payload is covered — the
// enum-level marker the registry tables' Send bounds resolve through.

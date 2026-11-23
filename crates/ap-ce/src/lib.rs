//! Plan 10 leaf 10.02 — the antipythony C-API bridge.
//!
//! The Rust dispatcher mapping the S0 C-API surface onto ap-obj values
//! (docs/ce-compat-survey.md §2 S0 ~120-fn core subset; this spike lands
//! the density-measuring subset, not all of S0).
//!
//! Design (locked by the survey):
//! - Refcounts are EMULATION: `CeObject` carries `ob_refcnt: isize` as its
//!   first member (our layout, not CPython's); INCREF/DECREF bump it, real
//!   memory ownership stays with the tracing GC (`Gc<T>` = Arc). PEP 703's
//!   "do not rely on the returned value" legitimizes this.
//! - Handles are opaque pointers to boxed `CeObject` structs; C never sees
//!   a `Value` enum. Every C-visible PyObject* dereferences to `CeObject`,
//!   whose `kind` decides which ap-obj payload it fronts.
//! - Types are heap types ONLY (PyType_FromSpec — the measured PyO3 path).
//!   Slots become `__ap_ce__.<slot>` marker callables registered in a
//!   process-global SLOT table; ap-run's ce_bridge routes dispatch here.
//! - Error state is thread-local (THE thread that runs the module), exact
//!   CPython semantics for the spike surface; ce_bridge converts to
//!   `Flow::Raise`.

pub mod api;
pub mod exports;
pub mod exports_containers;
pub mod exports_objproto;
pub mod ffi_types;
pub mod payloads;
pub mod runtime_ctx;
pub mod slots;
pub mod state;
pub mod value_conv;
pub mod value_map;

pub use api::{method_registry, module_registry};
pub use state::slot_registry;
pub use state::{
    ce_newref, ce_xdecref, err_clear, err_is_set, err_set, err_take, from_value, py_false, py_none,
    py_true, CeHandle, CeKind, CeObject, CePayload,
};
pub use value_conv::to_value;

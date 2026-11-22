//! Handle layer: the C-visible object. `ob_refcnt` first member — refcount
//! emulation over the tracing GC (survey commitment 1).

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use ap_obj::dict::PyDict;
use ap_obj::int::{Gc, Int};
use ap_obj::object::PyObject;
use ap_obj::value::{PyString, Tuple, Value};

/// What the handle fronts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeKind {
    None,
    Bool,
    Int,
    Float,
    Str,
    Tuple,
    List,
    Dict,
    /// A module handle minted by PyModule_Create.
    Module,
    /// A method-table function (PyCFunction): index into the method
    /// registry, name owned by the table.
    CFunction,
    /// A heap type built by PyType_FromSpec: index into the SPEC table.
    Type,
    /// An instance of a CeKind::Type (tp_new/tp_init dispatched by ap-run's
    /// ce_bridge; state lives in the instance dict).
    Instance,
}

/// The heap payload for reference-valued kinds.
pub enum CePayload {
    Int(Int),
    Float(f64),
    Bool(bool),
    Str(String),
    Tuple(Vec<CeHandle>),
    List(Vec<CeHandle>),
    Dict(Gc<PyDict>),
    Module {
        name: String,
        ns: Gc<PyDict>,
    },
    CFunction {
        module: String,
        name: String,
        meth_index: usize,
    },
    Type {
        spec_index: usize,
    },
    Instance {
        class_spec: usize,
        state: Gc<PyDict>,
    },
}

impl std::fmt::Debug for CePayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CePayload::Int(i) => write!(f, "Int({i:?})"),
            CePayload::Float(x) => write!(f, "Float({x})"),
            CePayload::Bool(b) => write!(f, "Bool({b})"),
            CePayload::Str(s) => write!(f, "Str({s:?})"),
            CePayload::Tuple(v) => write!(f, "Tuple(len={})", v.len()),
            CePayload::List(v) => write!(f, "List(len={})", v.len()),
            CePayload::Dict(_) => write!(f, "Dict"),
            CePayload::Module { name, .. } => write!(f, "Module({name})"),
            CePayload::CFunction { name, .. } => write!(f, "CFunction({name})"),
            CePayload::Type { spec_index } => write!(f, "Type(#{spec_index})"),
            CePayload::Instance { class_spec, .. } => write!(f, "Instance(#{class_spec})"),
        }
    }
}

/// The opaque struct C pointers point at. Layout is OURS: ob_refcnt first
/// (so header macros resolve against it), payload in a RefCell.
#[repr(C)]
pub struct CeObject {
    /// Emulated refcount (survey commitment 1). Never drives reclamation.
    pub ob_refcnt: isize,
    pub ob_type: CeKind,
    pub payload: RefCell<Option<CePayload>>,
}

pub type CeHandle = *mut CeObject;

// ---------------------------------------------------------------------------
// The immortal singletons (Py_None / Py_True / Py_False data symbols).
// ---------------------------------------------------------------------------

static NONE_CELL: OnceLock<usize> = OnceLock::new();
static TRUE_CELL: OnceLock<usize> = OnceLock::new();
static FALSE_CELL: OnceLock<usize> = OnceLock::new();

pub fn py_none() -> CeHandle {
    *NONE_CELL.get_or_init(|| immortal(CeKind::None, CePayload::Bool(false))) as CeHandle
}

pub fn py_true() -> CeHandle {
    *TRUE_CELL.get_or_init(|| immortal(CeKind::Bool, CePayload::Bool(true))) as CeHandle
}

pub fn py_false() -> CeHandle {
    *FALSE_CELL.get_or_init(|| immortal(CeKind::Bool, CePayload::Bool(false))) as CeHandle
}

fn immortal(kind: CeKind, payload: CePayload) -> usize {
    let o = Box::new(CeObject {
        ob_refcnt: isize::MAX, // immortal: INCREF/DECREF never reach 0
        ob_type: kind,
        payload: RefCell::new(Some(payload)),
    });
    Box::into_raw(o) as usize
}

// ---------------------------------------------------------------------------
// Handle construction
// ---------------------------------------------------------------------------

pub fn new_handle(kind: CeKind, payload: CePayload) -> CeHandle {
    Box::into_raw(Box::new(CeObject {
        ob_refcnt: 1,
        ob_type: kind,
        payload: RefCell::new(Some(payload)),
    }))
}

/// Value -> handle (new reference, refcount 1; immortals returned shared).
pub fn from_value(v: &Value) -> CeHandle {
    match v {
        Value::None => py_none(),
        Value::Bool(b) => {
            // Distinct singletons like CPython (Py_True is Py_True).
            if *b {
                py_true()
            } else {
                py_false()
            }
        }
        Value::Int(i) => new_handle(CeKind::Int, CePayload::Int(i.clone())),
        Value::Float(x) => new_handle(CeKind::Float, CePayload::Float(*x)),
        Value::Str(s) => new_handle(CeKind::Str, CePayload::Str(s.as_str().to_string())),
        Value::Tuple(t) => {
            let items: Vec<CeHandle> = t.0.iter().map(from_value).collect();
            new_handle(CeKind::Tuple, CePayload::Tuple(items))
        }
        Value::List(l) => {
            let items: Vec<CeHandle> = l.items().iter().map(from_value).collect();
            new_handle(CeKind::List, CePayload::List(items))
        }
        Value::Dict(d) => new_handle(CeKind::Dict, CePayload::Dict(d.clone())),
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Module => {
            let ns = o
                .module_ns
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .unwrap_or_else(|| PyDict::new());
            let name = o.name.clone();
            new_handle(CeKind::Module, CePayload::Module { name, ns })
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            // Class Values round-trip as Type handles (spec reverse lookup
            // by class name; single-spec-per-name is the S0 contract).
            let name = o.name.clone();
            match crate::slots::spec_index_for_class_name(&name) {
                Some(idx) => new_handle(CeKind::Type, CePayload::Type { spec_index: idx }),
                None => {
                    let repr = ap_obj::ops::repr(&Value::Obj(o.clone()))
                        .unwrap_or_else(|_| "<??>".to_string());
                    new_handle(CeKind::Str, CePayload::Str(repr))
                }
            }
        }
        other => {
            // Fallback: pin the repr (S0 spike never needs richer fallbacks;
            // PyO3-shaped modules only produce the kinds above).
            let repr = ap_obj::ops::repr(other).unwrap_or_else(|_| "<??>".to_string());
            new_handle(CeKind::Str, CePayload::Str(repr))
        }
    }
}

/// INCREF: bump the emulation.
pub fn ce_newref(h: CeHandle) {
    unsafe {
        let r = &mut (*h).ob_refcnt;
        if *r != isize::MAX {
            *r += 1;
        }
    }
}

/// DECREF: drop the emulation. NEVER frees (tracing GC owns memory).
pub fn ce_xdecref(h: CeHandle) {
    unsafe {
        let r = &mut (*h).ob_refcnt;
        if *r != isize::MAX && *r > 0 {
            *r -= 1;
        }
    }
}

pub fn refcnt(h: CeHandle) -> isize {
    unsafe { (*h).ob_refcnt }
}

// ---------------------------------------------------------------------------
// Registries (shared with slots.rs / api.rs via OnceLock statics here).
// ---------------------------------------------------------------------------


pub fn slot_registry() -> &'static Mutex<Vec<crate::slots::SpecRecord>> {
    static R: OnceLock<Mutex<Vec<crate::slots::SpecRecord>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

// ---------------------------------------------------------------------------
// C-side error state (thread-local; the spike module runs on the
// interpreter thread).
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ErrState {
    pub set: bool,
    pub class: String,
    pub msg: String,
}

thread_local! {
    pub static ERR: RefCell<ErrState> = RefCell::new(ErrState::default());
}

pub fn err_set(class: &str, msg: impl Into<String>) {
    ERR.with(|e| {
        *e.borrow_mut() = ErrState {
            set: true,
            class: class.to_string(),
            msg: msg.into(),
        };
    });
}

pub fn err_take() -> Option<(String, String)> {
    ERR.with(|e| {
        let mut b = e.borrow_mut();
        if b.set {
            b.set = false;
            Some((std::mem::take(&mut b.class), std::mem::take(&mut b.msg)))
        } else {
            None
        }
    })
}

pub fn err_is_set() -> bool {
    ERR.with(|e| e.borrow().set)
}

/// Test/bridge helper: reset the thread error state.
pub fn err_clear() {
    ERR.with(|e| *e.borrow_mut() = ErrState::default());
}

/// Allocator seam (PyMem_Malloc route).
pub fn mem_malloc(n: usize) -> *mut c_void {
    unsafe { libc::malloc(n) }
}

pub fn mem_free(p: *mut c_void) {
    unsafe { libc::free(p) }
}

impl CeObject {
    pub fn type_name(&self) -> &'static str {
        match self.ob_type {
            CeKind::None => "NoneType",
            CeKind::Bool => "bool",
            CeKind::Int => "int",
            CeKind::Float => "float",
            CeKind::Str => "str",
            CeKind::Tuple => "tuple",
            CeKind::List => "list",
            CeKind::Dict => "dict",
            CeKind::Module => "module",
            CeKind::CFunction => "builtin_function_or_method",
            CeKind::Type => "type",
            CeKind::Instance => "object", // refined via spec name at bridge
        }
    }
}

/// Convenience used by value_conv: mint the dotted BuiltinFn marker the
/// runtime dispatches (mod.name) for a registered C function.
pub fn marker_for(module: &str, name: &str) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::BuiltinFn, format!("{module}.{name}"));
    Value::Obj(o)
}

// Re-export Arc for value_conv's Instance reconstruction.
pub use std::sync::Arc as CeArc;

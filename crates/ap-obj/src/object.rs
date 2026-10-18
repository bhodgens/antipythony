//! The generic heap-object escape hatch (`Value::Obj`) for the v0 executor.
//!
//! The plain `Value` arms cover immutable builtin data. Everything the
//! tier-0 executor allocates that has *identity* (functions, bound methods,
//! type objects, modules, ranges, slices, iterators, generators, exception
//! instances) lives behind `PyObject`: a type tag + interior-mutable state.
//!
//! Semantics by design:
//! - **eq / hash are IDENTITY-based** (CPython's default object protocol):
//!   two distinct function objects are never equal, a function is hashable,
//!   an exception instance is hashable (default `object.__hash__`).
//! - **truthy by default** (`object.__bool__` absent → True).
//! - attributes resolve through the executor's attr surface (`attrs` map for
//!   instance-shaped objects; method dispatch on the type tag happens in
//!   ap-run, which owns the method tables).
//!
//! 02.03 (classes/protocols) replaces the tag dispatch with real type
//! objects + MRO + descriptors; the payload kinds stay.
//!
//! NOT here: GC. `Gc<T>` is the `Rc` placeholder (02.04); reference cycles
//! through closures/generators leak until then — documented v0 divergence.

use crate::int::Gc;
use crate::value::Value;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// What kind of object this is. The executor matches on this to dispatch
/// calls/methods; the tag also names the type (`type(v).__name__`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjKind {
    /// A user-defined (or builtin-named) function object: holds its IR body
    /// indirectly through the executor's function table, keyed by `FnId`.
    Function,
    /// A bound method: function + receiver value.
    BoundMethod,
    /// A builtin function implemented in Rust (print, len, ...).
    BuiltinFn,
    /// A type object (`int`, `str`, a user class stub). Holds just a name;
    /// the executor resolves constructor behavior from the name.
    Type,
    /// A module object (the `sys` stub in v0).
    Module,
    /// `range` object: start/stop/step ints; iterable, hashable, reversible.
    Range,
    /// A materialized `slice` object (`slice(1, 10, 2)` / as subscript arg).
    SliceObj,
    /// An iterator handle; state keyed in the executor's iterator table.
    Iterator,
    /// A generator: suspended frame + resumable state machine.
    Generator,
    /// An exception INSTANCE (`ValueError("x")`): class name + args tuple +
    /// traceback slot (filled as it propagates).
    Exception,
    /// `sys.flags`-style inert namespace bags (v0 sys stub internals).
    Namespace,
    /// `sys.version_info`-style named tuple (indexable + named access).
    NamedTuple,
    /// A class defined by a v0 program (02.03 replaces with real types).
    /// 02.02 only needs it to exist so `class` statements can fail with the
    /// right diagnostic after binding, and `__len__`/`__bool__`/`__str__`/
    /// `__repr__` dunder protocols (V048/V050/V066/V012 f-string conv) work.
    UserClass,
    /// An instance of a UserClass.
    UserInstance,
    /// A classmethod/staticmethod/property descriptor wrapper (v0 minimal).
    Descriptor,
    /// Any other singleton the executor needs an identity for.
    Opaque,
}

/// Interior state of a generic object. Which fields are meaningful depends
/// on [`ObjKind`]; unused fields stay default.
pub struct PyObject {
    pub kind: ObjKind,
    /// Type/name string (function name, class name, module name, `<lambda>`).
    pub name: String,
    /// For Function: index into the executor's function table.
    pub fn_id: Option<u32>,
    /// For BoundMethod: the receiver.
    pub self_value: RefCell<Option<Value>>,
    /// For BoundMethod/BuiltinFn: the underlying callable.
    pub func: RefCell<Option<Value>>,
    /// For Range: (start, stop, step) ints. For NamedTuple: field names.
    pub ints: RefCell<Vec<i64>>,
    /// For SliceObj: (lower, upper, step) as Values.
    pub slice_parts: RefCell<Vec<Value>>,
    /// Attribute namespace (UserInstance, Module, Namespace, UserClass
    /// method tables). Identity map by string key; last write wins.
    pub attrs: RefCell<Gc<crate::dict::PyDict>>,
    /// For Exception: the args tuple (str(exc) renders per S-06-7).
    pub args: RefCell<Vec<Value>>,
    /// For Exception: `__cause__` / `__context__` handles (chaining).
    pub cause: RefCell<Option<Value>>,
    pub context: RefCell<Option<Value>>,
    /// For UserClass: which dunders are defined (name -> fn Value).
    pub dunders: RefCell<crate::dict::PyDict>,
    /// For Generator: executor-resumable state (opaque handle id).
    pub gen_id: RefCell<Option<u64>>,
    /// For Namespace (sys.float_info etc.): printable repr override.
    pub repr_text: RefCell<Option<String>>,
    /// For Module: exported names dict (lazy attribute reads).
    pub module_ns: RefCell<Option<std::rc::Rc<crate::dict::PyDict>>>,
    /// For Function: executor-side closure state slot (defining-frame id
    /// in the interpreter's frame table; the executor owns the typing).
    pub closure_slot: RefCell<Option<u64>>,
    /// For UserClass: the class payload (TypeObject: MRO, dict, flags).
    /// Strong handle — the TypeObject never points back at the PyObject.
    pub class_ref: RefCell<Option<Rc<crate::classes::TypeObject>>>,
    /// For UserInstance: a WEAK back-pointer to the instance's class
    /// payload (the strong handle lives on the class PyObject; instances
    /// must not keep their class's payload alive solo — no cycles).
    pub type_ref: RefCell<Option<Weak<crate::classes::TypeObject>>>,
    /// For Descriptor: which descriptor kind ("property"/"classmethod"/
    /// "staticmethod"/"member").
    pub descr_kind: RefCell<Option<String>>,
    /// For BoundMethod: the class whose dict the function was found in
    /// (zero-arg super() start-class, S-03-17/18).
    pub method_owner: RefCell<Option<std::rc::Weak<crate::classes::TypeObject>>>,
}

impl PyObject {
    pub fn new(kind: ObjKind, name: impl Into<String>) -> Gc<PyObject> {
        Gc::new(PyObject {
            kind,
            name: name.into(),
            fn_id: None,
            self_value: RefCell::new(None),
            func: RefCell::new(None),
            ints: RefCell::new(Vec::new()),
            slice_parts: RefCell::new(Vec::new()),
            attrs: RefCell::new(crate::dict::PyDict::new()),
            args: RefCell::new(Vec::new()),
            cause: RefCell::new(None),
            context: RefCell::new(None),
            dunders: RefCell::new((*crate::dict::PyDict::new()).clone()),
            gen_id: RefCell::new(None),
            repr_text: RefCell::new(None),
            module_ns: RefCell::new(None),
            closure_slot: RefCell::new(None),
            class_ref: RefCell::new(None),
            type_ref: RefCell::new(None),
            descr_kind: RefCell::new(None),
            method_owner: RefCell::new(None),
        })
    }

    /// Constructor with the receiver preset (bound methods).
    pub fn with_self(kind: ObjKind, name: impl Into<String>, self_value: Value) -> Gc<PyObject> {
        let o = PyObject::new(kind, name);
        *o.self_value.borrow_mut() = Some(self_value);
        o
    }

    /// Constructor with a fn id preset (function objects).
    pub fn with_fn_id(kind: ObjKind, name: impl Into<String>, fn_id: u32) -> Gc<PyObject> {
        Gc::new(PyObject {
            kind,
            name: name.into(),
            fn_id: Some(fn_id),
            self_value: RefCell::new(None),
            func: RefCell::new(None),
            ints: RefCell::new(Vec::new()),
            slice_parts: RefCell::new(Vec::new()),
            attrs: RefCell::new(crate::dict::PyDict::new()),
            args: RefCell::new(Vec::new()),
            cause: RefCell::new(None),
            context: RefCell::new(None),
            dunders: RefCell::new((*crate::dict::PyDict::new()).clone()),
            gen_id: RefCell::new(None),
            repr_text: RefCell::new(None),
            module_ns: RefCell::new(None),
            closure_slot: RefCell::new(None),
            class_ref: RefCell::new(None),
            type_ref: RefCell::new(None),
            descr_kind: RefCell::new(None),
            method_owner: RefCell::new(None),
        })
    }

    /// The Python type name this object reports (`type(v).__name__`).
    pub fn type_name(&self) -> String {
        match self.kind {
            ObjKind::Function | ObjKind::BuiltinFn => "function".to_string(),
            ObjKind::BoundMethod => "method".to_string(),
            ObjKind::Type | ObjKind::UserClass => "type".to_string(),
            ObjKind::Module => "module".to_string(),
            ObjKind::Range => "range".to_string(),
            ObjKind::SliceObj => "slice".to_string(),
            ObjKind::Iterator => "iterator".to_string(),
            ObjKind::Generator => "generator".to_string(),
            ObjKind::Exception => self.name.clone(),
            ObjKind::Namespace => self.name.clone(),
            ObjKind::NamedTuple => self.name.clone(),
            ObjKind::UserInstance => self.name.clone(),
            ObjKind::Descriptor => self.name.clone(),
            ObjKind::Opaque => "object".to_string(),
        }
    }

    /// Default-object-protocol repr: `<module.Class object at 0x…>` for
    /// instances, `<function 'f'>`-style for callables. Addresses are
    /// UNSPECIFIED (C-24) — corpus masks them (V066 splits at ' at 0x').
    pub fn default_repr(&self, ptr: usize) -> String {
        match self.kind {
            ObjKind::UserInstance => {
                format!(
                    "<{}.{} object at 0x{:x}>",
                    crate::MODULE_NAME,
                    self.name,
                    ptr
                )
            }
            // Class objects: <class 'name'> (S-03-04; c001 __bases__ repr).
            ObjKind::UserClass => format!("<class '{}'>", self.name),
            ObjKind::Function => format!("<function {0} at 0x{1:x}>", self.name, ptr),
            _ => format!("<{} object at 0x{:x}>", self.name, ptr),
        }
    }
}

impl std::fmt::Debug for PyObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PyObject({:?}, {})", self.kind, self.name)
    }
}

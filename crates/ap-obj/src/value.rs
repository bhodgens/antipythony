//! The tagged `Value` and its heap object types (see crate docs for the
//! representation tradeoff: enum-of-8 + `Gc<T>` handle, NOT NaN-boxing).

use crate::dict::PyDict;
use crate::int::{Gc, Int};
use crate::object::ObjKind;
use crate::object::PyObject;

/// The weak target address of a weakref token object (`_weakref_token` /
/// `_weakref_proxy` carry `<target_addr>`), or None for other objects.
/// Tokens hash and compare BY TARGET (CPython: two refs to the same object
/// are equal and hash alike — the WeakSet/WKD data model requires it).
pub fn weakref_target_addr(o: &PyObject) -> Option<usize> {
    if o.kind != ObjKind::Opaque {
        return None;
    }
    if o.name != "_weakref_token" && o.name != "_weakref_proxy" {
        return None;
    }
    o.attrs
        .borrow()
        .get(&Value::Str(PyString::new("<target_addr>")))
        .and_then(|v| v.as_int())
        .and_then(|i| i.to_i64())
        .map(|n| n.max(0) as usize)
}

/// Static type name for a dynamic object (the `type_name()` fallback).
/// Exception instances and UserInstances report their CLASS name, which is
/// dynamic — those go through `PyObject::type_name()` at the surfaces that
/// render it; this returns the container-neutral names for error messages.
pub(crate) fn obj_static_type_name(kind: ObjKind) -> &'static str {
    match kind {
        ObjKind::Function | ObjKind::BuiltinFn => "function",
        ObjKind::BoundMethod => "method",
        ObjKind::Type | ObjKind::UserClass => "type",
        ObjKind::Module => "module",
        ObjKind::Range => "range",
        ObjKind::SliceObj => "slice",
        ObjKind::Iterator => "iterator",
        ObjKind::Generator => "generator",
        ObjKind::Exception => "BaseException",
        ObjKind::Namespace | ObjKind::NamedTuple => "object",
        ObjKind::UserInstance => "object",
        ObjKind::Descriptor => "object",
        ObjKind::Opaque => "object",
    }
}

/// A Python value (v1 core object model; classes/user types are 02.03).
///
/// Size: 16 bytes. Immediates (smi int, float, bool, None) are inline; every
/// reference-valued object (str, bytes, list, dict, tuple, set, bigint) sits
/// behind `Gc<T>` — a handle that 02.04 turns into the MMTk-managed pointer.
#[derive(Debug, Clone)]
pub enum Value {
    None,
    /// `bool ⊂ int` (S-01-9): distinct arm, but every numeric op treats it
    /// as 0/1 via `as_int()`; `type()` distinguishes, arithmetic does not.
    Bool(bool),
    Int(Int),
    Float(f64),
    Str(Gc<PyString>),
    Bytes(Gc<Vec<u8>>),
    List(Gc<PyList>),
    Tuple(Gc<Tuple>),
    Dict(Gc<PyDict>),
    Set(Gc<PySet>),
    /// `frozenset` — a distinct type from 02.02 (S-01-26: hashable, order-
    /// independent equality/hash; non-empty repr order UNSPECIFIED per C-26).
    /// The storage is the same hash-bucketed set; the wrapper only changes
    /// the type name, hash, and repr.
    Frozen(Gc<PySet>),
    /// `...` (Ellipsis singleton; truthy, identity-compared, repr `Ellipsis`).
    Ellipsis,
    /// Everything the executor allocates that is not a plain builtin value:
    /// type objects, modules, functions, bound methods, ranges, slices,
    /// iterators, generators, exception instances. Identity semantics
    /// (eq/hash by object identity, like CPython's default object protocol).
    Obj(Gc<crate::object::PyObject>),
}

use crate::dict::PySet;

impl Value {
    /// Numeric view for bool-as-int contexts (S-01-9): `True` → 1.
    pub fn as_int(&self) -> Option<Int> {
        match self {
            Value::Bool(b) => Some(Int::Smi(i64::from(*b))),
            Value::Int(i) => Some(i.clone()),
            _ => None,
        }
    }

    /// The Python type name (`type(v).__name__`), used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::None => "NoneType",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Bytes(_) => "bytes",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Dict(_) => "dict",
            Value::Set(_) => "set",
            Value::Frozen(_) => "frozenset",
            Value::Ellipsis => "ellipsis",
            // Dynamic objects report through PyObject (function, module,
            // range, generator, the exception CLASS NAME, ...). Static
            // callers that need a static string use the fallback.
            Value::Obj(o) => obj_static_type_name(o.kind),
        }
    }

    /// Truthiness (S-01-27): falsy = False, None, every numeric zero, and
    /// every empty container. `__bool__`/`__len__` protocol dispatch is
    /// 02.03 (user classes); builtins resolve directly.
    pub fn is_truthy(&self) -> bool {
        match self {
            Value::None => false,
            Value::Bool(b) => *b,
            Value::Int(i) => !i.is_zero(),
            Value::Float(f) => *f != 0.0, // -0.0 == 0.0 is falsy (S-01-27)
            Value::Str(s) => !s.as_str().is_empty(),
            Value::Bytes(b) => !b.is_empty(),
            Value::List(l) => l.len() != 0,
            Value::Tuple(t) => !t.0.is_empty(),
            Value::Dict(d) => d.len() != 0,
            Value::Set(s) => s.len() != 0,
            // frozenset truthiness follows len (S-01-27; __bool__ absent).
            Value::Frozen(s) => s.len() != 0,
            // `...` is truthy; generic objects default truthy (object
            // protocol: no __bool__/__len__ → True). UserInstance truthiness
            // is dispatched by the executor BEFORE falling back here.
            Value::Ellipsis => true,
            Value::Obj(_) => true,
        }
    }
}

/// CPython-style string: compact ASCII or full Unicode (crate docs §strings).
///
/// Carries a lazily-cached code-point count for the UTF-8 arm and a cached
/// hash — the cached-hash pattern is monty's (`types/str.rs`), adapted.
pub struct PyString {
    repr: StrRepr,
    cached_len: std::cell::Cell<Option<usize>>,
    cached_hash: std::cell::Cell<Option<i64>>,
}

// Manual Clone (cached values copy over — pure optimization).

#[derive(Debug, Clone)]
enum StrRepr {
    /// Pure ASCII: bytes == code points.
    Ascii(Box<str>),
    /// Contains non-ASCII: UTF-8 (Rust String).
    Unicode(Box<str>),
}

impl PyString {
    pub fn new(s: impl Into<Box<str>>) -> Gc<PyString> {
        let s = s.into();
        let repr = if s.is_ascii() {
            StrRepr::Ascii(s)
        } else {
            StrRepr::Unicode(s)
        };
        Gc::new(PyString {
            repr,
            cached_len: std::cell::Cell::new(None),
            cached_hash: std::cell::Cell::new(None),
        })
    }

    pub fn as_str(&self) -> &str {
        match &self.repr {
            StrRepr::Ascii(s) | StrRepr::Unicode(s) => s,
        }
    }

    pub fn is_ascii(&self) -> bool {
        matches!(self.repr, StrRepr::Ascii(_))
    }

    /// `len()` — code points, not bytes (S-01-14).
    pub fn len(&self) -> usize {
        if let Some(n) = self.cached_len.get() {
            return n;
        }
        let n = match &self.repr {
            StrRepr::Ascii(s) => s.len(),
            StrRepr::Unicode(s) => s.chars().count(),
        };
        self.cached_len.set(Some(n));
        n
    }

    /// Cached Python hash (str hash is O(n); strings are hashed repeatedly
    /// as dict keys — CPython caches too).
    ///
    /// CPython hashes the string's INTERNAL byte representation: compact
    /// latin1 (1 byte/char) when every code point ≤ 0xFF, UCS2/UCS4 beyond.
    /// We mirror the latin1/UTF-8 split (see crate docs §strings): latin1-
    /// representable strings hash their latin1 bytes, wider strings hash
    /// UTF-8 bytes (CPython's UCS4 hashes differ from UTF-8 bytes, but both
    /// are UNSPECIFIED per S-01-38 — only equal-content-equal-hash matters,
    /// and pinned-seed-0 reproducibility is per-implementation).
    pub fn py_hash(&self) -> i64 {
        if let Some(h) = self.cached_hash.get() {
            return h;
        }
        let text = self.as_str();
        let h = if self.is_ascii() {
            crate::hash::hash_bytes(text.as_bytes())
        } else if text.chars().all(|c| (c as u32) <= 0xFF) {
            let latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
            crate::hash::hash_bytes(&latin1)
        } else {
            crate::hash::hash_bytes(text.as_bytes())
        };
        self.cached_hash.set(Some(h));
        h
    }
}

impl Clone for PyString {
    fn clone(&self) -> Self {
        PyString {
            repr: self.repr.clone(),
            cached_len: std::cell::Cell::new(self.cached_len.get()),
            cached_hash: std::cell::Cell::new(self.cached_hash.get()),
        }
    }
}

impl std::fmt::Debug for PyString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PyString({:?})", self.as_str())
    }
}

impl PartialEq for PyString {
    /// Content equality only; caches are pure optimization (monty's rule).
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}
impl Eq for PyString {}

/// Immutable tuple.
#[derive(Debug, Clone)]
pub struct Tuple(pub Vec<Value>);

impl PartialEq for Tuple {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .all(|(a, b)| crate::dict::values_eq(a, b))
    }
}

impl Tuple {
    pub fn iter(&self) -> std::slice::Iter<'_, Value> {
        self.0.iter()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Insertion-ordered mutable list with CPython's over-allocation growth
/// (pattern documented in crate docs; growth factor is unobservable).
pub struct PyList {
    /// Interior-mutable through shared `Gc` handles: `ys = xs; ys.append(4)`
    /// mutates `xs` (S-01-19 aliasing). `RefCell` today; 02.07's
    /// thread-safety audit picks the concurrent wrapper.
    items: std::cell::RefCell<Vec<Value>>,
    /// Capacity actually reserved (CPython `allocated`); kept so growth
    /// mirrors CPython's `newsize + newsize>>3 + 6` pattern, though the
    /// exact values are unobservable (S-01-31-adjacent).
    allocated: std::cell::Cell<usize>,
}

impl Clone for PyList {
    fn clone(&self) -> Self {
        PyList {
            items: std::cell::RefCell::new(self.items.borrow().clone()),
            allocated: std::cell::Cell::new(0),
        }
    }
}

impl std::fmt::Debug for PyList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyList")
            .field("len", &self.items.borrow().len())
            .finish()
    }
}

impl PyList {
    pub fn new() -> Gc<PyList> {
        let l = Gc::new(PyList {
            items: std::cell::RefCell::new(Vec::new()),
            allocated: std::cell::Cell::new(0),
        });
        crate::gc_gate::gc_register_pub(&Value::List(l.clone()));
        l
    }

    pub fn from_vec(items: Vec<Value>) -> Gc<PyList> {
        let l = Gc::new(PyList {
            items: std::cell::RefCell::new(items),
            allocated: std::cell::Cell::new(0),
        });
        crate::gc_gate::gc_register_pub(&Value::List(l.clone()));
        l
    }

    pub fn len(&self) -> usize {
        self.items.borrow().len()
    }

    pub fn items(&self) -> std::cell::Ref<'_, [Value]> {
        std::cell::Ref::map(self.items.borrow(), |v| &v[..])
    }

    /// CPython's over-allocation formula (listobject.c list_resize):
    /// `new_allocated = (newsize + newsize>>3 + 6) & !3` — replicated for
    /// capacity discipline; `Vec` does the actual (re)allocation.
    fn reserve_for(&self, newsize: usize) {
        if newsize > self.allocated.get() {
            let new_allocated = (newsize + (newsize >> 3) + 6) & !3;
            self.items.borrow_mut().reserve(new_allocated - newsize);
            self.allocated.set(new_allocated);
        }
    }

    pub fn append(&self, v: Value) {
        let n = self.items.borrow().len() + 1;
        self.reserve_for(n);
        self.items.borrow_mut().push(v);
    }

    pub fn get(&self, i: usize) -> Option<Value> {
        self.items.borrow().get(i).cloned()
    }

    /// Overwrite `items[i]` (in-bounds assignment; caller validated).
    pub fn set_at(&self, i: usize, v: Value) {
        self.items.borrow_mut()[i] = v;
    }

    /// Remove `items[i]` (`del xs[i]`; caller validated the index).
    pub fn remove_at(&self, i: usize) {
        self.items.borrow_mut().remove(i);
    }

    /// Remove `items[lo..hi]` (`del xs[lo:hi]`; caller validated the range).
    pub fn remove_range(&self, lo: usize, hi: usize) {
        if lo < hi {
            self.items.borrow_mut().drain(lo..hi);
        }
    }

    /// `xs.clear()`.
    pub fn clear(&self) {
        self.items.borrow_mut().clear();
        self.allocated.set(0);
    }

    /// Shared `repr` recursion helper: format with CPython punctuation
    /// (`[a, b]`), cycle-protecting self-references with `[...]` (S-01-44).
    pub(crate) fn fmt_repr(
        &self,
        out: &mut String,
        active: &mut std::collections::HashSet<usize>,
        id: usize,
    ) {
        if !active.insert(id) {
            out.push_str("[...]");
            return;
        }
        out.push('[');
        let items = self.items.borrow();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            item.fmt_repr(out, active);
        }
        out.push(']');
        active.remove(&id);
    }
}

impl Value {
    /// Recursive repr with cycle detection (S-01-44: self-referential list →
    /// `[1, [...]]`). `active` holds address-keys of containers currently
    /// being rendered. For self-referential tuples CPython prints the full
    /// recursion via `RecursionError` at runtime — value-layer reprs of
    /// tuple cycles hit the same ellipsis here (documented: constructing a
    /// tuple cycle requires a container mutation that only lists/dicts
    /// support in v1, so this branch is defensive).
    pub fn fmt_repr(&self, out: &mut String, active: &mut std::collections::HashSet<usize>) {
        match self {
            Value::None => out.push_str("None"),
            Value::Bool(true) => out.push_str("True"),
            Value::Bool(false) => out.push_str("False"),
            Value::Int(i) => out.push_str(&i.to_string_raw()),
            Value::Float(f) => out.push_str(&crate::float_repr::py_float_repr(*f)),
            Value::Str(s) => out.push_str(&crate::str_repr::str_repr(s.as_str())),
            Value::Bytes(b) => out.push_str(&crate::str_repr::bytes_repr(b)),
            Value::List(l) => l.fmt_repr(out, active, addr_of(l)),
            Value::Tuple(t) => {
                out.push('(');
                for (i, item) in t.0.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    item.fmt_repr(out, active);
                }
                if t.0.len() == 1 {
                    out.push(','); // 1-tuple keeps the comma (S-01-20/44)
                }
                out.push(')');
            }
            Value::Dict(d) => d.fmt_repr(out, active),
            Value::Set(s) => s.fmt_repr(out, active),
            Value::Frozen(s) => {
                // frozenset repr (S-01-26): `frozenset()` empty,
                // `frozenset({e1, e2})` otherwise, internal order (C-26).
                let elems = s.sorted_elements_public();
                if elems.is_empty() {
                    out.push_str("frozenset()");
                } else {
                    out.push_str("frozenset({");
                    for (i, e) in elems.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        e.fmt_repr(out, active);
                    }
                    out.push_str("})");
                }
            }
            Value::Ellipsis => out.push_str("Ellipsis"),
            // Generic objects: repr is rendered by the executor (functions,
            // modules, exception str() need executor context). The value
            // layer renders the default object-protocol form.
            Value::Obj(o) => out.push_str(&o.default_repr(crate::value::addr_of(o))),
        }
    }
}

/// Address key for cycle detection: heap objects are unique per identity;
/// the Rc payload pointer is heap-stable and shared across clones, so the
/// recursion guard compares equal for the same object. When `Gc` becomes the
/// MMTk handle (02.04) this becomes the handle id.
pub fn addr_of<T>(r: &Gc<T>) -> usize {
    std::rc::Rc::as_ptr(r).addr()
}

/// Address of any Obj value's payload (0 for immediates) — the weakref
/// token key space.
pub fn addr_of_value(v: &Value) -> usize {
    match v {
        Value::Str(x) => addr_of(x),
        Value::Bytes(x) => addr_of(x),
        Value::List(x) => addr_of(x),
        Value::Tuple(x) => addr_of(x),
        Value::Dict(x) => addr_of(x),
        Value::Set(x) | Value::Frozen(x) => addr_of(x),
        Value::Obj(x) => addr_of(x),
        _ => 0,
    }
}

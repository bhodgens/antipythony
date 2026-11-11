//! The tagged `Value` and its heap object types (see crate docs for the
//! representation tradeoff: enum-of-8 + `Gc<T>` handle, NOT NaN-boxing).

use crate::dict::PyDict;
use crate::int::{Gc, Int};
use crate::object::ObjKind;
use crate::object::PyObject;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// A4 (memperf audit 2026-11-06): per-thread interned name handles.
//
// The object model historically minted a fresh `Gc<PyString>` (one heap alloc)
// for every magic/dunder name probe on hot paths: MRO attribute lookups,
// weakref target reads (`<target_addr>`), `values_eq` object arms, the GC
// support layer. The names are a small fixed set of `&'static str` constants,
// so a per-thread cache turns every probe into a HashMap hit + Arc bump
// (content-equal handles are reused; dict probing compares by content, so
// identity of the handle is an optimization, never a semantic).
//
// Thread-local by construction: `Gc<T> = Arc<T>` refcounts must not be shared
// across threads outside the GIL protocol, and a per-thread cache hands each
// thread its own Arc clone. (Under 02.07's GIL v1 the main-thread cache holds
// every handle the collector probes.)
// ---------------------------------------------------------------------------

thread_local! {
    static INTERNED_NAMES: std::cell::RefCell<std::collections::HashMap<String, Gc<PyString>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The interned `Gc<PyString>` handle for a process-static name. Same
/// content as `PyString::new(name)`, cached hash included (PyString caches
/// its hash lazily on first use, so a shared handle amortizes the siphash
/// across every probe of that name).
pub fn interned_str(name: &'static str) -> Gc<PyString> {
    interned_str_ref(name)
}

/// A4: interned handle for a BORROWED name — hot-path attr/dict probes
/// (MRO lookups take `&str` of runtime lifetime) get one Arc bump + the
/// handle's CACHED hash, replacing the per-probe `PyString::new` alloc and
/// full siphash. A miss allocates once per distinct name per thread.
pub fn interned_str_ref(name: &str) -> Gc<PyString> {
    INTERNED_NAMES.with(|c| {
        if let Some(s) = c.borrow().get(name) {
            return s.clone();
        }
        let s = PyString::new(name);
        c.borrow_mut().insert(name.to_string(), s.clone());
        s
    })
}

/// The interned handle wrapped as a dict-key `Value` (spares callers the
/// `Value::Str(PyString::new(..))` allocation per probe).
pub fn interned_key(name: &'static str) -> Value {
    Value::Str(interned_str(name))
}

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
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_str("<target_addr>")
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
/// Size: 16 bytes — C10 (memperf audit 2026-11-06) claimed "≥24: the Int
/// arm embeds a 16-byte enum", which the compiler disproves by niche
/// packing: `Int` (tag word + payload word) leaves spare tag bit-patterns,
/// and the outer `Value` tag reuses that niche instead of widening the
/// layout. `unit_tests::value_size_is_16_bytes_doc_pinned` measures the
/// real size every test run, so this doc can never silently rot again.
/// Immediates (smi int, float, bool, None) are inline; every
/// reference-valued object (str, bytes, list, dict, tuple, set, bigint)
/// sits behind `Gc<T>` — a handle that 02.04 turns into the MMTk-managed
/// pointer.
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
            Value::Dict(d) => !d.is_empty(),
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

/// 02.09 phase 1 (docs/no-gil-object-audit.md entry 27): `PyString.cached_len`
/// sentinel for "not yet computed" — a string of `usize::MAX` code points
/// cannot be allocated on any supported target, so the sentinel can never
/// collide with a real length.
const LEN_UNKNOWN: usize = usize::MAX;

/// CPython-style string: compact ASCII or full Unicode (crate docs §strings).
///
/// Carries a lazily-cached code-point count for the UTF-8 arm and a cached
/// hash — the cached-hash pattern is monty's (`types/str.rs`), adapted.
pub struct PyString {
    repr: StrRepr,
    cached_len: std::sync::atomic::AtomicUsize,
    /// 02.09a P4 (entry 28, hash-domain audit passed — see py_hash): the
    /// cache is AtomicI64 with 0 = "not yet computed" and the CPython
    /// -1 => -2 normalization applied before every store. Same-value race
    /// writes the same bits (idempotent, like entry 27's cached_len).
    cached_hash: std::sync::atomic::AtomicI64,
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
            cached_len: std::sync::atomic::AtomicUsize::new(LEN_UNKNOWN),
            cached_hash: std::sync::atomic::AtomicI64::new(0),
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
    ///
    /// 02.09 phase 1 (docs/no-gil-object-audit.md entry 27): the lazy fill
    /// is an idempotent same-value cache — `Cell<Option<usize>>` became an
    /// `AtomicUsize` with `usize::MAX` as the None sentinel (a string of
    /// `usize::MAX` code points is unallocatable on every supported target).
    /// Relaxed is sufficient: racing fills write identical bits, so any
    /// winner is indistinguishable from the GIL-serialized fill.
    #[allow(clippy::len_without_is_empty)] // is_empty would extend the public surface beyond the CPython-parity API
    pub fn len(&self) -> usize {
        let cached = self.cached_len.load(std::sync::atomic::Ordering::Relaxed);
        if cached != LEN_UNKNOWN {
            return cached;
        }
        let n = match &self.repr {
            StrRepr::Ascii(s) => s.len(),
            StrRepr::Unicode(s) => s.chars().count(),
        };
        self.cached_len
            .store(n, std::sync::atomic::Ordering::Relaxed);
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
    ///
    /// 02.09a P4 hash-domain audit (the phase-1 doc's explicit
    /// precondition): the cache is AtomicI64 with the -1 => None sentinel
    /// mapping. PROOF the domain never collides with the sentinel:
    ///   1. `crate::hash::hash_bytes` returns `r as i64` where r:u64 from
    ///      siphash13 — the only value that reinterprets to -1 is
    ///      u64::MAX (0xFFFF...F), i.e. the FULL siphash output.
    ///   2. CPython applies the same -1 => -2 mapping to str hashes
    ///      (unicodeobject.c: `if (x == -1) x = -2`) — we mirror it
    ///      BELOW (normalize -1 to -2 before caching), so -1 can never
    ///      enter the cache.
    ///   3. The empty string short-circuits to 0 (hash_bytes returns 0).
    /// Therefore cached values are in [-2^61..2^61] \ {-1} ∪ {0} and the
    /// 0 sentinel for "not yet computed" is unambiguous (any hash == 0
    /// recomputes to 0 — idempotent, benign).
    pub fn py_hash(&self) -> i64 {
        let cached = self.cached_hash.load(std::sync::atomic::Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        let text = self.as_str();
        let mut h = if self.is_ascii() {
            crate::hash::hash_bytes(text.as_bytes())
        } else if text.chars().all(|c| (c as u32) <= 0xFF) {
            let latin1: Vec<u8> = text.chars().map(|c| c as u8).collect();
            crate::hash::hash_bytes(&latin1)
        } else {
            crate::hash::hash_bytes(text.as_bytes())
        };
        if h == -1 {
            // CPython parity: -1 is the error/sentinel slot; map to -2.
            h = -2;
        }
        self.cached_hash
            .store(h, std::sync::atomic::Ordering::Relaxed);
        h
    }
}

impl Clone for PyString {
    fn clone(&self) -> Self {
        PyString {
            repr: self.repr.clone(),
            cached_len: std::sync::atomic::AtomicUsize::new(
                self.cached_len.load(std::sync::atomic::Ordering::Relaxed),
            ),
            cached_hash: std::sync::atomic::AtomicI64::new(
                self.cached_hash.load(std::sync::atomic::Ordering::Relaxed),
            ),
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
    #[allow(clippy::len_without_is_empty)] // is_empty would extend the public surface beyond the CPython-parity API
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Insertion-ordered mutable list with CPython's over-allocation growth
/// (pattern documented in crate docs; growth factor is unobservable).
pub struct PyList {
    /// Interior-mutable through shared `Gc` handles: `ys = xs; ys.append(4)`
    /// mutates `xs` (S-01-19 aliasing).
    /// 02.09b P7 (decision 2, audit entry 26): Mutex — items + allocated
    /// move under the SAME lock so the capacity invariant cannot tear
    /// between a growing worker and a reading one (the GC's `items()`
    /// scan reads under the same lock; contention flips SCAN_CONTENDED).
    items: Mutex<Vec<Value>>,
    /// Capacity actually reserved (CPython `allocated`); kept so growth
    /// mirrors CPython's `newsize + newsize>>3 + 6` pattern, though the
    /// exact values are unobservable (S-01-31-adjacent).
    allocated: std::cell::Cell<usize>,
}

/// 02.09b P7: read guard for `PyList::items()` — Deref to `[Value]`.
pub struct ItemsRef<'a>(std::sync::MutexGuard<'a, Vec<Value>>);

impl std::ops::Deref for ItemsRef<'_> {
    type Target = [Value];
    fn deref(&self) -> &[Value] {
        &self.0[..]
    }
}

impl Clone for PyList {
    fn clone(&self) -> Self {
        PyList {
            items: Mutex::new(self.items.lock().unwrap_or_else(|p| p.into_inner()).clone()),
            allocated: std::cell::Cell::new(0),
        }
    }
}

impl std::fmt::Debug for PyList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyList")
            .field(
                "len",
                &self.items.lock().unwrap_or_else(|p| p.into_inner()).len(),
            )
            .finish()
    }
}

impl PyList {
    pub fn new() -> Gc<PyList> {
        let l = Gc::new(PyList {
            items: Mutex::new(Vec::new()),
            allocated: std::cell::Cell::new(0),
        });
        crate::gc_gate::gc_register_pub(&Value::List(l.clone()));
        l
    }

    pub fn from_vec(items: Vec<Value>) -> Gc<PyList> {
        let l = Gc::new(PyList {
            items: Mutex::new(items),
            allocated: std::cell::Cell::new(0),
        });
        crate::gc_gate::gc_register_pub(&Value::List(l.clone()));
        l
    }

    #[allow(clippy::len_without_is_empty)] // is_empty would extend the public surface beyond the CPython-parity API
    pub fn len(&self) -> usize {
        self.items.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// 02.09b P7: was `Ref<'_, [Value]>` (RefCell); the Mutex version wraps
    /// the guard — `Deref<Target = [Value]>` keeps every read site source-
    /// compatible (`.iter()`, `.len()`, indexing, `.to_vec()`).
    pub fn items(&self) -> ItemsRef<'_> {
        ItemsRef(self.items.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// CPython's over-allocation formula (listobject.c list_resize):
    /// `new_allocated = (newsize + newsize>>3 + 6) & !3` — replicated for
    /// capacity discipline; `Vec` does the actual (re)allocation.
    fn reserve_for(&self, newsize: usize) {
        if newsize > self.allocated.get() {
            let new_allocated = (newsize + (newsize >> 3) + 6) & !3;
            self.items
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .reserve(new_allocated - newsize);
            self.allocated.set(new_allocated);
        }
    }

    pub fn append(&self, v: Value) {
        let n = self.items.lock().unwrap_or_else(|p| p.into_inner()).len() + 1;
        self.reserve_for(n);
        self.items.lock().unwrap_or_else(|p| p.into_inner()).push(v);
    }

    pub fn get(&self, i: usize) -> Option<Value> {
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(i)
            .cloned()
    }

    /// Overwrite `items[i]` (in-bounds assignment; caller validated).
    pub fn set_at(&self, i: usize, v: Value) {
        self.items.lock().unwrap_or_else(|p| p.into_inner())[i] = v;
    }

    /// Remove `items[i]` (`del xs[i]`; caller validated the index).
    pub fn remove_at(&self, i: usize) {
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(i);
    }

    /// Remove `items[lo..hi]` (`del xs[lo:hi]`; caller validated the range).
    pub fn remove_range(&self, lo: usize, hi: usize) {
        if lo < hi {
            self.items
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .drain(lo..hi);
        }
    }

    /// D2 (memperf audit 2026-11-06): in-place `xs.insert(i, v)` under one
    /// borrow (caller clamps the index). The old path copied the whole list
    /// out, inserted, cleared, and re-appended — O(n) clones per op,
    /// O(n²) for `insert(0, x)` loops.
    pub fn insert_at(&self, i: usize, v: Value) {
        let n = self.items.lock().unwrap_or_else(|p| p.into_inner()).len() + 1;
        self.reserve_for(n);
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(i, v);
    }

    /// D2: in-place `xs.remove(i)` returning the removed value
    /// (caller validated the index).
    pub fn remove_take(&self, i: usize) -> Value {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        b.remove(i)
    }

    /// D2: in-place `xs.reverse()` under one borrow.
    pub fn reverse_in_place(&self) {
        self.items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .reverse();
    }

    /// `xs.clear()`.
    ///
    /// C7 (memperf audit 2026-11-06): releases the backing buffer. The old
    /// body kept `items`' Vec capacity alive while setting `allocated` to 0
    /// — a lie (capacity != 0) that also pinned the whole peak buffer
    /// forever. CPython's `list_clear` frees the item array; match it.
    pub fn clear(&self) {
        *self.items.lock().unwrap_or_else(|p| p.into_inner()) = Vec::new();
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
        let items = self.items.lock().unwrap_or_else(|p| p.into_inner());
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
/// the Arc payload pointer is heap-stable and shared across clones, so the
/// recursion guard compares equal for the same object. When `Gc` becomes the
/// MMTk handle (02.04) this becomes the handle id.
pub fn addr_of<T>(r: &Gc<T>) -> usize {
    std::sync::Arc::as_ptr(r).addr()
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

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn value_size_is_16_bytes_doc_pinned() {
        // C10 (memperf audit 2026-11-06): the audit claimed Value >= 24
        // because the Int arm embeds a 16-byte enum. MEASURED: niche
        // packing keeps it 16 (Int's spare tag patterns carry the outer
        // discriminant). The doc says 16; THIS test makes the doc and the
        // layout agree permanently — a future arm that breaks the packing
        // fails here instead of silently invalidating the doc.
        assert_eq!(std::mem::size_of::<Int>(), 16);
        assert_eq!(std::mem::size_of::<Value>(), 16);
    }

    #[test]
    fn interned_str_handles_are_reused_and_content_equal() {
        // A4: interning returns content-equal handles (dict probes compare
        // by content — identity reuse is pure allocation optimization).
        let a = interned_str("<<intern_test_name>>");
        let b = interned_str("<<intern_test_name>>");
        assert!(std::sync::Arc::ptr_eq(&a, &b), "same handle for same name");
        assert_eq!(a.as_str(), b.as_str());
        let c = PyString::new("<<intern_test_name>>");
        assert_eq!(&*a, &*c, "interned == fresh (content)");
    }
}

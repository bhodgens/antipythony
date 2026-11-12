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
///
/// 02.09c — unboxed Smi storage (SPLIT-02.09 §02.09c; audit doc §4, census
/// verdict GO, Smi-only v1): the payload is a [`ListStorage`] enum. A list
/// whose every element is `Value::Int(Int::Smi(_))` stores a raw `Vec<i64>`
/// (8 B/elem, no per-element `Value` tag) — the tier-0 half of master.md's
/// compact-layout intent. `Value::Bool` is deliberately NOT unboxed (bool ⊂
/// int for arithmetic but `type()`/repr must keep saying `bool`), and
/// `Int::Big`/str/float/etc. never are.
///
/// **Write gate / demotion (one-way):** any insertion of a non-Smi value
/// (`append`/`set_at`/`insert_at`) demotes the storage Smi → Boxed inside
/// the same lock critical section. Demotion never happens on read: reads of
/// an unboxed list materialize a temporary boxed snapshot under the lock
/// (see [`ItemsRef`]) while the persistent storage stays raw i64.
///
/// **Post-no-GIL lock discipline (02.09b decision-2, inherited):** items
/// storage + growth bookkeeping synchronize on this ONE `Mutex` — the
/// variant adds no lock of its own; a demotion and a concurrent `items()`
/// read are serialized by the same critical section that boxed lists use,
/// so T120/T122 cross-thread mutation semantics are unchanged.
///
/// **GC fence invariant (audit §4, pinned by
/// `gcmod::unboxed_list_stays_registered_with_zero_internal_edges`):**
/// unboxed ⇒ the list holds NO `Value` refs ⇒ its internal-element edges
/// vanish from `count_internal_refs`. The list must therefore stay
/// REGISTERED with the collector for container identity (gc.is_tracked
/// parity, gc007) — registration happens in the constructors independent of
/// storage arm — and subtract-refs must still collect it when the last
/// external handle drops (sc == 1 > internal == 0 would be wrong only if an
/// edge were invisible that someone else counts; Smi elements have no
/// registry address, so no edge can be missed).
pub struct PyList {
    /// Interior-mutable through shared `Gc` handles: `ys = xs; ys.append(4)`
    /// mutates `xs` (S-01-19 aliasing).
    /// 02.09b P7 (decision 2, audit entry 26): Mutex — items + allocated
    /// move under the SAME lock so the capacity invariant cannot tear
    /// between a growing worker and a reading one (the GC's `items()`
    /// scan reads under the same lock; contention flips SCAN_CONTENDED).
    /// 02.09c: the guard payload is the [`ListStorage`] enum; the demote
    /// transition happens under this lock, so no reader can observe a
    /// half-migrated list.
    items: Mutex<ListStorage>,
    /// Capacity actually reserved (CPython `allocated`); kept so growth
    /// mirrors CPython's `newsize + newsize>>3 + 6` pattern, though the
    /// exact values are unobservable (S-01-31-adjacent).
    allocated: std::cell::Cell<usize>,
}

/// 02.09c: the list payload. `Smi` is the unboxed fast path — an exact
/// invariant `every element was Value::Int(Int::Smi(_))` holds by
/// construction (the write gate maintains it); `Boxed` is the general
/// `Vec<Value>` form. The transition is ONE-WAY Smi → Boxed (never
/// re-unbox: proving re-unboxability after arbitrary writes would cost the
/// same scan the gate avoids).
enum ListStorage {
    Boxed(Vec<Value>),
    Smi(Vec<i64>),
}

impl ListStorage {
    fn len(&self) -> usize {
        match self {
            ListStorage::Boxed(v) => v.len(),
            ListStorage::Smi(v) => v.len(),
        }
    }

    /// Demote Smi → Boxed in place (no-op when already boxed). Caller holds
    /// the items lock; the rebuilt Vec preserves element order exactly.
    fn demote(&mut self) {
        if let ListStorage::Smi(s) = self {
            *self = ListStorage::Boxed(
                s.iter()
                    .map(|&n| Value::Int(crate::int::Int::Smi(n)))
                    .collect(),
            );
        }
    }

    /// 02.09c write gate: prepare the storage for inserting `v` and return
    /// the boxed vec to push into. A Smi element into Smi storage pushes
    /// into the raw array (caller handles that arm); anything else demotes
    /// first. Caller holds the items lock.
    fn write_gate(&mut self, v: &Value) {
        if !matches!(v, Value::Int(crate::int::Int::Smi(_))) {
            self.demote();
        }
    }

    /// Best-effort capacity pre-reserve (see `PyList::reserve_for`): sizes
    /// whichever arm is live. Caller holds the items lock.
    fn reserve_tail(&mut self, additional: usize) {
        match self {
            ListStorage::Boxed(v) => v.reserve(additional),
            ListStorage::Smi(v) => v.reserve(additional),
        }
    }
}

/// Classify a freshly built item vector for unboxed storage (02.09c):
/// `Some(raw)` iff EVERY element is an inline small int. Bools and bigints
/// force the boxed arm (see the PyList doc: type()/repr must stay exact).
fn smi_storage_of(items: &[Value]) -> Option<Vec<i64>> {
    let mut raw = Vec::with_capacity(items.len());
    for v in items {
        match v {
            Value::Int(crate::int::Int::Smi(n)) => raw.push(*n),
            _ => return None,
        }
    }
    Some(raw)
}

/// 02.09b P7: read guard for `PyList::items()` — Deref to `[Value]`.
///
/// 02.09c: boxed lists deref the locked storage directly (zero-copy, the
/// 02.09b behavior every one of the ~180 `.items()` call sites was written
/// against). Unboxed lists materialize a temporary boxed snapshot at
/// acquisition, UNDER the same lock guard — so the snapshot cannot race a
/// concurrent writer, the persistent raw-i64 storage stays untouched (no
/// read-demotion), and every call site stays source-identical (census §4
/// option (iii): the escape hatch lives in the guard, not at the sites).
pub struct ItemsRef<'a> {
    guard: std::sync::MutexGuard<'a, ListStorage>,
    materialized: Option<Vec<Value>>,
}

impl<'a> ItemsRef<'a> {
    fn new(guard: std::sync::MutexGuard<'a, ListStorage>) -> Self {
        let materialized = match &*guard {
            ListStorage::Boxed(_) => None,
            ListStorage::Smi(s) => Some(
                s.iter()
                    .map(|&n| Value::Int(crate::int::Int::Smi(n)))
                    .collect::<Vec<Value>>(),
            ),
        };
        ItemsRef {
            guard,
            materialized,
        }
    }
}

impl std::ops::Deref for ItemsRef<'_> {
    type Target = [Value];
    fn deref(&self) -> &[Value] {
        match &self.materialized {
            Some(v) => v,
            None => match &*self.guard {
                ListStorage::Boxed(v) => v,
                ListStorage::Smi(_) => {
                    unreachable!("unboxed storage always materializes in ItemsRef::new")
                }
            },
        }
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

impl Clone for ListStorage {
    fn clone(&self) -> Self {
        match self {
            ListStorage::Boxed(v) => ListStorage::Boxed(v.clone()),
            // i64 copies — the unboxed clone is a memcpy, not a per-element
            // Value clone (part of the 02.09c win).
            ListStorage::Smi(v) => ListStorage::Smi(v.clone()),
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
            items: Mutex::new(ListStorage::Boxed(Vec::new())),
            allocated: std::cell::Cell::new(0),
        });
        crate::gc_gate::gc_register_pub(&Value::List(l.clone()));
        l
    }

    pub fn from_vec(items: Vec<Value>) -> Gc<PyList> {
        // 02.09c: an all-Smi constructor input takes the unboxed arm (the
        // GC registration below is identical — see the struct doc for the
        // fence invariant; the elements are not Values, so they contribute
        // no internal edges either way).
        let storage = match smi_storage_of(&items) {
            Some(raw) => ListStorage::Smi(raw),
            None => ListStorage::Boxed(items),
        };
        let l = Gc::new(PyList {
            items: Mutex::new(storage),
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
    /// 02.09c: an unboxed list materializes a boxed snapshot under the lock
    /// (see [`ItemsRef`]) — reads never demote the raw storage.
    pub fn items(&self) -> ItemsRef<'_> {
        ItemsRef::new(self.items.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// CPython's over-allocation formula (listobject.c list_resize):
    /// `new_allocated = (newsize + newsize>>3 + 6) & !3` — replicated for
    /// capacity discipline; `Vec` does the actual (re)allocation.
    ///
    /// 02.09c note: this pre-reserve is now best-effort — the demote fast
    /// path (`append`/`insert_at` under one lock) skips it entirely; a
    /// reserve through here takes the lock, sizes whichever arm is live,
    /// and drops the guard before the caller's second lock. Capacity values
    /// are unobservable (S-01-31-adjacent), so the extra lock hop is sound.
    fn reserve_for(&self, newsize: usize) {
        if newsize > self.allocated.get() {
            let new_allocated = (newsize + (newsize >> 3) + 6) & !3;
            self.items
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .reserve_tail(new_allocated - newsize);
            self.allocated.set(new_allocated);
        }
    }

    /// 02.09c write gate: append a Smi onto the unboxed array in place; any
    /// other value (or already-boxed storage) goes through the demote path.
    /// All in ONE lock critical section — the lock acquisition at the top
    /// covers the gate + push so a concurrent `items()` reader can never
    /// see the demotion mid-flight (02.09b decision-2 discipline).
    pub fn append(&self, v: Value) {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        if let (ListStorage::Smi(s), Value::Int(crate::int::Int::Smi(n))) = (&mut *b, &v) {
            s.push(*n);
            return;
        }
        b.write_gate(&v);
        match &mut *b {
            ListStorage::Boxed(vec) => vec.push(v),
            ListStorage::Smi(_) => unreachable!("write_gate demoted non-Smi insert"),
        }
    }

    pub fn get(&self, i: usize) -> Option<Value> {
        let b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        match &*b {
            ListStorage::Boxed(v) => v.get(i).cloned(),
            ListStorage::Smi(s) => s.get(i).map(|&n| Value::Int(crate::int::Int::Smi(n))),
        }
    }

    /// Overwrite `items[i]` (in-bounds assignment; caller validated).
    /// 02.09c write gate: a non-Smi assignment demotes the storage.
    pub fn set_at(&self, i: usize, v: Value) {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        if let (ListStorage::Smi(s), Value::Int(crate::int::Int::Smi(n))) = (&mut *b, &v) {
            s[i] = *n;
            return;
        }
        b.write_gate(&v);
        match &mut *b {
            ListStorage::Boxed(vec) => vec[i] = v,
            ListStorage::Smi(_) => unreachable!("write_gate demoted non-Smi write"),
        }
    }

    /// Remove `items[i]` (`del xs[i]`; caller validated the index).
    pub fn remove_at(&self, i: usize) {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        match &mut *b {
            ListStorage::Boxed(v) => {
                v.remove(i);
            }
            ListStorage::Smi(s) => {
                s.remove(i);
            }
        }
    }

    /// Remove `items[lo..hi]` (`del xs[lo:hi]`; caller validated the range).
    pub fn remove_range(&self, lo: usize, hi: usize) {
        if lo < hi {
            let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
            match &mut *b {
                ListStorage::Boxed(v) => {
                    v.drain(lo..hi);
                }
                ListStorage::Smi(s) => {
                    s.drain(lo..hi);
                }
            }
        }
    }

    /// D2 (memperf audit 2026-11-06): in-place `xs.insert(i, v)` under one
    /// borrow (caller clamps the index). The old path copied the whole list
    /// out, inserted, cleared, and re-appended — O(n) clones per op,
    /// O(n²) for `insert(0, x)` loops.
    /// 02.09c write gate: non-Smi inserts demote (same critical section).
    pub fn insert_at(&self, i: usize, v: Value) {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        let n = b.len() + 1;
        drop(b);
        self.reserve_for(n);
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        if let (ListStorage::Smi(s), Value::Int(crate::int::Int::Smi(m))) = (&mut *b, &v) {
            s.insert(i, *m);
            return;
        }
        b.write_gate(&v);
        match &mut *b {
            ListStorage::Boxed(vec) => vec.insert(i, v),
            ListStorage::Smi(_) => unreachable!("write_gate demoted non-Smi insert"),
        }
    }

    /// D2: in-place `xs.remove(i)` returning the removed value
    /// (caller validated the index).
    pub fn remove_take(&self, i: usize) -> Value {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        match &mut *b {
            ListStorage::Boxed(v) => v.remove(i),
            ListStorage::Smi(s) => Value::Int(crate::int::Int::Smi(s.remove(i))),
        }
    }

    /// D2: in-place `xs.reverse()` under one borrow.
    pub fn reverse_in_place(&self) {
        let mut b = self.items.lock().unwrap_or_else(|p| p.into_inner());
        match &mut *b {
            ListStorage::Boxed(v) => v.reverse(),
            ListStorage::Smi(s) => s.reverse(),
        }
    }

    /// `xs.clear()`.
    ///
    /// C7 (memperf audit 2026-11-06): releases the backing buffer. The old
    /// body kept `items`' Vec capacity alive while setting `allocated` to 0
    /// — a lie (capacity != 0) that also pinned the whole peak buffer
    /// forever. CPython's `list_clear` frees the item array; match it.
    pub fn clear(&self) {
        *self.items.lock().unwrap_or_else(|p| p.into_inner()) = ListStorage::Boxed(Vec::new());
        self.allocated.set(0);
    }

    /// Shared `repr` recursion helper: format with CPython punctuation
    /// (`[a, b]`), cycle-protecting self-references with `[...]` (S-01-44).
    /// 02.09c: an unboxed list cannot self-reference (raw i64s hold no
    /// handles), but the `active` protocol is kept — the list is still
    /// registered for container identity and the repr path is shared with
    /// boxed lists that CAN cycle.
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
        let items = self.items();
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
    use crate::int::{SMI_MAX, SMI_MIN};

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

    // ------------------------------------------------------------- 02.09c
    // Unboxed Smi storage rider (SPLIT-02.09 §02.09c, audit doc §4). These
    // pins live at the value layer (no collector hook installed in unit
    // context, so registration is inert here — the GC-facing fence lives in
    // ap-gc). Every assertion is a byte-exact-surface proxy: the boxed and
    // unboxed arms must be observationally identical. Value has no
    // PartialEq (equality is the Python surface, crate::ops::eq), so the
    // value-level asserts go through it.

    /// Storage-arm selection: an all-Smi from_vec goes unboxed; bools (not
    /// Int::Smi in the Value model), bigints, and any other type force the
    /// boxed arm. Not directly spec-pinned (internal representation) — this
    /// pins the invariant the write gate and demote rely on.
    fn storage_is_smi(l: &PyList) -> bool {
        matches!(
            &*l.items.lock().unwrap_or_else(|p| p.into_inner()),
            ListStorage::Smi(_)
        )
    }

    fn veq(a: &Value, b: &Value) -> bool {
        crate::ops::eq(a, b)
    }

    #[test]
    fn smi_storage_arm_selection_and_demote_triggers() {
        let smi = PyList::from_vec(vec![
            Value::Int(Int::Smi(1)),
            Value::Int(Int::Smi(-2)),
            Value::Int(Int::Smi(SMI_MAX)),
            Value::Int(Int::Smi(SMI_MIN)),
        ]);
        assert!(storage_is_smi(&smi), "all-Smi constructor => unboxed");

        // Boundary: 2**59 (just past the smi max) is a Big => boxed.
        let big = PyList::from_vec(vec![
            Value::Int(Int::Smi(1)),
            Value::Int(crate::int::Int::from_i64(1 << 59)),
        ]);
        assert!(!storage_is_smi(&big), "bigint element => boxed");

        // Bool is NOT unboxed (type()/repr must keep saying bool).
        let bools = PyList::from_vec(vec![Value::Int(Int::Smi(1)), Value::Bool(true)]);
        assert!(!storage_is_smi(&bools), "bool element => boxed");

        // Empty lists stay boxed (an empty raw array saves nothing and the
        // first append hits the gate either way).
        assert!(!storage_is_smi(&PyList::new()));

        // append: Smi keeps unboxed; the first non-Smi demotes in place.
        let l = PyList::from_vec(vec![Value::Int(Int::Smi(1))]);
        l.append(Value::Int(Int::Smi(2)));
        assert!(storage_is_smi(&l));
        l.append(Value::Str(PyString::new("x")));
        assert!(!storage_is_smi(&l), "non-Smi append must demote");

        // set_at: same gate.
        let l = PyList::from_vec(vec![Value::Int(Int::Smi(1)), Value::Int(Int::Smi(2))]);
        l.set_at(1, Value::Int(Int::Smi(9)));
        assert!(storage_is_smi(&l));
        l.set_at(0, Value::None);
        assert!(!storage_is_smi(&l), "non-Smi set_at must demote");

        // insert_at: same gate.
        let l = PyList::from_vec(vec![Value::Int(Int::Smi(1))]);
        l.insert_at(0, Value::Int(Int::Smi(0)));
        assert!(storage_is_smi(&l));
        l.insert_at(1, Value::Float(2.5));
        assert!(!storage_is_smi(&l), "non-Smi insert_at must demote");
    }

    #[test]
    fn unboxed_observably_identical_to_boxed() {
        // The fence in miniature: build the same logical list both ways and
        // require identical reads through every accessor.
        let unboxed = PyList::from_vec(vec![
            Value::Int(Int::Smi(3)),
            Value::Int(Int::Smi(1)),
            Value::Int(Int::Smi(2)),
        ]);
        let boxed = PyList::from_vec(vec![
            Value::Str(PyString::new("3")),
            Value::Int(Int::Smi(1)),
            Value::Str(PyString::new("2")),
        ]);
        boxed.set_at(0, Value::Int(Int::Smi(3)));
        boxed.set_at(2, Value::Int(Int::Smi(2)));
        // (boxed is now logically [3, 1, 2] but still boxed — mixed history)

        assert_eq!(unboxed.len(), boxed.len());
        for i in 0..3 {
            assert!(
                veq(&unboxed.get(i).unwrap(), &boxed.get(i).unwrap()),
                "get({i}) must not distinguish storage arms"
            );
        }
        let (u, b) = (unboxed.items(), boxed.items());
        assert_eq!(u.len(), b.len());
        for (x, y) in u.iter().zip(b.iter()) {
            assert!(veq(x, y), "items() snapshots must be identical");
        }
        drop((u, b));

        // repr path (fmt_repr) — byte-exact surface.
        let mut ru = String::new();
        let mut rb = String::new();
        let mut active = std::collections::HashSet::new();
        let ua = unboxed.clone();
        let ba = boxed.clone();
        unboxed.fmt_repr(&mut ru, &mut active, addr_of(&ua));
        boxed.fmt_repr(&mut rb, &mut active, addr_of(&ba));
        assert_eq!(ru, "[3, 1, 2]");
        assert_eq!(ru, rb);

        // remove paths.
        assert!(
            veq(&unboxed.remove_take(1), &Value::Int(Int::Smi(1)),),
            "remove_take on unboxed returns the exact Smi"
        );
        assert_eq!(unboxed.len(), 2);
        unboxed.remove_at(0);
        unboxed.remove_range(0, 1);
        assert_eq!(unboxed.len(), 0);
        unboxed.reverse_in_place(); // empty, no-op — exercises the arm
        unboxed.clear();
        assert_eq!(unboxed.len(), 0);

        // Clone of an unboxed list is deep (i64 copy) and independent.
        // NOTE: `Gc<PyList>` is `Arc<PyList>` — `orig.clone()` would clone
        // the HANDLE (aliasing, S-01-19); the DEEP clone needs the inner
        // PyList's Clone impl explicitly.
        let orig = PyList::from_vec(vec![Value::Int(Int::Smi(7))]);
        let copy = PyList::from_vec(vec![Value::Int(Int::Smi(7))]);
        let cloned = (*orig).clone();
        cloned.append(Value::Str(PyString::new("demote-clone-only")));
        assert!(storage_is_smi(&orig), "clone mutation must not leak back");
        assert_eq!(
            cloned.len(),
            2,
            "the deep clone took the demote, orig did not"
        );
        assert!(
            !storage_is_smi(&cloned),
            "clone took the demote; orig stayed Smi"
        );
        assert!(veq(&orig.get(0).unwrap(), &copy.get(0).unwrap()));
    }

    #[test]
    fn demoted_values_are_exact_smis_not_normalized_bools() {
        // Demotion rebuilds Value::Int(Int::Smi(n)) from raw i64 — the
        // round trip must be exact for both smi boundary values.
        let l = PyList::from_vec(vec![
            Value::Int(Int::Smi(SMI_MIN)),
            Value::Int(Int::Smi(SMI_MAX)),
        ]);
        l.append(Value::Float(0.0)); // demote
        assert!(veq(&l.get(0).unwrap(), &Value::Int(Int::Smi(SMI_MIN))));
        assert!(veq(&l.get(1).unwrap(), &Value::Int(Int::Smi(SMI_MAX))));
        let items = l.items();
        assert!(matches!(items[0], Value::Int(Int::Smi(v)) if v == SMI_MIN));
        assert!(matches!(items[1], Value::Int(Int::Smi(v)) if v == SMI_MAX));
    }
}

//! # ap-obj — the antipythany Python object model (leaf 02.01)
//!
//! The core value layer: a tagged `Value`, Python-exact integers (small-int
//! fast path + arbitrary precision), IEEE-754 floats with CPython's shortest
//! round-trip repr, CPython-style compact strings, bytes, the insertion-ordered
//! containers (`list`, `dict` compact layout, `tuple`), hash-based `set`, and
//! the hashing/equality/repr/truthiness protocol surface (spec 01, `S-01-*`;
//! compat contract `C-12..C-18` for numbers, `C-26` for repr ordering).
//!
//! ## `Value` representation choice: enum-of-8, not NaN-boxing
//!
//! The leaf brief allowed tagged-pointer or NaN-boxing. Both were evaluated:
//!
//! - **Chosen: a plain `enum` with an inline `i64` smi, inline `f64`, and
//!   inline `bool`/`None`/`unit` variants; heap objects (str/bytes/list/dict/
//!   tuple/set/bigint) sit behind `Gc<T>`, a `Box` placeholder for the MMTk
//!   handle that arrives in 02.04.** The enum is 16 bytes (i64 payload +
//!   discriminant padding), `Copy`-cheap for scalars, and needs no unsafe:
//!   every pointer-shaped invariant MMTk will require (never holding a raw
//!   pointer across a safepoint) is already enforced by the type system
//!   because heap values are only ever reached through `Gc<T>`.
//! - **Rejected: NaN-boxing.** It packs doubles into the pointer word and
//!   buys 8 bytes per value, but every float op pays a mask/unmask round trip,
//!   it is `unsafe` end-to-end, and MMTk handles are *not* 48-bit biased
//!   pointers — boxing them needs an indirection table that erases the win.
//!   pon (tier-1 study, `docs/pon-study-tier1.md`) compiles typed `IntI64`
//!   regions regardless of box width, so tier-1 does not care either.
//! - Tagged pointers were rejected for the same GC-handle reason: with
//!   movable objects the tag bits cannot live in the pointer.
//!
//! ## Small ints (the smi fast path)
//!
//! `Smi` is **i61**: values in `[-2^60, 2^60)`. Arithmetic that leaves that
//! range promotes transparently to the heap `BigInt` — the boundary is
//! invisible (C-13: no wrapped/saturated result may ever escape; the corpus
//! sweeps `2^B ± k` around the documented width `B = 60`). The width is a
//! *documented conformance constant* (`SMI_BITS`), read by test agents from
//! this doc, not from code.
//!
//! Integers that fit i64 but not i61 still take the heap path; `BigInt` holds
//! `i64`-shaped values losslessly, so `hash(int)` is exact for them.
//!
//! ## Strings: CPython's storage trichotomy, simplified
//!
//! `Str` keeps a flag-efficient split: **CompactAscii** (bytes == code points,
//! `len`/indexing are O(1) byte ops) vs **Unicode** (Rust `String`, UTF-8;
//! `len` counts code points, caching a `char_count`). CPython adds UCS2/UCS4
//! widths to make *indexing* O(1) for wide strings; UTF-8 indexing is O(n)
//! here — observable behavior (len, indexing, slicing, repr per S-01-14/15)
//! is identical, and if a conformance workload ever shows up where O(n)
//! indexing hurts, the Unicode variant upgrades to a UCS2/UCS4 vec behind the
//! same API (02.08's `str` methods are the consumers).
//!
//! Hash note: `hash(str)` pins the siphash13 keys to all-zero (the
//! `PYTHONHASHSEED=0` layout) so runs are reproducible; `S-01-38` makes the
//! salt unobservable in conforming programs (sets are only ever repr'd sorted
//! per C-26), and `hash(bytes)` hashes the same bytes, matching CPython's
//! str/bytes hash agreement for equal ASCII content.
//!
//! ## Dict: CPython's compact layout
//!
//! `PyDict` = a sparse index table + a dense entries vector, two-thirds max
//! load (`USABLE_FRACTION`), perturb probing (`i = i*5 + perturb + 1`,
//! `PERTURB_SHIFT = 5`), growth to `used*3` rounded up to a power of two —
//! the 3.4+ compact-dict design. Deletion writes a `DKIX_DUMMY` tombstone in
//! the index table (entries stay put, so insertion order survives `del`,
//! S-01-21) and compacts on resize. Key collapse (`1`, `1.0`, `True` are one
//! key, S-01-23) falls out of hash+eq probing: the *first inserted key object*
//! stays in the entries vector, only the value is overwritten.
//!
//! ## Attribution
//!
//! Patterns adapted from [pydantic/monty](https://github.com/pydantic/monty)
//! (MIT): the enum-of-immediates + heap-ref `Value` shape (`value.rs`), the
//! cached-hash string payload idea (`types/str.rs`), and the tombstone-on-
//! delete entry discipline (`types/hash_table.rs`). Adapted, not copied —
//! this is a from-scratch implementation of the same *patterns*; monty's
//! arena/refcount heap is replaced by the `Gc<T>`/MMTk handle design this
//! crate documents for 02.04. Hash algorithms (siphash13 keys, int modulus,
//! float frexp loop, tuple xxHash, frozenset shuffle-bits) are transcriptions
//! of CPython 3.14.0's `Python/pyhash.c`, `Objects/tupleobject.c`,
//! `Objects/setobject.c` (PSF) because byte-exact hashes are normative
//! (S-01-36/37, and frozenset interning in S-01-26).
//!
//! ## Not in this leaf
//!
//! No interpreter (02.02), no user classes / `__eq__` dispatch hooks beyond
//! the extension points `Value::py_eq` documents (02.03), no GC (02.04 —
//! `Gc<T>` is a `Box` until then), no complex numbers (out of v1 corpus).

pub mod classes;
pub mod dict;
pub mod exc;
pub mod float_repr;
pub mod gc_gate;
pub mod hash;
pub mod int;
pub mod list_repr;
pub mod object;
pub mod ops;
pub mod str_repr;
pub mod value;

pub use dict::{PyDict, PySet};
pub use exc::PyException;
pub use float_repr::py_float_repr;
pub use int::{Gc, Int, SMI_BITS, SMI_MAX, SMI_MIN};
pub use list_repr::List;
pub use object::{ObjKind, PyObject};
pub use value::{PyString, Tuple, Value};

/// The module name user instances report in default reprs (V066:
/// `<__main__.Plain object at 0x...>` — a script run as `__main__`).
pub const MODULE_NAME: &str = "__main__";

/// Python `hash(None)`: the identity of the singleton is pinned per process in
/// CPython (pointer-derived). Ours is a fixed constant — `None` never crosses
/// a dict-key boundary in conforming corpus (it IS hashable, but nothing
/// pins the exact value; C-26 lists what is byte-anchored and `hash(None)`
/// is not in it). Documented divergence-free choice: we return 5740353901.
pub const NONE_HASH: i64 = 5740353901;

/// `hash(Ellipsis)` — identity-derived in CPython (pointer); UNSPECIFIED per
/// S-01-38/C-26 (never corpus-asserted). Fixed constant like NONE_HASH.
pub const ELLIPSIS_HASH: i64 = 5740353902;

//! Compact dict (CPython 3.4+ layout) and hash-based set.
//!
//! ## Dict (S-01-21..24)
//!
//! CPython's compact layout: a sparse **index table** of entry-vector slots
//! plus a dense **entries vector** in insertion order. Lookup hashes the key,
//! probes `i = hash & mask`, collisions follow `i = (i*5 + perturb + 1) &
//! mask` with `perturb >>= 5` — CPython's exact recurrence
//! (`dictobject.c do_lookup`, PERTURB_SHIFT = 5). Resize at 2/3 load
//! (`USABLE_FRACTION`) to `used*3` rounded up to a power of two
//! (`GROWTH_RATE`, CPython 3.14).
//!
//! - **Insertion order survives deletes** (S-01-21): deletion writes
//!   `DKIX_DUMMY` into the index slot; the entry stays in the (sparse)
//!   vector. A deleted-then-reinserted key appends (moves to the end) —
//!   probed live against 3.14.
//! - **Key collapse** (S-01-23): `1`, `1.0`, `True` hash equal and compare
//!   equal, so they probe to one entry; the FIRST key object is kept, only
//!   the value is overwritten (probed: `{1: 'a', 1.0: 'b', True: 'c'}` →
//!   `{1: 'c'}`).
//! - **Equality is order-independent** (S-01-22).
//! - Rehash on resize **compacts** dead entries (CPython also purges
//!   dummies at resize).
//!
//! ## Set (S-01-25/26)
//!
//! Hash-bucketed storage (hash → bucket → eq-resolved elements). Iteration
//! order is UNSPECIFIED and unobservable (C-26: corpus sorts or byte-diffs
//! only 0/1-element sets), so the internal order is free to be Rust's.
//! The empty/non-empty reprs and sorted-element support live here.

use crate::exc::PyException;
use crate::hash;
use crate::int::Gc;
use crate::value::Value;
use std::collections::HashMap;

/// Index-table sentinel: empty slot.
const DKIX_EMPTY: isize = -1;
/// Index-table sentinel: deleted (tombstone).
const DKIX_DUMMY: isize = -2;

/// A dict entry: first-inserted key object + current value.
#[derive(Debug, Clone)]
struct Entry {
    key: Value,
    value: Value,
}

/// Probe result: index-table position + entry-vector slot (when a live
/// entry matched). Keeping BOTH is what makes deletion O(1) — the caller
/// never re-probes.
struct Probe {
    table_pos: usize,
    slot: Option<usize>,
}

/// Insertion-ordered compact dict.
///
/// Interior-mutable through shared `Gc` handles (`d[k] = v` where the only
/// reference lives inside another container). The whole state is one
/// `RefCell<DictState>` so probes can't observe half-mutations; 02.07 picks
/// the concurrent wrapper.
#[derive(Debug, Clone, Default)]
pub struct PyDict {
    state: std::cell::RefCell<DictState>,
}

#[derive(Debug, Clone, Default)]
struct DictState {
    /// Dense-ish entries in first-insertion order; deletion leaves `None`
    /// holes (indices keep pointing past them until the next resize).
    entries: Vec<Option<Entry>>,
    /// Sparse index table: entry-vector positions or sentinels.
    indices: Vec<isize>,
    /// `indices.len() - 1`.
    mask: usize,
    /// Live entries.
    used: usize,
    /// Tombstoned entries (load accounting).
    dummies: usize,
}

impl PyDict {
    pub fn new() -> Gc<PyDict> {
        let d = Gc::new(PyDict {
            state: std::cell::RefCell::new(DictState {
                entries: Vec::new(),
                indices: vec![DKIX_EMPTY; 8],
                mask: 7,
                used: 0,
                dummies: 0,
            }),
        });
        crate::gc_gate::gc_register_pub(&Value::Dict(d.clone()));
        d
    }

    pub fn len(&self) -> usize {
        self.state.borrow().used
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn hash_of(key: &Value) -> Result<i64, PyException> {
        hash::py_hash(key)
    }

    /// Probe per CPython's recurrence. Returns the table position and, when
    /// a live entry compared equal, its entry-vector slot.
    fn probe(state: &DictState, key: &Value, h: u64) -> Probe {
        let mut i = (h as usize) & state.mask;
        let mut perturb = h as usize;
        loop {
            match state.indices[i] {
                DKIX_EMPTY => {
                    return Probe {
                        table_pos: i,
                        slot: None,
                    };
                }
                ix if ix >= 0 => {
                    let slot = ix as usize;
                    if let Some(Some(entry)) = state.entries.get(slot) {
                        if values_eq(&entry.key, key) {
                            return Probe {
                                table_pos: i,
                                slot: Some(slot),
                            };
                        }
                    }
                }
                _ => {} // DKIX_DUMMY: keep probing
            }
            perturb >>= 5;
            i = (i.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1)) & state.mask;
        }
    }

    /// Find or claim the slot for `key` (caller pre-hashed). Grow at 2/3
    /// effective load. Returns the entry-vector slot (new entries get
    /// `Value::None` as placeholder value, set by the caller immediately).
    fn find_or_claim(state: &mut DictState, key: Value, h: i64) -> usize {
        if (state.used + state.dummies + 1) * 3 > state.indices.len() * 2 {
            Self::resize(state);
        }
        let probe = Self::probe(state, &key, h as u64);
        match probe.slot {
            Some(slot) => slot,
            None => {
                let slot = state.entries.len();
                state.entries.push(Some(Entry {
                    key,
                    value: Value::None,
                }));
                state.indices[probe.table_pos] = slot as isize;
                state.used += 1;
                slot
            }
        }
    }

    /// Rebuild at `used*3` capacity (power of two), dropping dead entries.
    fn resize(state: &mut DictState) {
        let min_used = state.used * 3;
        let mut new_cap = 8usize;
        while new_cap <= min_used {
            new_cap <<= 1;
        }
        let old = std::mem::take(&mut state.entries);
        state.indices = vec![DKIX_EMPTY; new_cap];
        state.mask = new_cap - 1;
        state.dummies = 0;
        for entry in old.into_iter().flatten() {
            // Keys already stored were hashable at insert; builtins hash
            // purely, so rehash cannot fail (documented invariant).
            let h = Self::hash_of(&entry.key).expect("stored key rehash") as u64;
            let mut i = (h as usize) & state.mask;
            let mut perturb = h as usize;
            while state.indices[i] != DKIX_EMPTY {
                perturb >>= 5;
                i = (i.wrapping_mul(5).wrapping_add(perturb).wrapping_add(1)) & state.mask;
            }
            state.entries.push(Some(entry));
            state.indices[i] = (state.entries.len() - 1) as isize;
        }
    }

    /// `d[key] = value` (insert or overwrite). Overwrite keeps the FIRST
    /// key object and its insertion position (S-01-21/23).
    pub fn set(&self, key: Value, value: Value) -> Result<(), PyException> {
        let h = Self::hash_of(&key)?;
        // 02.07 GIL v1: cross-thread writers retry instead of panicking —
        // a sibling thread's read borrow clears within a scheduler quantum
        // (readers never block while holding the state borrow).
        let mut state = loop {
            match self.state.try_borrow_mut() {
                Ok(s) => break s,
                Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
            }
        };
        let slot = Self::find_or_claim(&mut state, key, h);
        if let Some(Some(entry)) = state.entries.get_mut(slot) {
            entry.value = value;
        }
        Ok(())
    }

    /// `d.get(key)` — `Some(Value)` (cloned handle) when present; unhashable
    /// keys are simply not present (CPython raises only at the dict-key SET
    /// surface, `d[[1]] = ...` / `d[[1]]` — those go through `set`/`hash_of`,
    /// and a raw `d.get([1])` is a TypeError in CPython too; the executor
    /// raises BEFORE calling get. Documented: get-on-unhashable returns None
    /// here, the raise belongs to the operation surface in `ops`).
    /// 02.07: contention-tolerant read — None while another thread holds the
    /// state borrow (caller retries; a MISSING key and a contended borrow are
    /// distinguishable only by the caller's retry policy, which is fine for
    /// the GIL v1 lookup path).
    pub fn try_get(&self, key: &Value) -> Option<Option<Value>> {
        let h = Self::hash_of(key).ok()? as u64;
        let state = self.state.try_borrow().ok()?;
        let probe = Self::probe(&state, key, h);
        Some(probe.slot.and_then(|slot| {
            state
                .entries
                .get(slot)
                .and_then(|e| e.as_ref())
                .map(|e| e.value.clone())
        }))
    }

    pub fn get(&self, key: &Value) -> Option<Value> {
        let h = Self::hash_of(key).ok()? as u64;
        let state = self.state.borrow();
        let probe = Self::probe(&state, key, h);
        probe.slot.and_then(|slot| {
            state
                .entries
                .get(slot)
                .and_then(|e| e.as_ref())
                .map(|e| e.value.clone())
        })
    }

    /// `key in d`.
    pub fn contains(&self, key: &Value) -> bool {
        self.get(key).is_some()
    }

    /// `d.pop(key)` — remove and return the value; `None` when absent.
    pub fn pop(&self, key: &Value) -> Option<Value> {
        let h = Self::hash_of(key).ok()? as u64;
        let mut state = self.state.borrow_mut();
        let probe = Self::probe(&state, key, h);
        let slot = probe.slot?;
        state.indices[probe.table_pos] = DKIX_DUMMY;
        let entry = state.entries[slot].take();
        state.used -= 1;
        state.dummies += 1;
        entry.map(|e| e.value)
    }

    /// `d.popitem()` — remove and return the LAST inserted (key, value)
    /// pair (CPython 3.7+ LIFO); None when empty.
    pub fn popitem(&self) -> Option<(Value, Value)> {
        let mut state = self.state.borrow_mut();
        let slot = state
            .entries
            .iter()
            .rposition(|e| e.is_some())?;
        let entry = state.entries[slot].take();
        // Find the index slot pointing at this entry and mark it dummy.
        if let Some(h) = entry.as_ref().map(|e| Self::hash_of(&e.key).ok()) {
            if let Some(hv) = h {
                for (i, ix) in state.indices.iter().enumerate() {
                    if *ix == slot as isize {
                        state.indices[i] = DKIX_DUMMY;
                        break;
                    }
                }
                let _ = hv;
            }
        }
        state.used -= 1;
        state.dummies += 1;
        entry.map(|e| (e.key, e.value))
    }

    /// Insertion-ordered snapshot of live entries (cloned handles — the
    /// borrow must end before callers can re-enter the dict, e.g. repr
    /// recursion through a value whose `__repr__` mutates this dict).
    pub fn items(&self) -> Vec<(Value, Value)> {
        self.state
            .borrow()
            .entries
            .iter()
            .filter_map(|e| e.as_ref().map(|e| (e.key.clone(), e.value.clone())))
            .collect()
    }

    pub fn keys(&self) -> Vec<Value> {
        self.items().into_iter().map(|(k, _)| k).collect()
    }

    /// Order-independent equality (S-01-22).
    pub fn py_eq(&self, other: &PyDict) -> bool {
        let items = self.items();
        items.len() == other.len()
            && items
                .iter()
                .all(|(k, v)| other.get(k).map(|ov| values_eq(v, &ov)).unwrap_or(false))
    }

    /// Recursive repr with CPython punctuation (S-01-44), cycle-protecting
    /// with `{...}` (V065: ys["self"] = ys).
    pub(crate) fn fmt_repr(&self, out: &mut String, active: &mut std::collections::HashSet<usize>) {
        // RefCell-state address identity (stable for the dict's life).
        let ptr = std::rc::Rc::as_ptr(&std::rc::Rc::new(0u8)) as usize; // placeholder
        let _ = ptr;
        let ptr = self as *const PyDict as *const u8 as usize;
        if !active.insert(ptr) {
            out.push_str("{...}");
            return;
        }
        out.push('{');
        for (i, (k, v)) in self.items().into_iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            k.fmt_repr(out, active);
            out.push_str(": ");
            v.fmt_repr(out, active);
        }
        out.push('}');
        active.remove(&ptr);
    }
}

/// Python `==` for values (structural, S-01-30/33) — the 02.03 protocol
/// hook point: user `__eq__` dispatch replaces this only for user types;
/// builtin arms stay exactly this.
pub fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::None, Value::None) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        // bool/int/float cross-type numeric equality (S-01-33)
        (Value::Bool(_) | Value::Int(_), Value::Bool(_) | Value::Int(_)) => {
            match (a.as_int(), b.as_int()) {
                (Some(x), Some(y)) => x.eq_int(&y),
                _ => false,
            }
        }
        (Value::Float(x), Value::Float(y)) => x == y, // NaN != NaN falls out
        (Value::Float(_), Value::Bool(_) | Value::Int(_))
        | (Value::Bool(_) | Value::Int(_), Value::Float(_)) => {
            // Normalize both to (Option<exact-int>, f64) and compare exactly.
            let (i, f) = match (a, b) {
                (Value::Float(f), other) | (other, Value::Float(f)) => (other.as_int(), *f),
                _ => unreachable!(),
            };
            match i {
                Some(i) => cmp_int_f64(&i, f) == std::cmp::Ordering::Equal,
                None => false,
            }
        }
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bytes(x), Value::Bytes(y)) => x == y,
        // A-4 item 1: bytearray == bytes (CPython buffer equality; bytearray
        // is an Opaque object with its buffer in the "<data>" attr).
        (Value::Bytes(x), Value::Obj(o)) | (Value::Obj(o), Value::Bytes(x)) => {
            match bytearray_data(o) {
                Some(d) => &**x == &*d,
                None => false,
            }
        }
        (Value::List(x), Value::List(y)) => {
            let (x, y) = (&**x, &**y);
            x.len() == y.len()
                && x.items()
                    .iter()
                    .zip(y.items().iter())
                    .all(|(a, b)| values_eq(a, b))
        }
        (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.0.iter().zip(y.0.iter()).all(|(a, b)| values_eq(a, b))
        }
        (Value::Dict(x), Value::Dict(y)) => x.py_eq(y),
        (Value::Set(x), Value::Set(y)) => x.set_eq(y),
        (Value::Frozen(x), Value::Frozen(y)) => x.set_eq(y),
        (Value::Ellipsis, Value::Ellipsis) => true,
        // Generic objects compare (and hash) by IDENTITY (default object
        // protocol); equality across different arms is always false.
        // Exception: weakref tokens compare BY TARGET (CPython: two refs to
        // the same object are ==; the WeakSet/WKD data model relies on it).
        (Value::Obj(x), Value::Obj(y)) => {
            // A-4 item 1: two bytearrays compare by buffer content.
            if let (Some(a), Some(b)) = (bytearray_data(x), bytearray_data(y)) {
                a == b
            } else if let (Some(a), Some(b)) = (
                crate::value::weakref_target_addr(x),
                crate::value::weakref_target_addr(y),
            ) {
                a == b
            } else {
                std::rc::Rc::ptr_eq(x, y)
            }
        }
        // Different types never compare equal (S-01-33), never an error.
        _ => false,
    }
}

impl crate::int::Int {
    /// Bigint view for exact comparisons (clone; comparisons are rare vs
    /// arithmetic in corpus, correctness-first).
    pub fn to_big_for_cmp(&self) -> num_bigint::BigInt {
        match self {
            crate::int::Int::Smi(v) => num_bigint::BigInt::from(*v),
            crate::int::Int::Big(b) => (**b).clone(),
        }
    }
}

/// A-4 item 1: the bytearray Opaque object's buffer (None for other Objs).
pub(crate) fn bytearray_data(o: &crate::object::PyObject) -> Option<Gc<Vec<u8>>> {
    if o.kind == crate::object::ObjKind::Opaque && o.name == "bytearray" {
        let ab = o.attrs.borrow();
        match ab.get(&Value::Str(crate::PyString::new("<data>"))) {
            Some(Value::Bytes(b)) => Some(b.clone()),
            _ => None,
        }
    } else {
        None
    }
}

/// Exact comparison of an arbitrary-precision int against an f64
/// (S-01-33/34 exactness: `2**53 + 1 == 9007199254740992.0` is FALSE).
/// The float is decomposed into exact `mant * 2^exp` form and compared on
/// the bigint side — no double rounding anywhere.
pub fn cmp_int_f64(i: &crate::int::Int, f: f64) -> std::cmp::Ordering {
    use num_bigint::BigInt;
    use std::cmp::Ordering;
    if f.is_nan() {
        // Unordered; callers that need an Ordering (sorting) never see NaN
        // (NaN breaks total orders in CPython too). Consistent arbitrary.
        return Ordering::Greater;
    }
    if f.is_infinite() {
        return if f > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let (mant, exp) = decompose_f64_exact(f);
    if mant == 0 {
        return if i.is_zero() {
            Ordering::Equal
        } else if i.is_negative() {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let i_big = i.to_big_for_cmp();
    // Compare i vs mant * 2^exp without floating point: scale both to
    // integer domain. Negative exp → f is fractional; i*2^-exp vs mant.
    if exp >= 0 {
        i_big.cmp(&(BigInt::from(mant) << (exp as usize)))
    } else {
        let lhs = i_big << (-exp as usize);
        lhs.cmp(&BigInt::from(mant))
    }
}

// Small helper trait was folded into cmp_int_f64's body.

/// Split a finite float into exact `(mantissa_i64, exp2)` with
/// f == mant * 2^exp; `(0, 0)` for zero. Caller excludes non-finite.
fn decompose_f64_exact(f: f64) -> (i64, i32) {
    if f == 0.0 {
        return (0, 0);
    }
    let bits = f.to_bits();
    let sign = if bits >> 63 == 1 { -1i64 } else { 1 };
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    if raw_exp == 0 {
        // subnormal: f = ±frac * 2^-1074
        (sign * (frac as i64), -1074)
    } else {
        // f = ±(2^52 | frac) * 2^(raw_exp - 1075)
        (sign * ((frac | (1u64 << 52)) as i64), raw_exp - 1075)
    }
}

/// Python ordering `< >` for orderable pairs (S-01-34): numeric chains
/// (int/float/bool exact), str/bytes/list/tuple lexicographic. Mismatched
/// pairs return the exact TypeError text; the CALLER raises.
pub fn values_cmp(a: &Value, b: &Value) -> Result<std::cmp::Ordering, PyException> {
    let type_err = || {
        PyException::new(
            "TypeError",
            format!(
                "'<' not supported between instances of '{}' and '{}'",
                a.type_name(),
                b.type_name()
            ),
        )
    };
    match (a, b) {
        (Value::Bool(_) | Value::Int(_), Value::Bool(_) | Value::Int(_)) => {
            match (a.as_int(), b.as_int()) {
                (Some(x), Some(y)) => Ok(x.cmp_int(&y)),
                _ => Err(type_err()),
            }
        }
        (Value::Float(x), Value::Float(y)) => {
            x.partial_cmp(y).ok_or_else(|| type_err()) // NaN: not orderable
        }
        (Value::Int(_) | Value::Bool(_), Value::Float(_)) => Ok(cmp_int_f64(
            &a.as_int().unwrap(),
            match b {
                Value::Float(f) => *f,
                _ => unreachable!(),
            },
        )),
        (Value::Float(_), Value::Int(_) | Value::Bool(_)) => Ok(cmp_int_f64(
            &b.as_int().unwrap(),
            match a {
                Value::Float(f) => *f,
                _ => unreachable!(),
            },
        )
        .reverse()),
        (Value::Str(x), Value::Str(y)) => Ok(x.as_str().cmp(y.as_str())),
        (Value::Bytes(x), Value::Bytes(y)) => Ok((**x).cmp(&**y)),
        (Value::List(x), Value::List(y)) => {
            let (xi, yi) = (x.items(), y.items());
            seq_cmp(&xi, &yi)
        }
        (Value::Tuple(x), Value::Tuple(y)) => seq_cmp(&x.0, &y.0),
        // T1-d fix (sys.version_info chained compares): NamedTuple compares
        // as its <values> tuple against tuples/lists ((3,13,8)<=vi<(3,14) in
        // typing_extensions gates the deprecated class body).
        (Value::Obj(o), _) | (_, Value::Obj(o)) if o.kind == crate::object::ObjKind::NamedTuple => {
            let a_is_nt =
                matches!(a, Value::Obj(ao) if ao.kind == crate::object::ObjKind::NamedTuple);
            let vals = o
                .attrs
                .borrow()
                .get(&Value::Str(crate::PyString::new("<values>")));
            let tup = match vals {
                Some(Value::List(l)) => {
                    Value::Tuple(std::rc::Rc::new(crate::value::Tuple(l.items().to_vec())))
                }
                Some(other) => other.clone(),
                None => return Err(type_err()),
            };
            if a_is_nt {
                values_cmp(&tup, b)
            } else {
                values_cmp(a, &tup).map(|ord| ord.reverse())
            }
        }
        _ => Err(type_err()),
    }
}

fn seq_cmp(a: &[Value], b: &[Value]) -> Result<std::cmp::Ordering, PyException> {
    use std::cmp::Ordering;
    for (x, y) in a.iter().zip(b.iter()) {
        match values_cmp(x, y)? {
            Ordering::Equal => continue,
            non_eq => return Ok(non_eq),
        }
    }
    Ok(a.len().cmp(&b.len()))
}

/// Hash-based unordered set (S-01-25/26). Iteration order is UNSPECIFIED
/// (C-26) — reprs and tests sort; internally hash → bucket → eq resolves.
/// Interior-mutable through shared handles, like `PyList`.
#[derive(Debug, Clone, Default)]
pub struct PySet {
    state: std::cell::RefCell<SetState>,
}

#[derive(Debug, Clone, Default)]
struct SetState {
    elements: HashMap<u64, Vec<Value>>,
    size: usize,
}

impl PySet {
    pub fn new() -> Gc<PySet> {
        let s = Gc::new(PySet {
            state: std::cell::RefCell::new(SetState::default()),
        });
        crate::gc_gate::gc_register_pub(&Value::Set(s.clone()));
        s
    }

    pub fn len(&self) -> usize {
        self.state.borrow().size
    }

    /// `s.add(x)`; unhashable raises the SET-ELEMENT phrasing — probed
    /// live on 3.14.7: `s.add([2])` says "cannot use 'list' as a set
    /// element (unhashable type: 'list')" (V046; the older plain-text note
    /// was wrong).
    pub fn add(&self, v: Value) -> Result<(), PyException> {
        let h =
            hash::py_hash(&v).map_err(|_| PyException::unhashable_set_element(v.type_name()))?;
        let mut state = self.state.borrow_mut();
        let bucket = state.elements.entry(h as u64).or_default();
        if bucket.iter().any(|e| values_eq(e, &v)) {
            return Ok(());
        }
        bucket.push(v);
        state.size += 1;
        Ok(())
    }

    /// `x in s` (raises for unhashable x, like CPython).
    pub fn contains(&self, v: &Value) -> Result<bool, PyException> {
        let h = hash::py_hash(v)?;
        let state = self.state.borrow();
        Ok(state
            .elements
            .get(&(h as u64))
            .map(|bucket| bucket.iter().any(|e| values_eq(e, v)))
            .unwrap_or(false))
    }

    /// Order-independent equality (S-01-25).
    pub fn set_eq(&self, other: &PySet) -> bool {
        let elems = self.elements_snapshot();
        elems.len() == other.len() && elems.iter().all(|e| other.contains(e).unwrap_or(false))
    }

    fn elements_snapshot(&self) -> Vec<Value> {
        self.state
            .borrow()
            .elements
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Snapshot iteration (cloned handles; same re-entrancy note as
    /// `PyDict::items`).
    pub fn iter(&self) -> std::vec::IntoIter<Value> {
        self.elements_snapshot().into_iter()
    }

    /// Sorted element view for corpus support (C-26: tests use `sorted`).
    /// Sort falls back to internal order for mutually unorderable elements
    /// (CPython's sort would raise; the corpus never does this).
    pub fn sorted_elements(&self) -> Vec<Value> {
        let mut v = self.elements_snapshot();
        v.sort_by(|a, b| values_cmp(a, b).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    /// `s.discard(x)` — remove when present, never raises.
    pub fn pop_discard(&self, v: &Value) -> bool {
        let h = match hash::py_hash(v) {
            Ok(h) => h as u64,
            Err(_) => return false,
        };
        let mut state = self.state.borrow_mut();
        if let Some(bucket) = state.elements.get_mut(&h) {
            if let Some(pos) = bucket.iter().position(|e| values_eq(e, v)) {
                bucket.remove(pos);
                state.size -= 1;
                return true;
            }
        }
        false
    }

    /// Sorted element view exposed for the frozenset repr (C-26 masking).
    pub fn sorted_elements_public(&self) -> Vec<Value> {
        self.sorted_elements()
    }

    /// Repr (S-01-44): `set()` / `{e1, e2}` in INTERNAL order — unobservable
    /// per C-26.
    pub(crate) fn fmt_repr(&self, out: &mut String, active: &mut std::collections::HashSet<usize>) {
        let elems = self.elements_snapshot();
        if elems.is_empty() {
            out.push_str("set()");
            return;
        }
        out.push('{');
        for (i, e) in elems.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            e.fmt_repr(out, active);
        }
        out.push('}');
    }

    /// Element hashes for `hash::hash_frozenset_lanes` (S-01-26's
    /// `hash(frozenset([1,2])) == hash(frozenset([2,1]))` requirement).
    pub fn element_hashes(&self) -> Result<Vec<i64>, PyException> {
        self.elements_snapshot().iter().map(hash::py_hash).collect()
    }
}

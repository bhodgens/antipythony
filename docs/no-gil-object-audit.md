# no-GIL object-state audit (leaf 02.09, phase 1)

Status: IN PROGRESS (phase 1 of the 02.09 split at orchestrator.md "### 02.09").
Branch: `no-gil-0209`. GIL v1 (crates/ap-run/src/threading.rs) is RETAINED in
this phase; this document is the map phase 2 uses to remove it.

## Methodology

Every interior-mutable surface in `crates/ap-obj` was enumerated by grepping
`RefCell<` / `Cell<` / `borrow_mut` / `RefMut` across `crates/ap-obj/src/`
(50 raw hits across 4 files) and cross-checked against the ap-run consumers
that mutate the same handles (threading.rs, gcsupport.rs, classes.rs callers,
dict/list method surfaces).

Classification per the leaf brief:
- **(a) thread-confined** — mutation provably happens on a single OS thread
  for the object's whole lifetime (no marker or wrapper needed once the
  refcount problem is solved).
- **(b) needs atomic/CAS** — single-word state where a relaxed atomic is
  behavior-identical to the current GIL-serialized access.
- **(c) needs a real lock** — multi-word or algorithmic state; a RefCell
  cannot be shared across threads (borrow flags are thread-local).
- **(d) needs epoch/version counters** — 02.05b lands these; phase 1 marks
  the site. Where the counter ALREADY exists as a `Cell<u64>` and relaxed
  atomics are behavior-neutral under GIL v1, phase 1 converts it.

## The blocker every surface shares (read this first)

`Gc<T> = std::rc::Rc<T>` (crates/ap-obj/src/int.rs:76). **No `unsafe impl
Send/Sync` is added to any ap-obj object type in phase 1, because none is
provable**: under GIL v1 every Python value can legally cross OS threads
(`queue.Queue.put` carries `Value` clones into a worker's graph — T101-T115
exercise exactly this), so no object is single-thread-owned, and `Rc` strong
counts are non-atomic (a cross-thread clone/drop is UB regardless of the
interior-mutability story). Send/Sync markers for these types become provable
only when the refcount story changes (atomic refcounts / Arc / per-thread
heaps with a cross-thread bridge — the 02.05b/02.05g land). The audit
therefore documents the per-type discipline (02.07's started list) and
classifies the INTERIOR state, which is the part phase 2 must wrap.

The existing audited unsafety wrapper is `crate::gcsupport::GilSendPub<T>`
(ap-run/src/gcsupport.rs:1773 — `unsafe impl Send/Sync for GilSendPub<T>`,
contract: all access GIL-serialized). threading.rs's global tables
(THREADS/LOCALS/PRIMS/OBJ_MIRROR/CANCELLED/WORKER_RECLAIM/WORKER_INTERP/
EXCEPTHOOK) all hold their Rc payloads behind it. Those wrappers are the
phase-2 seam: with the GIL gone each becomes a real lock or an ownership
transfer.

## Audit table

Key: fields marked **CONVERTED** were changed in phase 1 (behavior-neutral
under GIL v1); **(d)** sites are what 02.05b extends; **GC-pair** sites are
02.05g's (safepoint protocol — out of phase-1 scope per the brief).

### 1. `crates/ap-obj/src/object.rs` — `PyObject` (18 fields, one struct)

Shared via `Gc<PyObject>` everywhere; instances cross threads freely (queue,
threading.Thread target args). Every field below is (c) at minimum: the
RefCell borrow flag itself is thread-local, so ANY cross-thread access needs
either confinement proof (impossible per the blocker above) or a lock/hazard
scheme. Per-kind mutation analysis:

| # | Field | Mutators | Class | Notes |
|---|-------|----------|-------|-------|
| 1 | `self_value: RefCell<Option<Value>>` | `PyObject::with_self` (ctor, line 156); bound-method call sites (ap-run) | (a)-per-object* | set once at construction; afterwards read-only. *confinement claim still gated on refcount story |
| 2 | `func: RefCell<Option<Value>>` | ctors; descriptor `__get__` binding (ap-run/methods.rs) | (c) | method objects rebind per access — hot, allocation churn under a lock is the phase-2 perf risk |
| 3 | `ints: RefCell<Vec<i64>>` | Range/NamedTuple ctors only | (a)-per-object* | immutable after construction |
| 4 | `slice_parts: RefCell<Vec<Value>>` | slice ctor + `indices()` normalization | (c) | written post-construction by slice resolution |
| 5 | `attrs: RefCell<Gc<PyDict>>` | attribute get/set everywhere (hot path); threading.rs attr_str/attr_bool cross-thread probes | (c) | the OUTER RefCell is only swapped at class-assignment/init; the INNER PyDict has its own entry (table 3) |
| 6 | `args: RefCell<Vec<Value>>` | exception raise sites; `exception_instance_value` (threading.rs:1314) | (c) | exception args can be re-set (`exc.args = ...`) |
| 7 | `cause: RefCell<Option<Value>>` | `__cause__` assignment | (c) | |
| 8 | `context: RefCell<Option<Value>>` | implicit chaining at raise | (c) | |
| 9 | `dunders: RefCell<PyDict>` | class-body execution (set-once per class, then read) | (a)-per-object* | effectively immutable after class creation; `__dunder__` monkeypatching would move it to (c) |
| 10 | `gen_id: RefCell<Option<u64>>` | generator ctor + executor resume bookkeeping | (c) | per-generator, single-owner in practice (a generator driven from two threads is a user error CPython also forbids) — phase 2 may prove (a) with a driver-owner field |
| 11 | `repr_text: RefCell<Option<String>>` | ops.rs:791 repr memoization for Namespace/sys-objects | (b) | idempotent same-value cache; phase 2: `Mutex<Option<String>>` or once-lock semantics; phase 1 unchanged |
| 12 | `module_ns: RefCell<Option<Rc<PyDict>>>` | importer module init (set-once) | (a)-per-object* | module dict installed once at import; reads only afterwards |
| 13 | `closure_slot: RefCell<Option<u64>>` | function ctor | (a)-per-object* | set-once |
| 14 | `defaults: RefCell<Vec<Value>>` | function ctor; `__defaults__` assignment | (c) | mutable via user code |
| 15 | `class_ref: RefCell<Option<Rc<TypeObject>>>` | class-instance binding (classes.rs:248, set-once) + `__class__` assignment | (c) | `__class__` reassignment is legal Python |
| 16 | `type_ref: RefCell<Option<Weak<TypeObject>>>` | instance-of binding (set-once) | (a)-per-object* | |
| 17 | `descr_kind: RefCell<Option<String>>` | descriptor ctor | (a)-per-object* | set-once |
| 18 | `method_owner: RefCell<Option<Weak<TypeObject>>>` | bound-method ctor | (a)-per-object* | set-once |

\* "(a)-per-object": mutation confined to a construction window, but the
OBJECT is still shareable — the honest phase-2 disposition is per-object
phase state (immutable-after-init proof or a small init mutex), not a
process-wide lock. Refcount story (blocker above) gates all of them.

### 2. `crates/ap-obj/src/classes.rs` — `TypeObject` (3 fields)

| # | Field | Mutators | Class | Notes |
|---|-------|----------|-------|-------|
| 19 | `mro: RefCell<Vec<Rc<TypeRef>>>` | class construction (classes.rs:102); MRO patching | (c) | classes are process-global (interned_type cache); any thread may read |
| 20 | `metaclass: RefCell<Option<Rc<TypeRef>>>` | class construction | (a)-per-object* | set-once at `class` statement |
| 21 | `class_value: RefCell<Option<Value>>` | classes.rs:250 (set-once backref) | (a)-per-object* | |

Reads dominate (attribute lookup walks `mro` per access) — phase 2 wants
`ArcSwap`-style read-mostly handling or RCU on `mro`, not a per-access mutex.

### 3. `crates/ap-obj/src/dict.rs` — `PyDict` (2 fields) + `PySet` (1 field)

| # | Field | Mutators | Class | Notes |
|---|-------|----------|-------|-------|
| 22 | `PyDict.state: RefCell<DictState>` | `set`/`pop`/`popitem`/`clear`/resize (dict.rs:341,369) + all reads | (c) | multi-word (entries+indices+mask+used+dummies) — CAS impossible, needs per-dict lock or epochs+retry in phase 2. Algorithmic internals UNCHANGED per phase-1 brief |
| 23 | `PyDict.epoch: Cell<u64>` | bumped by every `set`/`pop`/`popitem`; read by ap-gc ROOT_SEEDS cache (`mutation_epoch()`, gcmod.rs:370) | **(d) CONVERTED** | pure mutation counter; `Cell<u64>` -> `AtomicU64` Relaxed. Under GIL v1 behavior-identical (accesses were serialized; Relaxed == the ordering the lock provided). Gives 02.05b a real atomic epoch to build version counters on |
| 24 | `PySet.state: RefCell<SetState>` | set add/discard (dict.rs:771,844) | (c) | same shape as PyDict.state |

### 4. `crates/ap-obj/src/value.rs` — `PyList`, `PyString`, intern table

| # | Field | Mutators | Class | Notes |
|---|-------|----------|-------|-------|
| 25 | `PyList.items: RefCell<Vec<Value>>` | append/setitem/del/insert/reverse/sort (value.rs:401-462) | (c) | multi-word Vec + S-01-19 aliasing invariant (`xs` may alias into itself during mutation — the RefCell guards mid-mutation observation). Per the brief: algorithmic internals UNCHANGED. Phase 2: per-list lock |
| 26 | `PyList.allocated: Cell<usize>` | same sites as `items` (capacity hint) | (c) | co-mutated with `items` — must move under the SAME lock as items in phase 2; NOT convertible alone (divorcing it would let phase 2 readers see inconsistent capacity, currently unobservable) |
| 27 | `PyString.cached_len: Cell<Option<usize>>` | `len()` lazy fill (value.rs:228-233) | **(b) CONVERTED** | idempotent same-value cache; `Cell<Option<usize>>` -> `AtomicUsize` with `usize::MAX` = None sentinel (a `usize::MAX`-length string is unallocatable). Relaxed. Under GIL v1: identical bytes — the winner of a same-value race writes the same bits |
| 28 | `PyString.cached_hash: Cell<Option<i64>>` | `hash()` lazy fill | (b) DEFERRED | convertible in principle (None sentinel needed), but `i64::MIN`-style sentinels require proving hash never produces the sentinel value (CPython maps hash -1 -> -2; our zero-keyed siphash13 does NOT document that mapping — verify before converting). Phase 2 with a hash-domain audit |
| 29 | `INTERNED_NAMES: thread_local RefCell<HashMap<String, Gc<PyString>>>` (value.rs:27) | `interned_str_ref` | **(a) already thread-confined** | thread_local since A4 (memperf 2026-11-06); each thread interns its own map. No conversion; phase 2 note: cross-thread identity of interned handles relies on the maps agreeing, which holds because keys are process-static `&'static str` |
| 30 | `PyTuple` (value.rs) | none | (a) | immutable tuple — no interior mutability at all; first candidate for `unsafe impl Sync` ONCE the refcount story lands |

### 5. Consumers in `crates/ap-run` that mutate ap-obj interiors cross-thread
(GIL protocol already documented; listed for phase-2 completeness)

| Site | Surface | Class |
|------|---------|-------|
| threading.rs `attr_str`/`attr_bool` (1325-1351) | `PyObject.attrs` borrow FROM A WORKER while another thread's native span holds it | existing bounded-retry; phase 2: real per-attr lock or hazard pointers |
| threading.rs `exception_instance_value` (1312-1316) | `PyObject.args` borrow_mut | under GIL, fine |
| threading.rs `thread_obj` ctor + `make_thread_value` (658-685, 1823-1856) | `attrs` borrow_mut at construction | set-once window |
| gcsupport worker registration channel | Value clones cross threads via Mutex<Vec> under GilSendPub | the audited seam |
| ops.rs:784,791 | `attrs`/`repr_text` memoization | (b)/(c) per above |

### 6. GC pairing notes (docs/gc-mmtk-swap.md; 02.05g — OUT OF SCOPE)

- ap-gc's REGISTRY/FROZEN/WEAKREFS/CALLBACKS/CALLBACK_EPOCHS are
  **thread-locals of the main thread** (gcmod.rs:44-69) with a
  forward-registration channel for workers (gcsupport::install_worker_thread)
  — already the (a) thread-confined shape, but rooted in "one collector
  thread's view", not per-mutator heaps. The safepoint protocol that keeps
  this correct without the GIL is 02.05g's deliverable.
- `PyDict.epoch` (entry 23) is the GC's invalidation key for the ROOT_SEEDS
  cache — after the phase-1 AtomicU64 conversion the cache read becomes
  `load(Relaxed)`, identical under the GIL; under no-GIL it additionally
  needs the collector to treat epochs as hints (stale cache entry = re-walk),
  which the cache already tolerates by design (gcmod.rs:345-359 invariant
  prose).

## Phase-1 conversions landed

1. `PyDict.epoch`: `Cell<u64>` -> `AtomicU64` (Relaxed store/load). Entry 23.
2. `PyString.cached_len`: `Cell<Option<usize>>` -> `AtomicUsize`
   (`usize::MAX` sentinel = None; Relaxed). Entry 27.

Behavior-neutrality argument (both): under GIL v1 all accessors were
mutually excluded; the new relaxed atomics implement the same happens-before
the lock did, single-word values with idempotent or monotone writes, and no
reader branches on anything but the loaded word. Debug assertions/tests
unchanged.

## Send/Sync markers added

None — see "The blocker every surface shares". `Gc<T> = Rc<T>` keeps every
ap-obj object type !Send/!Sync; the audited `GilSendPub` wrappers at the
ap-run tables remain the only Send/Sync crossings, unchanged. Phase 2+ can
only mark types after the refcount story changes; the per-type discipline
they must carry is the table above.

## Deferred (phase 2+ handoff)

- (c) class: per-object lock strategy for PyObject.attrs / PyDict.state /
  PyList.items(+allocated) / PySet.state / TypeObject.mro — decide lock vs
  epochs+retry against real T121 contention, do NOT pre-commit.
- (b) `PyString.cached_hash` atomic conversion pending a hash-domain audit
  (sentinel collision proof; CPython maps hash -1 -> -2 — check parity).
- (a)-per-object set-once fields: init-phase state machine (immutability
  proof or init mutex) — mechanical once refcounts are atomic.
- 02.05b: extend entry 23's epoch pattern into per-type version counters;
  the PyDict epoch is now a real atomic they can build on.
- 02.05g: GC safepoint protocol + ROOT_SEEDS hint-tolerance under no-GIL.
- Refcount story: Rc -> atomic-refcount (or Arc swap / per-thread heaps) —
  the prerequisite for EVERY Send/Sync marker. Owner: 02.05b/02.05g split.
- `INTERNED_NAMES` (entry 29): with per-thread heaps, interned handle
  identity across threads needs a shared intern table (or equality-by-value
  guarantee re-verified at the attr-probe sites).

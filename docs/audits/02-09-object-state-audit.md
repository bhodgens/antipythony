# 02.09-pre object-state mutability audit (findings only)

Campaign: robustness-ci held-queue item 2 ("02.09 sub-leaf p1: object-state
audit"). Measured 2026-10-30 on the live tree (HEAD 03db145 + dirty T2
fix-wave in `builtins.rs` / `builtins_modules.rs` and other sibling-owned
files). READ-ONLY census: grep/read only, no builds, no source edits.

Scope: `crates/ap-run/src/` and `crates/ap-obj/src/`.

Purpose: classify the object-state mutability surface, decide what is
already GIL-independent vs what blocks the 02.09 no-GIL swap (T121 >=3x
scaling GREEN gate, `plans/antipythony/master.md` Design intent + F3 row).

## Headline numbers (re-measured; old figure "300+ RefCell sites / 17 files")

- **RefCell: 285 textual occurrences (258 lines) across 18 files**
  (13 in ap-run, 5 in ap-obj). The old "300+/17" count included files
  since refactored and counted differently (occurrence vs line); on the
  current tree the surface is ~285 occurrences / 18 files — same order,
  slightly wider file spread (threading wave added threading*.rs).
- **Mutex: 47 occurrences (45 lines), concentrated in the threading/GIL
  machinery** (threading.rs 32, threading_call.rs 10, gcsupport.rs 4,
  ap-obj/int.rs 1 comment only).
- **UnsafeCell: 0.** No raw cell unsafety beyond two `unsafe impl Send/
  Sync` wrappers (`GilSendPub`) and one `static mut` (below).
- Struct-field declarations (distinct mutable slots, not occurrences):
  frame.rs 27 Interpreter fields, object.rs 18 PyObject fields,
  threading.rs 9 primitive fields, ap-obj/classes.rs 3 TypeObject fields,
  dict.rs 2 (PyDict/SetState), value.rs 1 (PyList items), interp.rs 1
  (scope_cache), iter.rs 1.

CAVEAT: sibling T2 fix wave is actively editing interp.rs / classes.rs /
methods.rs / builtins*.rs / frame.rs / generators.rs; counts for those
files are ±a few sites. Line citations for churn files are given as
regions.

## Per-file table

| File | RefCell occ | Mutex occ | What it holds | Cluster (see §Classification) |
|---|---|---|---|---|
| ap-run/frame.rs | 66 | 0 | `Interpreter` struct: 27 RefCell fields (locals/frames/registries/caches), region ~L20–L410 | C-INTERP |
| ap-obj/object.rs | 55 | 0 | `PyObject`: 18 RefCell payload fields (attrs, args, dunders, class_ref, defaults, gen_id, closure_slot…), region ~L79–L121 | C-PAYLOAD |
| ap-run/builtins_modules.rs | 32 | 0 | 11 `thread_local!` registries/caches: MODULE_CACHE, NAMED_CALLABLES, SD_REGISTRY, EXC_ATTRS, JSON_SEPS, ATEXIT_FNS, LINECACHE, MT (random), POLL_TABLES, LRU_STATE, JSON_DEFAULT_HOOK | C-TLCACHE |
| ap-run/interp.rs | 24 | 0 | thread_locals CELL_BACKING, CURRENT_INTERP, MODULE_FRAME_CELLS, PENDING_GEN_ERROR; `scope_cache` field; cell/sink construction sites | C-TLSTATE + C-INTERP |
| ap-run/threading.rs | 16 | 32 | GIL v1 machinery: GIL/GilState, THREADS, WORKER_INTERP, EXCEPTHOOK, MAIN_TID, CANCELLED, PRIMS, OBJ_MIRROR, JOIN_MU, TIDMAP; ThreadEntry RefCells | C-GIL |
| ap-run/classes.rs | 22 | 0 | mostly PyObject-field *construction* (`RefCell::new(...)` in bound-method make sites, region ~L1868–1885); CLASS_KWARGS thread_local | C-PAYLOAD (ctor) + C-TLSTATE |
| ap-run/importer.rs | 14 | 0 | thread_locals SYS_MODULES, SYS_PATH, SYS_MODULES_VIEW, LAST_MNF_NAME, BOOTSTRAP_SEEDED | C-TLSTATE (shared-semantics) |
| ap-run/threading_call.rs | 1 | 10 | blocking-call implementations: Mutex init sites for Lock/RLock/Event/Semaphore/Barrier/Queue state | C-GIL-adjacent (already Mutex) |
| ap-run/sre.rs | 9 | 0 | thread_local RE_CACHE, PAT_TABLE, MATCH_TABLE (regex caches) | C-TLCACHE |
| ap-run/gcsupport.rs | 7 | 4 | thread_locals WEAK_TOKENS, FINALIZERS, INTERNED_TYPE_PINS; `static mut FINALIZER_SEQ`; WORKER_REG_IN/OUT `GilSendPub<Mutex<...>>` | C-GC |
| ap-run/generators.rs | 7 | 0 | thread_local fiber tables ACTIVE/PARKED/CURRENT holding raw `*const FiberYielder` | C-TLSTATE (fibers) |
| ap-run/lxml.rs | 7 | 0 | thread_local DOCS registry + XPath cache | C-TLCACHE |
| ap-obj/classes.rs | 9 | 0 | `TypeObject`: mro, metaclass, class_value RefCells (region ~L50–L97) | C-PAYLOAD |
| ap-obj/dict.rs | 6 | 0 | `PyDict.state` / `SetState` RefCell (single-cell state design, ~L62–L87, L589) | C-PAYLOAD |
| ap-obj/value.rs | 5 | 0 | `PyList.items` RefCell (~L281–L319) | C-PAYLOAD |
| ap-run/builtins.rs | 2 | 0 | INTERNED thread_local (string intern table) | C-TLSTATE (shared-semantics) |
| ap-run/iter.rs | 2 | 0 | one iterator-state RefCell field | C-PAYLOAD |
| ap-obj/int.rs | 1+1 | (cmts) | comments only — `Gc<T> = Rc<T>` alias doc noting the MMTk swap point | n/a |

Files with zero sites: exc_hier.rs, format.rs, methods.rs, lib.rs,
sre_tables.rs, structmod.rs (ap-run); exc.rs, float_repr.rs, gc_gate.rs,
hash.rs, lib.rs, list_repr.rs, ops.rs, str_repr.rs (ap-obj).

## Classification

- **C-PAYLOAD — object payload (the shared Rc graph).** `PyObject`'s 18
  RefCell fields, `PyDict`/`PySet` state cells, `PyList.items`,
  `TypeObject` mro/metaclass/class_value, iterator state. This is the
  actual *object model*: `Gc<T> = Rc<T>` (ap-obj/int.rs ~L76) with
  RefCell interiors → every Value is `!Send + !Sync` by construction.
  ~78 RefCell occurrences, but the *reach* is total: every thread that
  touches any Value touches this cluster.
- **C-INTERP — interpreter state.** `Interpreter`'s 27 RefCell fields
  (frame.rs): locals, frame_stack, functions/iterators/generators tables,
  type_objects, exc_classes, stdout, super_stack, etc. Plus interp.rs
  construction/teardown. One `Interpreter` is shared process-wide by
  workers via `WORKER_INTERP: LazyLock<Mutex<GilSendPub<Rc<Interpreter>>>>`
  (threading.rs ~L200) — workers do NOT get private interpreters today.
- **C-TLSTATE — thread-local *state* with cross-thread semantics.**
  SYS_MODULES / SYS_PATH / SYS_MODULES_VIEW (importer.rs), CELL_BACKING +
  CURRENT_INTERP + MODULE_FRAME_CELLS (interp.rs), INTERNED (builtins.rs),
  fiber tables (generators.rs), CLASS_KWARGS (classes.rs). Race-free
  under any model (each thread its own copy) but *semantically wrong*
  without the GIL: CPython shares sys.modules/sys.path across threads, and
  the GIL v1 design relies on workers seeing the main thread's tables.
- **C-TLCACHE — thread-local pure caches.** builtins_modules.rs's 11
  registries, sre.rs RE_CACHE/PAT_TABLE/MATCH_TABLE, lxml.rs DOCS/XPath.
  Per-thread duplication is semantically fine (caches, counters, MT RNG
  per-thread is even desirable). **Already GIL-independent.**
- **C-GIL — the GIL machinery itself.** threading.rs + threading_call.rs:
  the `GIL` static, thread registries, sync-primitive Mutex state, and
  critically `GilSendPub<T>` (gcsupport.rs ~L1608): a blanket
  `unsafe impl Send/Sync` over `!Send` Values whose soundness is
  explicitly "GIL-serialized by protocol" (~L1603–1610). Also
  `OBJ_MIRROR`, `EXCEPTHOOK`, `WORKER_REG_IN/OUT` which store
  `GilSendPub<Value>` in global Mutexes.
- **C-GC — GC/weakref/finalizer support.** gcsupport.rs thread_local
  WEAK_TOKENS/FINALIZERS/INTERNED_TYPE_PINS, ap-gc REGISTRY/FROZEN/
  WEAKREFS main-thread-owned (per threading.rs header), and
  `static mut FINALIZER_SEQ: u64` (~L67, self-documented "single-threaded
  v0"). Under true concurrency the `static mut` is UB and the
  main-thread-owned GC registries are unreachable from workers without
  the GIL handshake.

## GIL-independent today vs blocking

**Already GIL-independent (no work needed for the swap):**
- C-TLCACHE entirely (builtins_modules registries, sre caches, lxml
  caches) — ~48 RefCell occurrences that are per-thread by construction.
- The threading *primitives* themselves (threading_call.rs Mutex state) —
  already std::sync, correct under any model.
- Atomics (NAME_COUNTER, NEXT_IDENT, SWITCH_INTERVAL, gc_gate HOOK,
  INT_STR_DIGIT_LIMIT_LIVE).

**Blocking the no-GIL swap, ranked by severity:**

1. **B1 — `GilSendPub` soundness inversion** (gcsupport.rs region ~L1600–
   1625; used in threading.rs THREADS/EXCEPTHOOK/WORKER_INTERP/OBJ_MIRROR,
   PRIMS, WORKER_REG_IN/OUT). Its `unsafe impl Send/Sync` is valid *only
   while the GIL exists*. Deleting the GIL without first deleting every
   `GilSendPub<Value>` crossing turns ~10 global statics into instant UB.
   This is the single hard coupling between "GIL" and "object state".
2. **B2 — C-PAYLOAD: the shared `Rc`+`RefCell` object graph.** Every
   Value is `!Send`. The GIL v1 model deliberately shares one Rc graph
   across OS threads (threading.rs header: "exactly CPython's model").
   No-GIL requires either (a) per-thread heaps + ownership-transfer
   bridges (the pon root-map direction the header names), or (b) an
   MMTk/tracing-GC swap with thread-safe handles (design already exists:
   docs/gc-mmtk-swap.md), or (c) Arc-ify payload cells (kills the
   single-thread perf the project promises). 18 PyObject fields +
   dict/list/type cells are the edit surface; the ~78 occurrences
   understate it because every `borrow()/borrow_mut()` call site in
   ap-run (thousands) is a participant.
3. **B3 — C-INTERP: one shared `Interpreter` behind WORKER_INTERP.**
   27 RefCell fields, all mutated during execution. No-GIL needs
   per-thread Interpreters (frame tables, locals, exception state are
   naturally per-thread) with only genuinely-global tables (type_objects,
   exc_classes) lifted into a shared locked/lock-free registry. The
   RefCell fields themselves can stay RefCell *if* each thread owns its
   Interpreter — this cluster is blocking only because of the sharing, not
   the cell type.
4. **B4 — C-GC: `static mut FINALIZER_SEQ` + main-thread-owned GC
   registries.** UB under concurrency (easy fix: AtomicU64), and the
   WEAK_TOKENS/FINALIZERS/REGISTRY topology must become either per-thread
   with a sweep rendezvous or shared-locked. Interacts with B2's heap
   decision — cannot be finalized before the heap model is picked.
5. **B5 — C-TLSTATE semantic migration.** SYS_MODULES/SYS_PATH/INTERNED/
   CELL_BACKING/MODULE_FRAME_CELLS must move from thread-locals to shared
   registries (or per-thread + import-lock) once threads run concurrently;
   today the GIL makes main-thread seeding visible to workers via
   install_worker_thread(). Data-race-free now, correctness-blocking at
   swap time.
6. **B6 — generators.rs raw-pointer fiber tables** (`*const FiberYielder`
   in thread_locals). Per-thread is fine, but the 02.09-pre generator
   rework (lazy suspension, in flight per
   plans/antipythony/02-native-backend/leaves/02.09-pre-generators.md)
   will churn this; audit after it lands, not before.

## Recommended partition order for the no-GIL swap

Ordered so each step is independently landable and gate-able (corpus +
T101–T115 stay green under the GIL throughout; only the last step flips
the gate):

1. **P1 — de-UB the easy statics.** `FINALIZER_SEQ` → `AtomicU64`. Zero
   semantic change, removes B4's UB half. (Trivial; can land any time,
   even before 02.09 dispatch.)
2. **P2 — per-thread Interpreters (B3).** Give each worker its own
   `Interpreter`; lift only type_objects/exc_classes/user_class_values
   into a shared `Mutex<HashMap>` (or RwLock) seeded at startup. Keep the
   GIL. T120–T122 must stay byte-identical. This shrinks WORKER_INTERP to
   a bootstrap seed and makes B3 disappear without touching the object
   model.
3. **P3 — pick and prototype the heap model (B2).** Decision gate:
   per-thread heaps + transfer bridge vs MMTk swap (docs/gc-mmtk-swap.md).
   The PyObject 18-field payload and dict/list cells get their Send story
   here. Prototype behind a feature flag with the GIL still on; measure
   against T121 baseline. This is the long pole and the only step that
   needs a locked-decision amendment if it contradicts master.md's MMTk
   trajectory.
4. **P4 — retire GilSendPub (B1).** Once P3 gives Values a real Send
   story, replace each `GilSendPub<T>` static (THREADS, EXCEPTHOOK,
   OBJ_MIRROR, PRIMS, WORKER_REG_*) with properly-typed Arc/channel
   structures; delete the unsafe impls. Hard rule: the GIL may not be
   removed while any `GilSendPub` remains — make that a grep-gated CI
   check (`! grep -r GilSendPub crates/`) at this step.
5. **P5 — migrate shared-semantics thread-locals (B5).** sys.modules /
   sys.path / INTERNED / CELL_BACKING / MODULE_FRAME_CELLS → shared
   registries with an import lock mirroring CPython's. C-TLCACHE stays
   thread-local (verified fine).
6. **P6 — GC topology finalization (B4 remainder).** Weak/finalizer
   registries per the P3 heap choice; MMTk handles weakrefs natively if
   that path was chosen, collapsing this step into P3.
7. **P7 — flip the switch.** Remove `acquire_gil/release_gil` from the
   44 call sites (39 in threading_call.rs, rest in threading.rs/interp.rs),
   delete the GIL static, run the T121 >=3x scaling gate as GREEN per
   master.md, plus full corpus + threading byte-exactness (T101–T115,
   T120/T122 order-independent bytes).

## Notes / risks

- The counts here are *textual occurrences*; the true participant surface
  is every `.borrow()/.borrow_mut()` on those cells, which is
  workspace-wide (ap-run alone is ~50k lines). The partition order above
  is deliberately structured so only P3 has to reason about that mass.
- threading.rs's header comment (02.07) already names the intended exit
  ("per-thread heaps with a cross-thread Weak bridge" vs the shared-heap
  easy path it shipped). P3 should formally choose between that and
  gc-mmtk-swap.md; both docs predate the T1 scrapy wave.
- Sibling churn: builtins.rs/builtins_modules.rs counts include the dirty
  T2 working tree; drift of a few sites is expected at commit time. None
  of the T2 edits change cluster membership (all C-TLCACHE/C-TLSTATE).
- methods.rs, format.rs, structmod.rs, exc_hier.rs, sre_tables.rs and 8
  ap-obj files have zero mutability sites — the surface is narrower than
  the raw file count suggests: 18 of 32 files, and 5 files (frame.rs,
  object.rs, threading.rs, builtins_modules.rs, interp.rs) hold ~2/3 of
  all RefCell occurrences.

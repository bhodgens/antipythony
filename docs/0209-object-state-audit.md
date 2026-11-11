# 02.09 object-state audit — no-GIL pre-implementation (sub-part 1 of 3)

Measured at HEAD 02113bb6f29f19c3dee40c1a453c21102edb052c (2026-11-05
13:36 -0700); landed onto shared HEAD cfd8b06. The 02113bb..cfd8b06 delta is
AGENTS.md / NEXT-STEPS-W112.md / plans/robustness-ci/PROGRESS.md +
ast_bridge.rs (27 lines); BRIDGE_SOURCE remains declared at
ast_bridge.rs:32 on cfd8b06 (verified), so every Rust citation below is
valid on both.
Predecessor: `docs/audits/02-09-object-state-audit.md` (2026-10-30 census,
cluster view). This doc supersedes it for the 02.09 implementation leaves: it
enumerates per-site inventories with verified file:line citations, adds the
GIL-removal seam punch-list, the GC-root map, the unboxed-storage rider
go/no-go, and the three-way sub-leaf split. READ-ONLY audit: no code changed.

Verdict key: **(a)** thread-confined by ownership — fine under no-GIL;
**(b)** needs atomic/sync wrapper; **(c)** needs confinement to the owning
interpreter (thread_local shared-across-interps/threads-by-name — latent
correctness bug); **(d)** GC-visible surface needing root-map /
conservative-scan coverage.

---

## 1. Interior-mutable surface inventory

### 1.1 ap-obj payload cells (the shared Rc graph)

`Gc<T> = Rc<T>` (crates/ap-obj/src/int.rs:76); every payload cell below is
`!Send + !Sync` by construction. Counts verified: 320 RefCell occurrences /
284 lines across ap-obj+ap-run+ap-gc; 45 Mutex lines; **zero** `UnsafeCell`
except the two documented unsafety seams (§1.3: `GilSendPub`, `static mut
FINALIZER_SEQ`).

| Site | Type | Guards | R/W pattern | Verdict |
|---|---|---|---|---|
| crates/ap-obj/src/value.rs:283 | `PyList.items: RefCell<Vec<Value>>` | list contents | every list op (§4 facade); `ys=xs; ys.append(4)` mutates `xs` (S-01-19 aliasing) | (b)/(d) — cross-handle shared; GC-tracked (value.rs:310,319 register at alloc) |
| crates/ap-obj/src/value.rs:287 | `PyList.allocated: Cell<usize>` | CPython growth model | read/write on append/shrink; unobservable (S-01-31-adjacent) | (a) under per-interpreter heaps; (b) if list shared |
| crates/ap-obj/src/value.rs:149 | `PyString.cached_len: Cell<Option<usize>>` | code-point count memo | write-once-on-first-non-ascii len() | (b) — benign racy-cache in CPython, but Rust UB: needs AtomicU64-encoded or Cell-removed |
| crates/ap-obj/src/value.rs:150 | `PyString.cached_hash: Cell<Option<i64>>` | hash memo | write-once-on-first-hash; read in every dict probe | (b) — same as cached_len; hottest cell in the runtime |
| crates/ap-obj/src/dict.rs:67 | `PyDict.state: RefCell<DictState>` | compact insertion-ordered dict (entries/indices/mask/used/dummies, dict.rs:71-86) | whole-state single cell so probes can't see half-mutation (dict.rs:63-64 comment); every `d[k]=v` | (b)/(d) |
| crates/ap-obj/src/dict.rs:618 | `PySet.state: RefCell<SetState>` | set/frozenset buckets (dict.rs:621-624) | membership ops | (b)/(d) |
| crates/ap-obj/src/object.rs:74-121 | `PyObject` — 18 RefCell fields: self_value:81, func:83, ints:85, slice_parts:87, attrs:90 (`RefCell<Gc<PyDict>>`), args:92, cause:94, context:95, dunders:97, gen_id:99, repr_text:101, module_ns:103, closure_slot:106, defaults:108, class_ref:111, type_ref:115, descr_kind:118, method_owner:121 | the entire generic object payload | attrs/dunders probed on every attribute access; class_ref/type_ref set at class creation (write-once-ish); cause/context at raise | (b)/(d). attrs:90 and dunders:97 are the hot shared-mutable pair; the rest are construction-time single-writes → mostly (a) with a write-once→finalize discipline |
| crates/ap-obj/src/object.rs:90 | `attrs: RefCell<Gc<PyDict>>` | instance `__dict__` | read at every getattr/setattr (e.g. lxml.rs:1522, interp.rs:13612 complex_parts) | (b)/(d) — and the double cell (`RefCell<Gc<..>>`) is itself removable: the inner PyDict is already interior-mutable; the outer RefCell only swaps the dict handle |
| crates/ap-obj/src/classes.rs:51 | `TypeObject.mro: RefCell<Vec<Rc<TypeRef>>>` | C3 linearization | `lookup`/`lookup_with_owner` borrow on EVERY attribute lookup (classes.rs:100-101, 109); write once post-construction (classes.rs:103 `*rc.mro.borrow_mut() = mro`) | (b) — read-mostly: candidate for write-once freeze + `Rc<Vec<..>>` clone-out, cheaper than a lock |
| crates/ap-obj/src/classes.rs:58 | `TypeObject.metaclass: RefCell<Option<Rc<TypeRef>>>` | `type(C)` | resolved after creation (02.03) | (a) after init-fence |
| crates/ap-obj/src/classes.rs:61 | `TypeObject.class_value: RefCell<Option<Value>>` | back-pointer `C` identity | set once at class construction | (a) after init-fence |
| crates/ap-obj/src/classes.rs:59 | `TypeObject.dict: Gc<PyDict>` | class namespace (`C.__dict__`) | mutated by class-body exec, setattr on class | (b)/(d) via PyDict |
| crates/ap-obj/src/gc_gate.rs:23 | `HOOK: AtomicUsize` | installed collector hook fn-ptr | store at gcsupport::install / install_worker_thread; load per allocation | already (b) — but see §1.3 hook-clobber finding |
| crates/ap-obj/src/int.rs:60-66 | `Int::Smi(i64)` / `Big(Gc<Big>)` | value arms | immutable once built | (a) |

### 1.2 Interpreter + frame state (crates/ap-run)

`Interpreter` (frame.rs:281) — 27+ RefCell/Cell fields (frame.rs:282-360:
globals/builtins Gc<PyDict>, functions/nested_functions, iterators,
generators, stdout, tb_frames, frame_stack, depth/recursion_limit Cells,
current_exception, registry, lambda_bodies, defining_frames, gen_sink,
si_value, sys_module, type_objects, stmt_lines/stmt_ordinal, exc_classes,
user_subclasses, user_class_values, object_class, type_class, self_rc:352).
`Frame` (frame.rs:21-49: locals RefCell<HashMap>, last_line Cell,
class_cell RefCell).

Today ONE `Interpreter` Rc is shared by all OS threads
(`WORKER_INTERP: LazyLock<Mutex<Option<GilSendPub<Rc<Interpreter>>>>>`,
threading.rs:200-202). Under no-GIL: **verdict (a) IF confinement flips** —
per-thread/per-interpreter ownership makes every one of these 27 cells
single-owner and they can stay RefCell. Lifting genuinely-global tables
(type_objects, exc_classes, user_class_values, object_class/type_class) into
a process registry is a sub-leaf-A edit. This matches the predecessor audit's
B3/P2 (docs/audits/02-09-object-state-audit.md).

`FnRegistry` (interp.rs:191): struct is per-run and lives inside
`Interpreter.registry: RefCell<Option<Rc<FnRegistry>>>` (frame.rs:311), but
its `scope_cache: RefCell<HashMap<..>>` (interp.rs:200) is interior-mutable
memo — the field comment at interp.rs:196-199 records the historical
thread_local-keyed-by-pointer aliasing bug (spurious NameErrors). Verdict
(a) under confinement; the leaked `&'static ScopeInfoModule`
(interp.rs:470 `Box::leak`) keeps FnRegistry contents immutable — fine.

### 1.3 thread_local! inventory (40 blocks, all in ap-run/ap-gc; ap-obj has ZERO — verified)

**Per-thread state with cross-thread/cross-interpreter SEMANTICS — class (c) latent bugs:**

| Site | Name | Guards | Pattern | Verdict |
|---|---|---|---|---|
| importer.rs:79 | `SYS_MODULES` | the import cache | inserted on every import (importer.rs:264); read sys_modules_get:255 | **(c)** — CPython shares sys.modules across threads. Seeded/cleared ONLY on main (interp.rs:111 reset_import_state; importer.rs:243-246). A worker thread sees an EMPTY table |
| importer.rs:80 | `SYS_PATH` | bootstrap search path | bootstrap_sys_path from main only (interp.rs:119); comment importer.rs:240-242: "intentionally persists" across runs | **(c)** — worker sees empty sys.path → `sys_path_entries()` (importer.rs:212-225) falls to empty SYS_PATH → any file-based import inside a thread target fails today. Also **(c) cross-run**: persists stale dir list into a second in-process run_module |
| importer.rs:88 | `SYS_PATH_VIEW` | live sys.path PyList | set via attach_sys_extras on main | **(c)+(d)** — holds Values (Rc) → root; worker-invisible |
| importer.rs:279 | `SYS_MODULES_VIEW` | python-visible sys.modules dict | mirror-synced on insert (importer.rs:270-274) | **(c)+(d)** |
| importer.rs:620 | `BOOTSTRAP_SEEDED` | once-flag for `_bootstrap` seeding | write-once-per-thread | **(c)** — worker re-seeds bootstrap modules into ITS private SYS_MODULES → duplicate module objects, `is` identity breaks across threads |
| interp.rs:9060 | `CURRENT_INTERP: RefCell<Option<Rc<Interpreter>>>` | thread→interp bind | set at run_module (interp.rs:95) and per worker (threading.rs:606 install_current_interp); read via current_interp_for_bindings (interp.rs:9069) / current_interp_handle (interp.rs:13602) | **(c)** — keyed-by-name indirection: two in-process interpreters on one thread alias the first; correct today only because a run owns its thread. The canonical (c) seam the no-GIL design must replace with explicit interp passing |
| builtins_modules.rs:505 | `CURRENT` (interp handle, 2nd copy) | module builder/bindings | set_current_interp (builtins_modules.rs:606-608) — the SAME dual-handle bug as CURRENT_INTERP, two parallel bindings of one concept | **(c)** |
| builtins_modules.rs:14841 | `CURRENT_MODULE_GLOBALS` | globals()/locals() during module-body exec (s022) | push/pop around module bodies | **(c)** — a worker importing concurrently needs per-thread confinement or an interpreter-scoped stack; today the GIL serializes |
| builtins.rs:2800 | `INTERNED` | sys-intern table | get-or-insert | **(c)+(d)** — `sys.intern` identity across threads is a CPython guarantee; per-thread table silently breaks `is` |
| interp.rs:23 | `CELL_BACKING` | __closure__ cell storage keyed by PyObject ptr (usize) | insert on cell-object creation; read on cell_contents | **(c)+(d)+(address-keyed)** — ptr keys recycle across threads/runs; the exact aliasing class documented at interp.rs:196-199 |
| interp.rs:13627 | `MODULE_FRAME_CELLS` | pre-created module-frame cells keyed by (frame ptr, name) | same pattern | **(c)+(d)+(address-keyed)** |
| generators.rs:73-80 | `ACTIVE`/`PARKED` (Vec<(u64, *const FiberYielder)>), `CURRENT`, `NEXT_FIBER` | fiber suspension stacks + id counter | push/pop around resume slices | **(a)** by design (fibers run on one thread) — BUT raw pointers + per-thread NEXT_FIBER counters hand out COLLIDING fiber ids across threads; PENDING_GEN_ERROR (interp.rs:7556) is keyed by fiber id → cross-thread id aliasing = **(c)** for the error-key space |
| interp.rs:7556/7558 | `PENDING_GEN_ERROR(_DEFAULT)` | staged generator errors keyed by fiber id | set/consume once | **(c)** — id collisions (above) |
| gcsupport.rs:51/53 | `WEAK_TOKENS`, `FINALIZERS` | weakref/finalize registries | appended on weakref()/finalize() | **(c)+(d)** — process-level objects (weakref.finalize registers cross-thread; atexit drain interp.rs:141 reads main-thread copy only). Holds Values → GC roots per thread |
| gcsupport.rs:1035-1039 | `INTERNED_TYPE_PINS` | strong pins for untracked Type objects | append-only | **(c)+(d)** |
| ap-gc gcmod.rs:48 | `REGISTRY` | THE collector: addr→Tracked clone | insert per tracked alloc (via hook), scan/sweep in collect | **(c)+(d)** — main-thread-owned by design (threading.rs:19-22 header); workers forward via channel (below). Worker-thread collect() would see an EMPTY registry — a live-object-sweep hazard if the flip ever runs GC on a worker |
| ap-gc gcmod.rs:62/65/69/75/80 | `CALLBACKS`, `FROZEN`, `WEAKREFS`, `FINALIZED_ADDRS`, `GARBAGE_NOW` | gc surface state | per collect()/freeze()/weak() | **(c)+(d)** (all main-thread-owned like REGISTRY; FROZEN/CALLBACKS are process-visible via gc module) |
| ap-gc gcmod.rs:89/91 | `ROOT_DICTS`, `ROOT_MODULE_NS` | explicit roots (globals/builtins/module ns) | add_root_* (gcmod.rs:191,201), clear_roots:214, called from gcsupport::install (gcsupport.rs:1622-1624) main-only | **(c)+(d)** — per-interpreter root sets: confinement must move these INTO the interpreter or key them per-thread with a rendezvous at collect |
| ap-gc weakbridge.rs:18/62 | `BRIDGE`, `IN_COLLECT` | collector→interpreter callbacks + reentrancy guard | set_bridge at install (main-only); IN_COLLECT thread-scoped | BRIDGE **(c)** (worker collect would fire main-thread function pointers via stale bridge); guard itself **(a)** |
| classes.rs:603 (ap-run) | `CURRENT_CLASS_NS` | exec-in-class-namespace for 1-arg exec | push/pop around exec_class_def | **(c)** — needs per-interpreter; two interpreters on one thread alias today |
| classes.rs:1494 | `CLASS_KWARGS` | class-body keyword staging | set/clear around class construction | **(c)** same |
| ast_bridge.rs:32 | `BRIDGE_SOURCE` | source text for t-string raw spans | write per module bridge, read per f-string node | **(c)** — a worker AST-bridging a module mid-run clobbers main's (or vice-versa; today the GIL at threading.rs:638 prevents it ONLY for worker-vs-worker, never main-vs-worker, see §2) |

**Pure per-thread caches/locals — class (a) or (b)-trivial (already GIL-independent):**

| Site | Name | Verdict |
|---|---|---|
| builtins_modules.rs:704 | `MODULE_CACHE` (native-registry modules) | **(c)-borderline**: native modules are constructed fresh per worker thread → `import math` identity differs across threads (same bug family as SYS_MODULES but lower severity: the registry() table builtins_modules.rs:707 is static fn-ptrs, rebuildable). Needs interpreter-confinement with SYS_MODULES for identity |
| builtins_modules.rs:503/504 | `NAMED_CALLABLES`, `NAMED_COUNTER` | **(b)** — counter needs Atomic; name→callable map: **(c)** for marker-callable identity across threads |
| builtins_modules.rs:507 | `NEXT_ITER` (id minter) | **(b)** → AtomicU64 |
| builtins_modules.rs:4589/4590/4592 | `SD_REGISTRY`, `SD_COUNTER`, `LRU_PENDING` | **(c)** for SD_REGISTRY/LRU_STATE (keyed by marker id, holds Values → also **(d)**); counter **(b)** |
| builtins_modules.rs:12793 | `LRU_STATE` | **(c)+(d)** — memoization caches Values; two threads memoizing the same `_lruwrapper_<id>` diverge (per-thread is CPython-acceptable for lru_cache *content* but not for wrapper identity) |
| builtins_modules.rs:9889 | `ATEXIT_FNS` | **(c)** — atexit registry is process-level in CPython; a worker registering atexit today loses it (drain reads main-thread slot, interp.rs:141 finalize_atexit_drain) |
| builtins_modules.rs:10180 | `MT` (random MT19937) | **(a)** — per-thread RNG is desirable (CPython's random is per-instance) |
| builtins_modules.rs:10970-10974 | `POLL_TABLES` | **(a)** (select poll objects are thread-local in practice); address-keyed → **(d)-none** (no Values) |
| builtins_modules.rs:6414/6416/6736/17759 | `EXC_ATTRS`, `LAST_JSON_ERR`, `JSON_SEPS`, `JSON_DEFAULT_HOOK` | **(a)** scratch state; JSON_DEFAULT_HOOK holds a Value → **(d)** if kept per-thread |
| builtins_modules.rs:9963 | `LINECACHE` | **(a)** pure cache |
| sre.rs:2017/2049/2050 | `RE_CACHE`, `PAT_TABLE`, `MATCH_TABLE` | **(a)-caches** but PAT/MATCH keyed by `Rc::as_ptr` (sre.rs:2053-2055) → cross-thread invisible compiled patterns: a worker matching a main-compiled `re.Pattern` object misses → runtime error → **(c)** for the address-key tables; RE_CACHE holds Gc<PyObject> → **(d)** |
| lxml.rs:1505 | `DOCS` (parsed-tree arena) | **(c)** — an lxml Element handle carrying `doc_id`/`idx` attrs (lxml.rs:1516-1527) is meaningless in another thread's DOCS; trees are Rc<RefCell<LxDoc>> — GC-adjacent (opaque, not registry-tracked → invisible to collector: scan must treat DOCS as a root set **(d)**) |
| lxml.rs:2891 | `XPATH_OBJECTS` | **(c)+(d)** — the XPath-object registry named in the leaf brief: compiled query + namespaces Value, id = vec index in marker name `lxml.etree.XPath.call.<id>` (lxml.rs:2896-2904). Cross-thread index into a per-thread vec = wrong-query execution, not just a miss |
| interp.rs:1912 | `LAST_PY_LOC` | **(a)** diagnostics |
| interp.rs:191-196 | `FILE_SRC` (fn-local static inside source_line_for) | **(a)** cache |
| interp.rs:10469 | `SUPERCALL_SKIP` (Cell<bool>) | **(a)** one-shot flag on the calling thread; **(b)** if a worker's flag could be consumed by main (today: no) |
| threading.rs:73-74 | `GIL_HELD`, `GIL_DEPTH` | deleted at the flip (§2) |
| ap-gc gcmod.rs:475, weakbridge.rs:63, gcmod.rs:53/57/59 | `SCAN_CONTENDED`, `IN_COLLECT`, `THRESHOLDS`, `ALLOC_SINCE`, `ENABLED` | **(a)** per-thread collector-control flags (§3 contention-abort); THRESHOLDS/ENABLED **(c)** — `gc.set_threshold`/`gc.disable` are process-level guarantees in CPython |

**Address-keyed global registries behind process Mutexes (not thread_locals):**
THREADS (threading.rs:178), LOCALS (threading.rs:185 — threading.local
namespaces keyed (obj addr, owner os tid), correctly keyed by tid),
EXCEPTHOOK (threading.rs:189), MAIN_TID (192), CANCELLED (195), WORKER_INTERP
(200), PRIMS (threading.rs:348), OBJ_MIRROR (threading.rs:791),
WORKER_REG_IN/OUT (gcsupport.rs:1678-1683). All Mutex-wrapped → already
**(b)**-shaped, but every `Rc`/`Value` payload rides `GilSendPub` → **(b)**
only because the GIL exists (§2 B1). All Value-holding maps are GC-visible →
**(d)**: LOCALS values, OBJ_MIRROR values, EXCEPTHOOK, WORKER_INTERP,
WORKER_REG queue contents.

### 1.4 The two documented unsafety seams (verified)

1. **`GilSendPub<T>`** — gcsupport.rs:1666-1668:
   `pub struct GilSendPub<T>(pub T); unsafe impl Send; unsafe impl Sync;`
   Blanket over `!Send` Rc payloads. Sound ONLY while a process-wide lock
   serializes access. Lives in 8+ globals (list above) and in the
   `std::mem::transmute` of the worker closure box at threading.rs:556-565
   ("the audited unsafety lives HERE and only here"). Hard rule for the
   flip: no `GilSendPub` may outlive the GIL (predecessor audit P4).
2. **`static mut FINALIZER_SEQ: u64`** — gcsupport.rs:67-76, self-labelled
   "Single-threaded v0". True data race (not just borrow panic) the moment
   two threads mint finalizer indices. Trivial AtomicU64 fix; can land
   before the flip.
3. Bonus (same class, found this pass): **`gc_gate::set_hook` clobber** —
   gcsupport.rs:1717 `install_worker_thread` (called by EVERY worker,
   threading.rs:609) rewrites the process-global HOOK from
   `ap_gc::gc_register` (direct, installed main-only at gcsupport.rs:1614)
   to `gc_register_forward_worker` → from the first thread spawn onward,
   MAIN's allocations also ride the mpsc channel, drained only at
   threading_call.rs:189 and :1581 (blocking-call points). Two consequences:
   (i) registration latency changes collection-trigger timing after any
   thread starts (GCSTATS/count drift); (ii) the last worker to exit leaves
   the hook pointing at a possibly-dead channel. Pre-existing design wart,
   amplified by no-GIL.

---

## 2. The GIL v1 removal seam

### 2.1 Where the lock lives (threading.rs)

- State: `GilState { mu: Mutex<u64 owner-tid, cv: Condvar }` in
  `static GIL: LazyLock` (threading.rs:62-70); per-thread reentrancy cells
  `GIL_HELD`/`GIL_DEPTH` (threading.rs:73-74).
- Protocol: `acquire_gil()` (91-105) spin-wait on condvar;
  `release_gil()` (108-133) depth-counted, `notify_all` at outermost;
  `GilGuard` RAII (136-149) — **`GilGuard::take` has zero callers** (grep
  across crates/).
- Call sites: 38 `acquire_gil()` + 15 `release_gil()` lines in
  threading_call.rs (the blocking-op rendezvous: join 627-629,
  Lock/RLock.acquire 912-940, Condition.wait, Event.wait, Semaphore,
  Barrier, queue get/put); threading.rs: worker timer path (628),
  `worker_bootstrap` (638 acquire, 659 release after done); interp.rs:134-136
  (release around `wait_for_non_daemons`, re-acquire after).

### 2.2 Finding C-BUG-1 (headline): the MAIN thread never takes the GIL

`run_module` never calls `acquire_gil` before `exec_module` — the only main-
thread touchpoints are the release/re-acquire bracket at interp.rs:134-136,
which runs AFTER the module body. `release_gil()` at entry is a no-op
(GIL_HELD=false early-return, threading.rs:110-111). So today the lock
serializes **worker-vs-worker only**: main-thread bytecode execution races
concurrently with whichever worker holds the GIL. Corroborating scars in
tree: the try_borrow spin-retry loops written specifically for "cross-thread
contention" — `frame_stack_pop` (interp.rs:1859-1875), `stmt_lines` read
retry (interp.rs:1930+), worker entry-stamp retry loop (threading.rs:611-
623), double-start try_borrow (threading.rs:459-465) — these are papering
over exactly this unlocked main path. The Rc+RefCell graph shared between an
unlocked main and a locked worker is a live UB hazard today (mitigated in
practice only because worker bodies mostly park at rendezvous ops). The
audit classifies this as the top (c)-class pre-existing bug: fix = acquire
at run_module entry (one line, keeps GIL v1 semantics) OR accept it as
evidence that the no-GIL flip removes MORE unsafe state than it creates.

### 2.3 Executor-level flip shape

`ap_run::run` as named in the orchestrator doc does not exist yet — the
entry point is `ap_run::run_module` (interp.rs:51, re-exported lib.rs:41;
callers ap-cli/src/main.rs:345, bin/ap_run_eval.rs:66,
bin/ap_obj_eval.rs:38). The flip:

1. delete `GIL`/`acquire_gil`/`release_gil`/`GilGuard` (threading.rs:62-149)
   behind the opt-in compat flag until T121 green, then remove the flag
   (per orchestrator §02.09 "retained ONLY as an opt-in compatibility mode").
2. delete the 53 call sites in 2.1 (threading_call.rs pairing: blocking ops
   already release/re-acquire around real Mutex+Condvar waits — deleting the
   GIL calls leaves the primitive waits intact; join_wait (threading.rs:681+)
   keeps its done-flag rendezvous).
3. worker_bootstrap (threading.rs:593-660): drop the 638 acquire / 659
   release; keep install_current_interp (606) + install_worker_thread (609)
   until their (c)-class replacements land.
4. interp.rs:133-136: the daemon-wait bracket loses its release/re-acquire
   (keep `wait_for_non_daemons`).

**The flip is only safe after the punch-list P1-P5 below**, because today's
"safe" is a three-way stack: GIL + `GilSendPub` blanket unsafety +
spin-retry loops, and §2.2 shows even that stack leaks for the main thread.

### 2.4 Punch-list (ordered, file targets)

| # | Change | Files | Gate |
|---|---|---|---|
| P1 | **DONE (02.09a, 76e25c2a):** `FINALIZER_SEQ` → AtomicU64; `NEXT_ITER`/`NAMED_COUNTER`/`SD_COUNTER`/`NEXT_FIBER` minters → process-atomic AtomicU64 | ap-run/src/gcsupport.rs:67; builtins_modules.rs:503-507,4590; generators.rs:80 | build + corpus green, no observable change — VERIFIED |
| P2 | **DECIDED (02.09a, recorded here):** the main-thread GIL hole stays OPEN for the compat window and is DOCUMENTED as the accepted interim — run_module body (incl. any acquire-at-entry) is 02.09-b's; the split doc's "document worker-only-serialization as the accepted interim and cite this split as the fix" arm is taken | ap-run/src/interp.rs:51-104 (02.09-b's file region) | threading slice — decision recorded in SPLIT-02.09.md §02.09a |
| P3 | **DONE (02.09a, 56a10c71..b361c70b):** all 21 (c)-class registries + CELL_BACKING_PINS/NAMED_BY_ADDR/CELL_WRAPPER_CACHE confined into the per-interp `RuntimeState` (crates/ap-run/src/runtime_state.rs) — process-global `Mutex<HashMap<InterpKey,T>>` keyed by `Arc::as_ptr(interp)`, seeded at run entry via gcsupport::install → set_active_interp, retired at run exit via retire_runtime_state; ~90 accessor sites rewired across importer.rs/builtins_modules.rs/builtins.rs/sre.rs/lxml.rs(DOCS+XPATH already process-Mutex)/interp.rs/classes.rs/ast_bridge.rs; the dual-handle CURRENT/CURRENT_INTERP seam dissolved (builtins_modules::current_interp routes to the ACTIVE handle) | ap-run/src/{runtime_state.rs (NEW), importer.rs, builtins_modules.rs, builtins.rs, sre.rs, interp.rs, classes.rs, ast_bridge.rs} | T101-T115 rendezvous slice byte-exact; T120/T122 — differential suites green (see 02.09a gate record) |
| P4 | **DONE-minus-heap (02.09a, eb08a396):** `TypeObject.mro` frozen write-once (RefCell → Mutex, seeded FULL inside `create`/the frame.rs sentinel before any observer; zero post-construction mutators — verified by grep); `PyString.cached_hash` → AtomicI64 with the siphash13 sentinel audit PASSED (hash_bytes u64→i64 can only hit -1 via full-output u64::MAX; CPython-parity `-1 => -2` normalization applied before store; 0 = uncomputed is idempotent). The MMTk-handle vs per-interp-heap decision remains the 02.09-b heap gate (out of this leaf per the split) | ap-obj/src/{classes.rs:51, value.rs}; ap-gc (heap decision = 02.09-b) | ap-obj unit tests 66/66 + cpython_differential + repr_fuzz green; hash byte-exactness pinned by existing hash differential suites |
| P5 | Retire `GilSendPub` (B1): THREADS/LOCALS/EXCEPTHOOK/WORKER_INTERP/PRIMS/OBJ_MIRROR/WORKER_REG_* payloads → Arc/typed channel; delete gcsupport.rs:1666-1668 + threading.rs:563-565 transmute; grep-gated CI ban | ap-run/src/gcsupport.rs, threading.rs, threading_call.rs | hard precondition of the flip |
| P6 | GC topology (B4): per-thread or shared registry decision; root-set ownership move (ROOT_DICTS/ROOT_MODULE_NS/WEAK_TOKENS/FINALIZERS/INTERNED_TYPE_PINS → per-interpreter + per-thread conservative roots, §3); drain-ownership for worker registration channel; hook-clobber fix (set_hook idempotent / per-thread hook) | ap-gc/src/{gcmod.rs,weakbridge.rs}, ap-run/src/gcsupport.rs | gc slice + parallel mutator contention-abort preserved |
| P7 | Flip: delete GIL + 53 call sites + compat flag after T121; run gate: T121 ≥3x on 4 cores GREEN, T101-T115 + T120/T122 order-independent bytes, full corpus, gc slice | threading.rs, threading_call.rs, interp.rs | the 02.09 exit gate |

P8 (rider, §4): unboxed list storage — independent, may land any time after
P3/P5 for the list type.

---

## 3. GC angle

**What gc-mmtk-swap.md commits to about roots** (docs/gc-mmtk-swap.md §1, §4):
today roots = explicit dicts (globals/builtins via `add_root_dict`), module
namespaces (`add_root_module_ns`), `sys.modules` values via the weakbridge
hook (gcsupport.rs:1637-1638 → importer.rs:251), plus the subtract-refs
invariant which *for free* covers native-stack Rc temporaries
(gc-mmtk-swap.md:21-25). Under MMTk §4.1: `scan_roots_in_mutator_thread`
walks ROOT_DICTS/ROOT_MODULE_NS/sys.modules + **native-stack temporaries need
an explicit shadow stack or pon-style stackmap** — "root enumeration for
native frames is entirely ours" (§4 preamble, :65). §4.2: global roots via
`scan_vm_specific_roots` "covers the static tables (interned types, fn
registry, module cache)".

**Per-thread root sets that MUST be scanned (the (d) column of §1.3)** — the
MUTATOR_THREADS × thread_local product the flip adds: per thread —
`ap-gc REGISTRY` (main only today), `ROOT_DICTS`, `ROOT_MODULE_NS`,
`WEAKREFS`, `FROZEN`, `FINALIZED_ADDRS`, `CALLBACKS`, `CALLBACKS_LIST`;
per thread in ap-run — `SYS_MODULES`(+_VIEW), `MODULE_CACHE`, `NAMED_CALLABLES`,
`SD_REGISTRY`, `LRU_STATE`, `INTERNED`, `CELL_BACKING`,
`MODULE_FRAME_CELLS`, `WEAK_TOKENS`, `FINALIZERS`, `INTERNED_TYPE_PINS`,
`RE_CACHE`, `PAT_TABLE`, `MATCH_TABLE`, `XPATH_OBJECTS` (Option<Value>),
`DOCS` (Rc arenas, collector-invisible), `ATEXIT_FNS`, `JSON_DEFAULT_HOOK`,
`CURRENT_CLASS_NS`, `SYS_PATH_VIEW`; process globals — `LOCALS` dicts,
`OBJ_MIRROR`, `EXCEPTHOOK`, `WORKER_INTERP`, `WORKER_REG_*` queues,
`FN registry`/interned type pins. Pre-mmtk (tier-0 parallel, subtract-refs)
the conservative fallback is: **keep every Value-holding thread_local
reachable from a process-global root table** — i.e. register each thread's
root-bearing locals with a shared collector rendezvous (the P6 decision), or
delete the invisibility outright via P3 confinement so `interp` IS the root
set the collector already walks. P3 confinement makes the GC root problem
~70% of the (d) column disappear — that coupling is the audit's central
argument for ordering P3 before P6/P7.

**Contention-abort rule — confirmed in code.** Scan-time:
`mark_reachable`/object scan uses try_borrow on `attrs/func/self_value/
cause/context/dunders` (ap-gc/src/gcmod.rs:269-283); any contended borrow
sets `SCAN_CONTENDED` (:276, :283; cell declared :475). `collect_gen`
consumes it: `if SCAN_CONTENDED... return 0` (gcmod.rs:496-499) — the cycle
aborts, never sweeps a partial view (rule + historical incident documented at
gcmod.rs:869-873; test `contention_abort_never_sweeps_and_clears_flag`
gcmod.rs:1037-1072). Under no-GIL this rule becomes load-bearing for
parallel mutation: a concurrent mutator borrow is normal, not exceptional,
so abort-and-retry-at-next-threshold (:499 comment) is the designed
interim; the doc gate "parallel mutator + collector contention aborts
preserved" (orchestrator §02.09 gate) maps directly to keeping gcmod.rs:496
semantics byte-identical.

---

## 4. Unboxed-storage rider (tier-0: list-of-Smi as raw i64 array)

**Where lists store today.** `PyList { items: RefCell<Vec<Value>>, allocated:
Cell<usize> }` (ap-obj/src/value.rs:279-288); `Value` is the 16-byte enum
(value.rs:56-76; Int arm is `Int::Smi(i64)` inline, ap-obj/src/int.rs:60-66)
so a list of small ints today pays 16 B/elem + RefCell + a per-elem `Value`
tag. Constructors `new`/`from_vec` (value.rs:308-322) register with the GC
hook; clone deep-copies items (value.rs:291-296).

**Facade a typed region must preserve.** PyList public surface (value.rs
impl block): `new`, `from_vec`, `len`, `items() -> Ref<'_, [Value]>`,
`append`, `get`, `set_at`, `remove_at`, `remove_range`, `clear`, `fmt_repr`
(+ Clone/Debug). The hard constraint is **`items()` returning a borrowed
`[Value]` slice** (value.rs:24/33-36 of the impl): ~180 `.items()` call
sites across ap-run (interp.rs 44, builtins_modules.rs 31, builtins.rs 26,
methods.rs 16, classes.rs/importer.rs 13 each, gcsupport.rs 12, ops.rs 8,
threading_call.rs 7, dict.rs 7 + repr paths) read `&[Value]` in place. A raw
i64 storage can only serve those by (i) materializing a boxed Value slice on
non-Smi access (kills the win for mixed reads), (ii) iterator-based facade
rewrite of the 180 sites, or (iii) a tagged PyList variant with
`items_as_values()` slow path — the CPython compact-int-list approach keeps
`ObItemId32` union + direct reads at C sites. (iii) is the shape compatible
with the 16-byte Value contract.

**Fence tests.** crates/ap-run/tests/method_surface_differential.rs (71
tests — list method surface), crates/ap-obj/tests/cpython_differential.rs +
repr_fuzz.rs (repr/growth/aliasing), gc slice (`tests/corpus/gc/` — the
subtract-refs/registry must see unboxed lists identically: strong_count of
the PyList Rc is unchanged; internal-element edges become invisible to
`count_internal_refs` ONLY IF the list could hold refs — Smi-only by
construction, so a gated invariant "unboxed ⇒ no Value refs ⇒ still
registered for container identity" must be pinned), and the threading
T120/T122 corpus (list mutation across threads).

**Go/no-go + effort.** GO, but as its own sub-leaf (02.09-C) AFTER
audit+confinement lands: the storage variant is orthogonal to the flip and
must not couple to it. It does NOT fit one 256K leaf whole: PyList variant
type + write-gate + repr/sort/aliasing paths ≈ 1 slice (256K leaf);
`.items()` escape-hatch audit across the 180 sites ≈ 1 slice; fence-gate
runs (differential + gc + corpus) ≈ 0.5-1 slice. Recommendation: 2-3 leaf
slices, Smi-only v1, defer bytes/str-ascii variants.

---

## 5. Split verdict — three sub-leaves, disjoint file sets

Orchestrator names them (audit+wrappers) / (GC roots) / (unboxed storage).
Based on the punch-list (P1-P8), the split that keeps dispatches parallel
and files disjoint:

- **02.09-A — audit + Send/Sync wrappers + interpreter confinement**
  (P1, P2, P3, P4-minus-heap-decision, P5).
  Owns: ap-run/src/{importer.rs, builtins_modules.rs, builtins.rs,
  classes.rs, ast_bridge.rs, sre.rs, lxml.rs, generators.rs, frame.rs,
  threading.rs, threading_call.rs}, ap-run/src/gcsupport.rs (GilSendPub
  retirement only), ap-obj/src/{object.rs, classes.rs (mro freeze),
  dict.rs, value.rs (Cell caches only)}. Gate: P3+P5 land green **under
  the still-present GIL**; T101-T115 + T120/T122 bytes unchanged.
- **02.09-B — GC roots + flip** (P6, P7).
  Owns: ap-gc/src/{gcmod.rs, weakbridge.rs}, ap-run/src/interp.rs
  (run_module root-wiring + daemon bracket + the GIL-removal edit at
  :133-136), ap-run/src/gcsupport.rs (hook-drain topology — coordinate:
  file split from A's GilSendPub edits via A-commit-first ordering).
  Gate: gc slice green with parallel mutator; **T121 ≥3x on 4 cores
  GREEN** — this leaf owns the flip moment, so it dispatches AFTER A.
- **02.09-C — unboxed list storage** (§4 rider).
  Owns: ap-obj/src/value.rs (PyList region) + ap-run list dispatch points
  in methods.rs/interp.rs item-read sites (post-B merge, or sequenced on
  top of A's value.rs cache edits — value.rs is the only overlap with A;
  keep A's Cell-cache edits to the PyString region, C's to the PyList
  region). Independent of the flip; may dispatch any time after A.

**Critical-path ordering: A → B; C parallel-after-A.** The overlap hot-spots
to arbitrate at dispatch time: gcsupport.rs (A vs B), value.rs (A vs C),
interp.rs (B owns; A may confine registries read through it — keep A's
interp.rs edits to the CELL_BACKING/MODULE_FRAME_CELLS declarations, B's to
run_module body).

## Appendix — headline numbers (re-measured on this HEAD, grep-verified)

RefCell 320 occurrences / 284 lines / ap-obj+ap-run+ap-gc; Mutex 45 lines
(GIL machinery concentrated); UnsafeCell 0; thread_local! blocks 40 (ap-run 35 across 12 files, ap-gc 5 across 2 files); ap-obj
thread_locals: **0** (the object layer is pure Rc-graph — confinement debt
lives entirely in ap-run/ap-gc); PyObject RefCell fields 18
(object.rs:81-121); Interpreter RefCell/Cell fields 27 (frame.rs:282-360);
GilSendPub unsafe seams 2 impls (gcsupport.rs:1667-1668) + 1 transmute
(threading.rs:563-565); GIL call sites 53 (threading_call.rs 38+15 minus
pairing overlap: 38 acquire-lines, 15 release-lines), `GilGuard::take`
dead code; main-thread GIL acquisitions on the exec path: **0** (§2.2).

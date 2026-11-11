# SPLIT-02.09 — no-GIL swap (GIL v1 removal) decomposition

Design leaf, read-only output. Base inputs:
- orchestrator.md §02.09 (the gated-leaf spec + its own split note:
  "(audit+wrappers) vs (GC roots) vs (unboxed storage) — three sub-leaves,
  same gate") + Dispatch notes ("do not let 'after tier-1' prose re-gate it")
- docs/0209-object-state-audit.md (the 2026-11-05 per-site census @5aca9234:
  interior-mutable surface inventory with file:line citations, the §2 GIL
  removal seam + P1–P8 punch-list, §3 GC angle, §4 unboxed-storage rider,
  §5 the A/B/C split verdict — this split IMPLEMENTS that verdict)
- docs/no-gil-object-audit.md (phase-1 leaf 02.09 @87c7bbb1: 30 ap-obj
  surfaces classified (a)/(b)/(c)/(d), 2 behavior-neutral conversions
  landed, the Rc-refcount blocker statement, phase-2 handoff list)
- docs/gc-mmtk-swap.md (root topology: subtract-refs invariant, §4 root
  scanning contract, §9 what does NOT change)
- docs/pon-study-tier1.md (§Background compile threading model — the
  pin/snapshot/CAS discipline already landed in 02.05e is "exactly right for
  a no-GIL runtime"; §GC integration points — the root-reporting contract)
- docs/pon-study-lowering.md (thread model of the driver/compiler split)
- SPLIT-02.05.md (format model; its §02.09 ordering recommendation —
  superseded by landed facts, see the ordering section below)
- tests/t10_threading/test_t10_threading.py (the gate harness: T121 scaling
  `test_scaling_gate_3x_on_4cores_antipythony` @xfail(strict=False); spot
  set T101–T115/T120/T122; STILL_XFAIL_T10_SPOTS = {T109})

## What already landed (do NOT re-scope)

Verified in tree at dd258919:

| Landed | Commit | Notes |
|---|---|---|
| GIL v1: threading.rs (1856 L) + threading_call.rs (2062 L) | 02.07-era | GilState mu/cv @threading.rs:61-66; acquire/release @106/123; GilGuard @151 (**`GilGuard::take` has zero callers** — dead code); 38 acquire + 15 release sites in threading_call.rs; worker_bootstrap acquire/release; interp.rs:180-182 daemon-wait bracket |
| `docs/no-gil-object-audit.md` (phase 1) | 87c7bbb1 | 30 ap-obj surfaces classified; PyDict.epoch Cell→AtomicU64 (audit entry 23, dict.rs:78) + PyString.cached_len Cell→AtomicUsize (entry 27, value.rs:213) CONVERTED; **zero Send/Sync markers** (Rc blocker); phase-2 handoff list |
| P1 (part): FINALIZER_SEQ static mut → AtomicU64 | 6f94b261 | gcsupport.rs:70; the remaining P1 minters (NEXT_ITER builtins_modules.rs:510, NAMED_COUNTER :507, SD_COUNTER :5132, NEXT_FIBER generators.rs:92) are still thread-local Cells |
| `docs/0209-object-state-audit.md` (per-site census) | 5aca9234 | 40 thread_local! blocks inventoried (ap-run 35 / ap-gc 5 / ap-obj 0); §2.2 headline finding **C-BUG-1: the main thread never takes the GIL** (run_module never acquires; the lock serializes worker-vs-worker only); punch-list P1–P8 + §5 split verdict |
| 02.05g rootscan.rs: SafepointMap + fp-chain walker + capture-point seam | 7349a898/d42212d | ap-gc/src/rootscan.rs — the JIT-side root feed §02.09 names as its pairing item; process-global RwLock registry; **per-thread capture point** (jit_hook.rs:864 set / :897 clear); conservative floor = subtract-refs invariant retained |
| 02.05e TierUpDriver + deopt tables | bfa9f9c/66fcf77 | process-global OnceLock<Mutex> DEOPT_TABLE (deopt.rs:143 — the 02.05 union-fix); CAS install, snapshot-at-enqueue, pin-until-retire — already multi-thread-correct on the COMPILER side |
| ap-obj atomics surface | 02.05b-era | versioning.rs BUILTINS_VERSION/type_version/surgery_version AtomicU64; int.rs digit-limit AtomicUsize |
| Worker GC registration channel | gcsupport.rs:1799-1841 | WORKER_REG_IN/OUT mpsc of GilSendPub<Value>; install_worker_thread rewrites the process-global HOOK (the audit §1.3 hook-clobber finding); drain_worker_registrations at main-thread turns |
| GC contention-abort rule | gcmod.rs:43-45/897 | SCAN_CONTENDED try_borrow aborts the pass, retry at next threshold; `contention_abort_never_sweeps_and_clears_flag` pins it — the orchestrator gate "contention aborts preserved" maps to THIS code, byte-identical |
| Gc<T> = Rc<T> | int.rs:76 | the blocker every surface shares (phase-1 doc): non-atomic refcounts + values legally cross threads since 02.07 (queue/T101–T115) ⇒ no Send/Sync marker is provable today |

Open per the audits: the (c)-class lock story for PyObject.attrs/PyDict.state/
PyList.items(+allocated)/PySet.state/TypeObject.mro; interpreter confinement
of the 20+ thread_local registries (P3); GilSendPub retirement (P5); the GC
topology decision (P6); the flip itself (P7); unboxed list storage (P8/rider);
PyString.cached_hash conversion (deferred on a hash-sentinel audit).

## The three decisions that gate everything (decide in-leaf, record in leaf reports)

1. **Refcount story: `Gc<T> = Rc<T>` → atomic refcounts.** Every Send/Sync
   marker is unprovable until this lands (phase-1 doc, "The blocker every
   surface shares"). Recommended: swap the alias to `Arc<T>` (or a custom
   atomic-refcount box with Rc's API) — mechanical at the call sites because
   Gc<T> is already a type alias with no weak-count surgery outside ap-gc;
   preserves the 16-byte Value contract, address stability, and the
   subtract-refs arithmetic (counts just become atomic). DO NOT couple this
   to the MMTk stage-2 handle swap (docs/gc-mmtk-swap.md §7 step 1) — that
   migration keeps its own sequencing; this flip only needs counts that are
   sound under concurrent clone/drop. Cost: Arc drop contention on hot
   shared objects (interned strings, builtins) is the T121-relevant risk —
   measure before and after on the M2 Ultra (24-core), keep the numbers in
   the leaf report.
2. **Lock strategy for the (c) surfaces: decide against real contention, not
   in the abstract** (phase-1 doc's deferred note). PyDict.state and
   PyList.items(+allocated — must move under the SAME lock, audit entry 26)
   are multi-word algorithmic state → per-object parking Mutex or
   sharded/RwLock; TypeObject.mro is read-mostly (walked on EVERY attribute
   lookup, written once post-construction) → write-once freeze +
   clone-out/Rc-swap, NOT a per-access lock; PyObject.attrs' outer
   RefCell<Gc<PyDict>> double-cell is removable (the inner dict is already
   interior-mutable — 0209-object-state-audit §1.1 row for object.rs:90).
   Fallback that must remain available: the dict/list epoch counters
   already AtomicU64 — epochs+retry reads where lock traffic shows up hot.
3. **GC topology for the first flip: shared-heap, conservative, quiesced at
   rendezvous.** The spec explicitly blesses this: "the conservative-scan
   fallback keeps tier-0 parallel but slower — acceptable for the first
   flip". Concretely: keep ONE registry + the subtract-refs invariant (now
   atomic via decision 1); keep the contention-abort rule semantics
   byte-identical (it is gate-named); make collections quiesce mutators at
   the points that already exist (blocking-op rendezvous in
   threading_call.rs are natural safepoints — workers are parked there by
   construction) OR run try-collect with abort-on-contention; do NOT build
   the MMTk stop-the-world handshake in this split (that is the stage-2
   swap's deliverable, and 02.05g's rootscan already provides the JIT-side
   feed for it). What MUST be added: a **mutator thread census** — the
   collector must see every live thread's root-bearing thread_locals and
   (once JIT frames exist under threads) every thread's rootscan capture
   point; today both are main-thread-only. This is 02.09-b's core edit.

## Sub-leaves

Naming: 02.09a…02.09c, matching the orchestrator split-note triad and the
census doc's §5 A/B/C verdict (a ≈ a, b ≈ b, c ≈ c). Each is one-256K-leaf
dispatchable. File ownership is exclusive per sub-leaf (hot-spots called out
below). **Same gate for all three** (the leaf is gated as a unit): T121 ≥3x
on 4 cores GREEN + threading slice + full conformance + gc slice.

---

### 02.09a — Audit remediation: confinement + Send/Sync wrappers (P1-rest, P2, P3, P4, P5)

**Scope:** Everything that must be true while the GIL still exists, so the
flip diff (02.09-b) stays reviewable. In punch-list terms: P2 decision +
P3 + P4-minus-heap-decision + P5, plus the un-landed half of P1.

- P1-rest: NEXT_ITER / NAMED_COUNTER / SD_COUNTER / NEXT_FIBER thread-local
  Cell minters → process-atomic (AtomicU64) — id-generation spaces must be
  process-unique before threads mint ids concurrently (fiber-id collision
  into PENDING_GEN_ERROR keys is the audit's named aliasing bug).
- P2: decide the main-thread GIL hole (C-BUG-1) WITH the orchestrator before
  the flip so the diff-at-flip is reviewable: either acquire at run_module
  entry (one line, keeps GIL v1 semantics honest for the compat window) or
  document worker-only-serialization as the accepted interim and cite this
  split as the fix. Either way the decision is recorded here, not re-litigated.
- P3 (the bulk): interpreter confinement — hang the (c)-class thread_local
  registries off `Interpreter` (or a per-interp `RuntimeState` struct
  seeded by the spawning interp, workers getting handles through the
  WORKER_INTERP table): SYS_MODULES/SYS_PATH/SYS_PATH_VIEW/SYS_MODULES_VIEW/
  BOOTSTRAP_SEEDED/MODULE_CACHE/NAMED_CALLABLES/ATEXIT_FNS/INTERNED/
  SD_REGISTRY/LRU_STATE/XPATH_OBJECTS/DOCS/PAT_TABLE/MATCH_TABLE/RE_CACHE/
  CELL_BACKING/MODULE_FRAME_CELLS/CLASS_KWARGS/CURRENT_CLASS_NS/BRIDGE_SOURCE.
  This ALSO fixes the dual-handle CURRENT_INTERP/builtins-CURRENT bug
  (explicit interp passing) and — the census's central argument — makes
  `interp` itself the root set the collector already walks, dissolving
  ~70% of the GC-visible (d) column before 02.09-b touches GC.
- P4-minus-heap: TypeObject.mro write-once freeze (post-construction, the
  classes.rs:103 write site is the only mutator); PyString.cached_hash →
  atomic WITH the hash-domain sentinel audit (CPython maps -1→-2; verify
  our siphash13 domain never collides with the sentinel before converting —
  the phase-1 doc's explicit precondition); the PyObject set-once fields
  get their init-phase discipline documented per type (markers land only
  after decision 1 makes them provable — the markers themselves may be
  staged here behind the refcount swap if it lands in this leaf, see Deps).
- P5: retire `GilSendPub` (gcsupport.rs:1774-1788, the transmute at
  threading.rs:868): THREADS/LOCALS/EXCEPTHOOK/WORKER_INTERP/PRIMS/
  OBJ_MIRROR/WORKER_REG_* payloads → Arc/typed channels; grep-gated CI ban
  (`GilSendPub` must not outlive the GIL — census P4 rule). This is the
  hard precondition of the flip; it can land and be proven green UNDER the
  GIL because Arc clones are correct under the lock too.

**Files (exclusive for the split's duration):** crates/ap-run/src/
{importer.rs, builtins_modules.rs, builtins.rs, classes.rs, ast_bridge.rs,
sre.rs, lxml.rs, generators.rs, frame.rs, threading.rs,
threading_call.rs}; crates/ap-run/src/gcsupport.rs (GilSendPub retirement
+ P1 minter edits); crates/ap-obj/src/{object.rs, classes.rs (mro freeze),
value.rs (PyString region ONLY — see 02.09-c for the PyList region rule)};
docs/no-gil-object-audit.md + docs/0209-object-state-audit.md (rows
PENDING→DONE). interp.rs is NOT in this set except the two declarations
CELL_BACKING/MODULE_FRAME_CELLS moving out of thread_local — the run_module
body belongs to 02.09-b.

**Risks:** this is the largest leaf by file count — the registries live in
12 files and some are read on every bytecode dispatch (RE_CACHE, INTERNED);
mechanical confinement first, hot-path second. builtins_modules.rs is
19,371 L and the hottest merge target in the repo — land in waves, commit
per registry family. The (c)→lock conversions must NOT change bytes under
the GIL (per-object Mutexes are uncontended ⇒ behavior-identical; the
threading slice pins this).

**Gate:** everything green WITH the GIL still in place: threading slice
T101–T115 + T120/T122 byte-exact; full workspace `cargo test`; pytest
threading suite unchanged; `grep GilSendPub crates/` returns zero; corpus
full green (confinement is observation-invisible). No T121 assertion yet —
the lock is still in.

**Deps:** none external; dispatchable NOW. Decision 1 (refcounts) may be
pulled INTO this leaf or staged at its end — pull it in if the leaf has
budget, because 02.09-b cannot start without it.

---

### 02.09b — GC topology + the flip (P6, P7) — owns the T121 gate

**Scope:** The flip moment. Two movements:

- **P6 GC topology (pre-flip, provable under the GIL):** per the decision-3
  spine — keep the shared registry + subtract-refs (now atomic); add the
  mutator thread census: a process-global table of live worker threads,
  each entry carrying (a) the thread's root-bearing state handle (post-a
  confinement makes this the Interpreter), and (b) the thread's rootscan
  capture point (rootscan.rs's capture point is per-thread; a collection
  must walk every live mutator's JIT frames, not just the collecting
  thread's). Fix the hook-clobber (install_worker_thread overwrites the
  process-global HOOK — make the hook per-thread or idempotent, audit §1.3
  finding 3); give the worker registration channel a real owner (drain on
  collect, not only at main-thread turns).
- **P7 the flip:** delete `GIL`/`acquire_gil`/`release_gil`/`GilGuard`
  behind the opt-in compat flag (`APX_GIL=1` env — per orchestrator:
  "retained ONLY as an opt-in compatibility mode flag until T121 is green,
  then deleted"); remove the 53 call sites (threading_call.rs blocking-op
  release/re-acquire pairs dissolve — the primitive Mutex+Condvar waits
  stay; interp.rs:180-182 keeps `wait_for_non_daemons` minus the bracket);
  worker_bootstrap drops its acquire/release, keeps install_current_interp
  + install_worker_thread (post-a these are confinement plumbing, not GIL
  plumbing). Remove the `@xfail_ap` mark from
  `test_scaling_gate_3x_on_4cores_antipythony` when it goes green (and
  T109 from STILL_XFAIL_T10_SPOTS if the no-GIL run clears it — flip, never
  weaken). After T121 ≥3x is measured green, delete the compat flag path in
  a follow-up commit inside this leaf.

**Files:** crates/ap-gc/src/{gcmod.rs, weakbridge.rs, rootscan.rs
(thread-census walk only — the map/fp-chain machinery is 02.05g's, DO NOT
redesign it)}; crates/ap-run/src/interp.rs (run_module body: root wiring +
the daemon-bracket edit + CURRENT_INTERP seam consumed post-a);
crates/ap-run/src/gcsupport.rs (hook topology — coordinate with a:
**a commits its gcsupport.rs edits first, b rebases on them**; the file is
the split's only a/b overlap); crates/ap-run/src/{threading.rs,
threading_call.rs} (delete-only after a has landed its edits there);
tests/t10_threading/test_t10_threading.py (gate-mark flips only, no
weakening).

**Risks:** soundness of collect-under-parallel-mutation: with atomic
refcounts, a false-RETAINED verdict is benign and a false-COLLECTED verdict
requires a decrement racing the scan — the quiesce-at-rendezvous /
abort-on-contention posture (decision 3) exists precisely to keep that
direction unreachable for the first flip; the fence test must drive
collect() concurrent with a mutating worker 100/100 (the tierup_race_stress
harness pattern is the model). Single-thread regression is the accepted,
temporary cost (per-object locks tax the hot path; tier-1 recovers it —
the SPLIT-02.05 reasoning carries over unchanged). Race-stress +
tier_mode_differential gates must re-run: the TierUpDriver/deopt tables are
process-global but the MUTATOR side was single-threaded until now — adaptive
mode + N mutator threads is a new surface (CAS install is already correct;
verify the feedback-cell seqlock path under threads).

**Gate:** THE 02.09 exit gate, all four legs: **T121 ≥3x on 4 cores GREEN**
(best-of-3 per mode, harness `test_scaling_gate_3x_on_4cores_antipythony`,
no longer expected-red) AND threading slice byte-exact (T101–T115 + T120/
T122 order-independent outputs) AND full conformance corpus green with the
lock removed AND gc slice green (parallel mutator + collector contention
aborts preserved — gcmod.rs:43-45 semantics byte-identical). Plus
`cargo test --workspace` 0 failed and the tier gates (tier_mode_differential
3/3, race-stress 100/100).

**Deps:** 02.09a (hard: GilSendPub must be gone and confinement landed —
the flip removes the last leg of the three-way "GIL + GilSendPub +
spin-retry" stack; census §2.3). Decision 1 (atomic refcounts) must be
landed by a first.

---

### 02.09c — Unboxed list storage rider (§4 of the census; orchestrator's "unboxed storage" arm)

**Scope:** PyList-of-Smi as a raw i64 array behind the Value facade — the
tier-0 half of master.md's compact-layout intent. Census §4 verdict: GO,
as its own leaf, AFTER a (it needs the lock story settled to define the
storage variant's synchronization), INDEPENDENT of the flip (may land
before or after b — it must not couple to the flip moment). Shape:
tagged PyList variant with Smi-only storage + write-gate on insertion
(any non-Smi append demotes the list to boxed Vec<Value>); `items() -> Ref<[Value]>`
is the hard constraint (~180 call sites read `&[Value]` in place) — census
option (iii): keep a materialized-boxed-slice slow path via
`items_as_values()`, audit every `.items()` site for the escape hatch.
Smi-only v1; bytes/str-ascii variants deferred.

**Files:** crates/ap-obj/src/value.rs (PyList region ONLY — a's value.rs
edits are confined to the PyString cache region; this is the split's only
a/c overlap, sequenced: a lands first); crates/ap-run list dispatch points
in methods.rs + interp.rs item-read sites (post-b merge or sequenced on
top — coordinate with b's interp.rs ownership via b-commit-first).

**Risks:** the gc interaction is the subtle one: unboxed ⇒ no Value refs ⇒
the list's internal-element edges vanish from `count_internal_refs` — the
census prescribes a gated invariant "unboxed ⇒ no refs ⇒ still registered
for container identity" pinned by a fence test (gc family must stay 20/20
byte-exact). Aliasing (S-01-19 self-mutation during sort) and repr paths
must be demote-or-handle. Threading: T120/T122 exercise cross-thread list
mutation — the variant's lock discipline must be settled with a's lock
decision, not invented ad hoc.

**Gate:** gc slice + method_surface_differential (71 list tests) +
repr_fuzz + cpython_differential all byte-identical; T120/T122 unchanged;
full corpus green (in whichever mode is current — under the GIL if it
lands pre-b, adaptive post-b); a micro-probe showing the memory win is
measurable (list-of-int heap probe), recorded but NOT gate-pinned (the
orchestrator gate is correctness; the memory intent rider rides along).

**Deps:** 02.09a (lock story + value.rs sequencing). Independent of 02.09-b.

---

## Dependency DAG

```
02.09a (confinement + wrappers + refcounts)
  ├──> 02.09b (GC topology + THE FLIP — owns T121 >=3x gate)
  └──> 02.09c (unboxed list storage)   [parallel with 02.09b, after a]
             (02.10 scheduler implementation waits on b's flip — orchestrator §02.10)
```

Recommended dispatch order (one heavy cargo lane — serial unless the
orchestrator says the sibling lane is free):

1. **02.09a** — unblocks both siblings; largest file-count leaf, budget
   2 waves; land in waves with per-family commits (builtins_modules.rs is
   a 19k-L merge magnet).
2. **02.09b** — the flip + gate; budget 2 waves (P6 must go green under
   the GIL before the P7 diff lands).
3. **02.09c** (parallel-eligible after a) — memory rider, never blocks the
   gate; do NOT let it delay b.

## Why this split honors the earlier ordering advice (superseding SPLIT-02.05's)

SPLIT-02.05 recommended "dispatch 02.09 FIRST, before 02.05c" on grounds
that the IC substrate's atomicity would be untestable under the GIL. The
premise was right but the sequencing pressure is now gone: 02.05 a–h and
02.06 have ALL LANDED (2.14x geomean verified on trunk; AoT binary
verified), built on GIL v1, and their no-GIL exposure is bounded and
known: (1) the IC version counters and dict epoch are already atomics
(02.05b's design rule "no per-GIL shortcuts" paid off — nothing to
re-audit); (2) the TierUpDriver/deopt tables were made process-global in
the 02.05e union fixes (the thread_local DEOPT_TABLE bug is already
fixed); (3) the compiler thread never took the GIL. The residual JIT-side
work under no-GIL is exactly one item: verify tier-1 runs correctly with
N concurrent mutator threads (race-stress extension) — absorbed into
02.09-b's gate, no new sub-leaf. T121 drives the interpreter (APX_IMPL),
not the JIT: tier-1 perf is orthogonal to the gate, exactly as SPLIT-02.05
predicted. No 02.05 re-work is a precondition of any 02.09 sub-leaf.

## Machine-load note (binding)

Every sub-leaf gate runs under the ONE-heavy-cargo-lane policy
(docs/test-interpreter-lock-policy.md): CARGO_BUILD_JOBS=4, workspace gates
only when the lane is free. 02.09-b is the noisy one: T121 is a TIMING gate
(best-of-3 × 2 modes × ~300s timeouts) plus the 100/100 collect-vs-mutator
race fence — schedule both when no sibling build lane is active, and run
T121 pinned (taskset-free, no competing cargo) or the 3x threshold
measures noise. 02.09a's differential batteries and 02.09c's fuzz families
are ordinary fenced probes.

## LICENSE note

This split vendors nothing from pon; the only pon-derived machinery
involved (rootscan.rs, tierup.rs discipline) landed under the granted
MIT/Apache-2.0 dual license with attribution headers already in file.
No new LICENSES/ work is triggered by any sub-leaf.

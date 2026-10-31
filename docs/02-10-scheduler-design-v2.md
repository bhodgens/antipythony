# 02.10 v2 — Go-style runtime scheduler: fiber-era revision

Status: RESEARCH ONLY (no code). Supersedes `docs/02-10-scheduler-design.md` (v1,
2026-10-22 tree) in part; v1 remains valid for its §1 (GMP framing), §2
(Python-visible constraints), §5 risks, and §6 open questions. **v2 exists
because v1's load-bearing premise is now false**: v1 §3.0 stated "there is no
suspension primitive in ap-run at all — generators materialize". Commit
`7c0a645` (2026-10-25, "feat(02.09-pre): PEP-342 lazy generator suspension —
corosensei stackful fibers") shipped exactly such a primitive. Every
current-code claim below cites a file read on the 2026-10-30 tree;
`generators.rs` is cited by REGION, not line, because a sibling session is
actively editing it (per dispatch context; `git status` confirms live churn in
the worktree).

Implementation remains gated: 02.10 code lands only after 02.09 (no-GIL swap)
is green — orchestrator.md §02.09/§02.10 and "Dispatch notes"
(plans/antipythany/02-native-backend/orchestrator.md:115-140).

## 0. What changed since v1 (the delta that forces this revision)

1. **Generators are now stackful fibers.** `crates/ap-run/src/generators.rs`
   (module header + `make_fiber`/`gen_yield`/`resume_live` regions) builds one
   `corosensei::Coroutine<GenInput, GenOutput, ()>` per def-generator. The
   fiber's Rust stack *is* the suspended frame — locals, call depth, loop
   cursors, try/except stack all live on it between resumes. `Expr::Yield`
   suspends via `yielder.suspend(GenOutput::Yield(v))`; `send`/`throw`/`close`
   resume with a `GenInput`. Genexprs keep the v0 materialization path
   (documented in the module header). This is v1's option **(a)-lite**:
   stackful suspension, shipped not for the scheduler but to fix pytest
   pluggy wrapper hooks (issue #7, per docs/t1-gate.md and NEXT-STEPS.md
   Wave-9 notes).
2. **Fiber stacks are 64 MiB each.** `make_fiber` uses
   `corosensei::stack::DefaultStack::new(64 * 1024 * 1024)` with the comment
   that the 1 MiB default overflowed on vendored-stdlib shapes (nested
   genexpr/listcomp inside generator bodies costs real Rust stack because the
   tier-0 executor recurses natively). corosensei's `DefaultStack` is an
   `mmap` region with a guard page (vendored source,
   `corosensei-0.3.4/src/stack/unix.rs`: `MAP_ANONYMOUS|MAP_PRIVATE|MAP_STACK`,
   guard page per `stack/mod.rs` contract).
3. **Coroutines are `!Send` by construction.** corosensei documents: "There is
   no way to guarantee [that all data on the stack is Send] using Rust language
   features so `Coroutine` does not implement the `Send` trait" — with an
   explicit note that a *manual* `Send` impl is sound only if everything live
   on the suspended stack is Send (`corosensei-0.3.4/src/coroutine.rs` doc
   region, "# Send" sections; compile-fail test asserts `Coroutine: !Send`).
   Our fiber bodies capture `Rc<Interpreter>` (`gen_fiber_body` region) —
   `Rc` is `!Send`, so no manual impl is available without the 02.09 audit
   converting the interpreter graph to Send-safe handles.
4. **Fiber identity state is thread-local.** `ACTIVE`/`PARKED`/`CURRENT`/
   `NEXT_FIBER` thread-locals track (fiber_id, yielder-ptr) stacks for
   nested yield-from (generators.rs thread_local region);
   `PENDING_GEN_ERROR` is a thread-local in interp.rs (region near the
   GenFrame enum). NEXT-STEPS.md "Wave-9 addendum" records the operational
   consequence: fibers resume on the SAME OS thread, so `CURRENT_INTERP` and
   friends survive resume — the design *depends* on thread affinity today.
5. **T1 scrapy is green** (docs/t1-gate.md) and **T2 pytest is blocked on
   generator-driven run-phase bugs** (NEXT-STEPS.md Wave-9/10; AGENTS.md
   status block) — i.e. the fiber machinery is load-bearing for ecosystem
   tiers NOW, before any scheduler exists. The scheduler must not destabilize
   it.
6. **GIL v1 and the blocking-op switch-point audit are unchanged** since v1
   read them: `threading.rs` spawns each Python thread as an OS thread with a
   64 MiB stack (`std::thread::Builder ... .stack_size(64 << 20)`, region
   ~lines 560-566), TLS is keyed by `current_os_tid()`, `sys.setswitchinterval`
   is stored-inert (`SWITCH_INTERVAL` atomic, threading.rs:190-191), and every
   blocking primitive releases/reacquires the GIL at audited points (54
   `release_gil`/`acquire_gil` occurrences in threading_call.rs — count via
   grep on the 2026-10-30 tree).

**Net effect on v1's recommendation.** v1 recommended "(c) substrate → (b)
resume-able explicitly-stacked executor", arguing (b) rides the mandatory
generator fix. The generator fix instead rode **(a)-lite (fibers)**. The
explicit-stacked-executor refactor of interp.rs (reify every compound-statement
continuation as data) has NOT happened and is now the *more* expensive path,
because a second suspension mechanism would coexist with the fiber one. v2
therefore recommends: **task = fiber** (one suspension mechanism), with a
work-stealing scheduler constrained by fiber `!Send` — detailed in §1/§3.

## 1. Work-stealing vs M:N for a fiber-based Python runtime

First, a category correction the leaf text blurs: work-stealing is a
*scheduling policy* (where runnable tasks come from), M:N is a *threading
model* (how tasks map to OS threads). Go is both (per-P local run queues,
steal-half when empty — v1 §1, golang.design under-the-hood ch. 9;
go.dev/src/runtime/HACKING). The real design axis for us is **what the
suspension primitive is**, because it constrains which M:N mappings are even
sound:

| Primitive | State today | Migration across OS threads | GC root story | 10k-task memory |
| --- | --- | --- | --- | --- |
| **Stackful fiber (corosensei)** | SHIPPED for generators (generators.rs) | `!Send` — **cannot** move a suspended fiber between threads | suspended fiber stack = machine stack needing (conservative) scan at STW | 64 MiB virtual/fiber today; tunable |
| **Explicit-stacked executor (v1's (b))** | not built; would be a second mechanism next to fibers | trivially movable (heap data) | exact roots, no stack scan | small per task |
| **1:1 OS threads (02.09 baseline)** | shipped shape (threading.rs spawn region) | n/a | native stack = mutator context | 64 MiB + kernel thread per Python thread — fails the 10k gate |

Since the fiber is the primitive we *have*, the M:N mapping question becomes:
**can a task ever change OS threads?** Three options:

**(M1) Thread-pinned tasks + steal-at-birth.** A Python `Thread` becomes a
`Task` holding an *unstarted* fiber spec (`FnEntry`, func_obj, args, kwargs —
the `GenFrame::Fn`-shaped data, interp.rs GenFrame region). corosensei runs
the closure lazily on first `resume` (crate docs; generators.rs `make_fiber`
comment "does NOT start it"). So the fiber's machine stack comes into
existence **on whichever worker first resumes the task**. Rule: a task may be
stolen/migrated only while its fiber has never started; after first resume it
is pinned to that worker (P) for life. Work-stealing still functions: idle Ps
steal half of a victim's *not-yet-materialized* task deque, which is exactly
the shape of the fan-out workloads the 10k-task gate cares about (spawn 10k,
most parked, few runnable at any instant). Parked-but-started tasks simply
don't participate in stealing — they resume where they live. This mirrors
Java Loom's constraint in spirit: Loom virtual threads *can* remount on
different carriers because continuations are heap-copied stack chunks
(cr.openjdk.org/~rpressler/loom/loom/sol1_part1.html "State of Loom"), but
Loom also accepts **pinning** — cases where the virtual thread cannot unmount
(native calls, `synchronized`) and blocks its carrier. Our fibers are pinned
*always*; that is strictly weaker than Loom but strictly stronger than 1:1,
and it needs zero unsafe.
- Cost: a long-running CPU task pins its worker (Loom has the same problem
  with pinned threads and compensates by growing the carrier pool up to 256 —
  rockthejvm.com virtual-threads guide, javaperformancetuning.com tips:
  default parallelism = cores, compensation threads for pinning). We adopt the
  same compensation lever (§3).

**(M2) Manual `unsafe impl Send` for Task after the 02.09 audit.** corosensei
explicitly sanctions this *if* everything on the suspended stack is Send.
Post-02.09 the interpreter graph must already be Send-safe (orchestrator.md
02.09: "every RefCell interior-mutable surface ... thread-confined or
atomic/sync wrapper"; the v1-era `transmute` escape at threading.rs:557-561
region goes away). But the fiber stack also holds *Rust* locals of the
tree-walking executor — `RefCell` borrow guards, `Rc` temporaries, `Value`
stacks — whose Send-ness is unauditable at the granularity corosensei
requires, and the thread-locals (`ACTIVE`/`PARKED`/`CURRENT`,
`PENDING_GEN_ERROR`) would need per-task relocation on every migration. The
NEXT-STEPS.md Wave-9 pytest debugging already shows how subtle fiber/thread
state coupling is on the *same* thread. Verdict: unsound-in-practice; rejected.

**(M3) Explicit-stacked tasks (v1's (b)) as a second mechanism.** Keep fibers
for generators only; build the reified-continuation executor for scheduler
tasks. Fully migratable, exact GC roots, cheapest 10k-task memory. But it is
the pervasive interp.rs refactor v1 costed (every compound statement needs a
continuation record), it must interoperate with fibers anyway (a task body
that runs a generator crosses both mechanisms — nested suspension across two
stack disciplines), and T2 pytest is being debugged *through* the fiber path
right now. Verdict: correct end-state candidate, wrong next move; revisit
only if (M1)'s pinning proves user-visible.

**Recommendation: (M1), with work-stealing restricted to unstarted tasks and
Loom-style worker-pool compensation for pinned/blocked carriers.** It reuses
the one suspension mechanism that exists, is proven against T1/T2-tier
stdlib shapes, and requires no unsafe. The 10k-task gate then hinges on fiber
stack sizing, not on the scheduler (§5, risk R1).

## 2. Scheduler × MMTk safepoints

The MMTk design (docs/gc-mmtk-swap.md) fixes: binding via `mmtk-rust` +
`mmtk-antipythany` plan crate, **MarkSweep (non-moving) first**, roots via
`Scanning::scan_roots_in_mutator_thread` + `scan_vm_specific_roots`, frame
locals via conservative shadow stack (or pon-style stackmaps later), weakrefs
via `ReferenceProcessor` (§§2-5). The stage-1 collector's **contention-abort**
(`SCAN_CONTENDED` thread-local; `collect_gen` returns 0 rather than sweep a
partial view — crates/ap-gc/src/gcmod.rs:472-499 and the unit-test doc comment
at :865-873) is the no-GIL stopgap: under 02.09's parallel mutators a scan
can hit a mid-mutation borrow and must abort. 02.09's gate explicitly keeps
"parallel mutator + collector contention aborts preserved"
(orchestrator.md:110-111).

The scheduler changes this picture in four ways:

**2.1 Mutator contexts become workers, not Python threads.** With M:N, the
set of threads MMTk stops and scans is the fixed worker pool (P count +
compensation threads), not the Python-thread count. `scan_roots_in_mutator_thread`
walks, per worker: the *currently mounted* task's live state (its fiber is the
current machine stack — ordinary mutator-stack scanning), the worker's shadow
stack, and the worker's run queue entries *that have never started* (those are
just heap `FnEntry`-shaped specs — ordinary object graph, reachable through
the task registry root).

**2.2 Suspended fibers are machine stacks that must be scanned while parked.**
This is v1 §3a's "single largest risk", now real and already shipped for
generators: a generator suspended at `yield` holds `Rc<Frame>` handles,
`Value` temporaries and interpreter borrows *on its corosensei stack*,
invisible to any heap-only root walk. Two consequences:
- *MarkSweep (non-moving)*: a **conservative scan of each suspended fiber's
  stack slab** at STW suffices — addresses don't move, over-retention is
  floating garbage only. The fiber stack bounds are known to corosensei
  (`DefaultStack` base/len, vendored `stack/unix.rs`), so the binding can
  enumerate slabs from a task/fiber registry. This is precisely Go's compromise
  for async-preempted frames: conservative scan of the interrupted frame
  rather than per-instruction maps (golang.design under-the-hood §9.7;
  go.googlesource.com proposal 24543 "conservative inner-frame" sub-proposal).
- *Copying-plan flip (gc-mmtk-swap §7 step 6)*: conservative roots + moving
  collector = every fiber stack must be scanned *precisely* or pinned. This
  is a hard gate on the plan flip, not on 02.10 — record it in
  gc-mmtk-swap's step-6 audit list when that leaf runs.

**2.3 STW handshake replaces contention-abort as the primary mechanism; abort
stays as backstop.** The protocol shape we copy is pon's (docs/pon-study-tier1.md
"Green Tea GC integration points"): generated/interpreter code polls
`pon_safepoint_poll()` at loop headers, answering a global
`pon_gc_stop_requested()` flag; threads acknowledge and wait for global
resume. Our tier-0 poll site is the per-statement bookkeeping that already
exists (`touch_line` / `interp.stmt_ordinal` advance, interp.rs:624-652
region) — v1 §3b located the same hook. Under a working STW handshake, a
collection never observes a mid-mutation heap, so `SCAN_CONTENDED` should
*never* fire; keep the abort as a correctness backstop (fail-safe, not
fail-wrong) exactly as 02.09's gate demands, and add a counter so a firing is
visible as a protocol bug.

**2.4 Park/unpark must be GC-visible.** A task parked on a `ParkKey` (v1 §4:
the transplant of the audited `release_gil/acquire_gil` blocking-op list) is
reachable only from the park table — the task registry must be a
`scan_vm_specific_roots` entry (like the existing `ROOT_DICTS`/module-ns
feeds, gc-mmtk-swap §4), or parked tasks' frames get swept. Finalizer
ordering (`GARBAGE_NOW` seq snapshot, gcmod.rs:506-521 region) is orthogonal
but must keep working under multi-worker collection: sequence numbers are
already global monotonic, so no change needed.

## 3. G-to-OS-thread mapping and preemption

**Mapping (from §1, M1):**
- `P count = APX_GOMAXPROCS | available_parallelism()` — Go's GOMAXPROCS
  accounting (v1 §1).
- Each P owns: a worker OS thread, a local deque of Tasks (unstarted:
  stealable; started: pinned), and the thread-local fiber bookkeeping
  (`ACTIVE`/`PARKED`/`CURRENT`) *as-is* — the pinning rule means those
  thread-locals never need cross-thread repair.
- Global spawn queue for `thread_start` (threading.rs spawn region becomes
  Task-construction + enqueue, replacing `std::thread::Builder`).
- **Compensation lane (Loom's carrier-pool growth):** tasks blocked in
  *native* syscalls that cannot park cooperatively (file IO, subprocess —
  today's GIL-release sites that wrap real blocking syscalls) may promote to a
  short-lived OS thread, capped (Loom caps compensation at 256; we cap at
  P×4 or similar, tunable). Pure-Python blocking primitives (Lock/Event/
  Cond/Queue — the threading_call.rs release/acquire pairs) park the Task with
  no thread cost at all: this is where the 10k-task memory win comes from.
- TLS re-key (v1 §2.2): `LOCALS` inner map keyed by `ThreadEntry.ident`, not
  `current_os_tid()`; `native_id` becomes "worker running me right now" and
  may change between statements for *unstarted-then-stolen* tasks only —
  document in compat-contract (v1 §6.3 recommendation stands).

**Preemption: signal-based vs cooperative polling.**

Python's only natural preemption points are bytecode/statement boundaries
(CPython's own `switchinterval` is a GIL-drop request checked at bytecode
edges; we inherit the surface — `SWITCH_INTERVAL` stored-inert,
threading.rs:190-191, 26-28). The two candidate mechanisms:

- **Signal-based (Go 1.14 async preemption).** Go sends `SIGURG`, the handler
  checks `isAsyncSafePoint(pc,...)` and, if safe, *rewrites the resume PC* to
  inject an `asyncPreempt` call that spills all registers; GC then scans the
  injected frame conservatively (go.googlesource.com proposal 24543;
  go.dev/src/runtime/signal_unix.go `sigPreempt`/`doSigPreempt`;
  golang.design §9.7). Go needed this after cooperative loop-preemption cost
  7.8% geomean and still left straggler gaps (proposal 24543 "Background").
  For us it is **categorically worse than for Go**: (i) Go preempts *its own
  generated code* with compiler-known unsafe-point metadata; our "bytecode"
  is Rust tree-walking frames with `RefCell` guards and `Rc` temporaries live
  at arbitrary points — a signal landing mid-`borrow_mut` and force-switching
  stacks would resume elsewhere with a borrowed `RefCell` on the parked stack
  (any later borrow from another task on the same structures deadlocks or
  panics); (ii) we have no per-instruction stackmaps, so *every* fiber frame
  would need conservative scanning on every preempt; (iii) EINTR fallout
  (golang.design §9.6: even Go must tolerate `EINTR` retries everywhere);
  (iv) Wasm target (Plan 03) has no signals at all — a signal mechanism is
  native-only, splitting the runtime's semantics per backend, which
  master.md's "same trait surface so semantics stay identical" contract
  forbids. The one legitimate use of signals: **handler sets an atomic flag,
  nothing else** — turning a sleeping/timer-driven deadline into a poll-visible
  request. No stack switch in the handler, no soundness surface. Recommended
  as an optional refinement of the poll below.

- **Cooperative polling at bytecode-equivalent boundaries.** Poll sites, in
  priority order: (1) per-statement — the existing `touch_line`/`stmt_ordinal`
  path (interp.rs:624-652 region), same site as the GC handshake flag, so one
  atomic-relaxed load serves *both* safepoint and quantum check (v1 §5 costed
  this as noise against µs-scale tier-0 statements); (2) loop back-edges and
  call entries in the hot exec paths — Go pre-1.14 relied on function-prologue
  checks and that was *adequate for scheduling* (its failures were GC-latency
  and tight-inlined-loop stragglers; proposal 24543 "Background" items 1-4);
  our statements are coarse, so back-edge polling is where the tight-loop
  coverage comes from; (3) long native loops we own (regex engine `sre.rs` —
  v1 §5 flagged it) get an explicit poll or run on the compensation lane.
  Quantum: a per-task statement budget derived from `sys.setswitchinterval`,
  finally giving the stored value meaning (v1 §2.3). Precedent: Tokio's
  per-task operation budget — 128 ops per tick, resources return
  "not ready" when exhausted, `tokio::task::consume_budget()` for CPU loops
  (tokio.rs/blog/2020-04-preemption; docs.rs tokio::task::coop). Tokio is the
  honest comparison class: a cooperative runtime that *cannot* preempt and
  says so, bounding tail latency with budgets instead. Java Loom is the same
  story at thread granularity: "The scheduler does not currently implement
  time sharing for virtual threads" (JEP 444, quoted in the HN thread
  news.ycombinator.com/item?id=37570723; State of Loom: no JDK scheduler
  employs time-slice preemption of virtual threads).

**Preemption verdict: cooperative statement+back-edge polling, sharing one
flag-word with the MMTk handshake; optional signal handler that only *sets*
the flag for deadline-driven preemption; never stack-switch from a signal.**
Safepoint latency is bounded by one statement (strictly better than Go's
async preemption for GC pause bounds — v1 §3b's argument survives intact).
CPU-hog fairness comes from the switchinterval budget at those polls.

## 4. Leaf decomposition for when 02.09 lands

Entry condition for the whole sequence: **02.09 green** — no-GIL, T121 ≥3x on
4 cores, threading corpus green, gc slice green with contention-aborts
preserved (orchestrator.md:108-111). Sequencing honors v1 §6.1's split
recommendation, updated: the "build the generator fix early" half is DONE
(7c0a645); what remains is scheduler-side only. Each sub-leaf fits one 256K
agent; test cases are authored by the T-track from the spec pack, never by the
implementer (master.md test protocol).

| Leaf | Name | Scope | Entry gate | Exit gate |
| --- | --- | --- | --- | --- |
| **02.10a** | Permit+pool substrate (v1's "2a", fiber-era) | `scheduler.rs` (new): `Permit`/`RunQueue`/`Task{ident, spec, fiber: Option<Coroutine>, state, parked_on}`; worker pool sized P (+compensation cap); `thread_start` → Task enqueue (replaces `std::thread::Builder` spawn, threading.rs spawn region); steal-half **of unstarted tasks only**; TLS re-key `LOCALS`→ident, `native_id` semantics change documented in compat-contract | 02.09 green; generators.rs fiber work settled (sibling committed — T2 wave done) | Full threading corpus (T101-T115, T120-T122) + s038 green; 3-run determinism (tests/t10_threading protocol) green; **each Python thread still runs its own fiber on its assigned worker — no behavior change beyond pool plumbing**; gc slice green |
| **02.10b** | Park/unpark migration | Every audited `release_gil/acquire_gil` pair in threading_call.rs becomes `park(ParkKey)`/`unpark(task)`; pure-Python prims (Lock/RLock/Cond/Event/Sem/Barrier/Queue) park the Task with zero thread cost; native-syscall sites take the compensation lane; `WORKER_INTERP` single-slot deleted; `PENDING_GEN_ERROR`/fiber thread-locals verified worker-scoped under M:N | 02.10a green | Threading corpus + queue slice green under M:N; **10k-spawn case completes with bounded RSS** (ratio-vs-serial gate per v1 §6.4; fiber stack size is the dominant term — see R1); T121 scaling stays green |
| **02.10c** | Safepoint protocol (scheduler×MMTk) | Statement-poll handshake flag shared with GC STW (pon pattern, pon-study-tier1.md:135-160); task/fiber registry as `scan_vm_specific_roots`; conservative scan of suspended fiber slabs in `scan_roots_in_mutator_thread` binding work (pairs with the mmtk spike deferred from 02.04, gc-mmtk-swap §8); `SCAN_CONTENDED` retained as counted backstop | 02.10b green; mmtk-core dependency spike done (02.05 window owns the fetch, gc-mmtk-swap §8) | gc corpus green *under the scheduler*; `APX_MMTK_STRESS=1` collection-during-threading stress case green; contention-abort counter reads 0 in all green runs (nonzero = protocol bug, fail loud); gc002/gc008/gc011/gc016 semantics preserved (gc-mmtk-swap §10 list) |
| **02.10d** | Switchinterval quantum | Per-task statement budget from `sys.setswitchinterval` at the same poll; loop back-edge polls in hot exec paths; optional deadline-signal flag-setter (native only, behind the same poll) | 02.10c green | CPU-hog corpus case (T-track authored, order-independent per T101 template) shows bounded hog latency; 3-run determinism green; T121 green; no corpus case newly sleep-order-dependent (v1 §5 rule) |
| **02.10e** (stretch) | Fiber stack policy + asyncio shape | Shrink default fiber stack from 64 MiB (overflow-driven sizing or tiered default; the 64 MiB comment in make_fiber documents *why* it's big — genexpr/listcomp nesting — so measure before shrinking); `Task` park-keys extended for a future event loop (asyncio remains OUT until T4 per spec 05 S-05-19 — v1 §2.1 and §6.5 stand: the asyncio half of the orchestrator's 02.10 gate text is vacuous pre-T4 and should be annotated) | 02.10d green | 10k-task RSS improves measurably vs 02.10b baseline (trend-only per tests/README bench rule); generator/controlflow corpus (G001 family) green at the new stack size; deep-recursion probe (recursion_limit 1000, frame.rs) green |

Dependency chain: 02.09 → a → b → c → d → e, strictly serial (each exit gate
is the next entry gate). 02.10c may co-dispatch with the MMTk stage-2 swap
leaf if the mmtk spike lands first, since both touch the binding's `Scanning`
impl — coordinate file ownership (gcsupport.rs vs the new mmtk-antipythany
crate) before dispatch.

## 5. Risks (delta over v1 §5; v1's determinism/refactor/regression risks stand)

- **R1 (new, dominant): 64 MiB fiber stacks break the 10k-task memory gate.**
  10k started fibers × 64 MiB = 640 GiB of *virtual* mapping. On Linux/macOS
  anonymous mmap is lazily committed so RSS stays near touched-page size, but
  (i) address-space pressure and per-VMA kernel costs are real at 10k VMAs,
  (ii) any future `MAP_NORESERVE`-less platform or strict-overcommit setting
  fails at spawn, (iii) guard-page faults from deep tier-0 recursion are the
  failure mode the 64 MiB was chosen to avoid. Mitigations, in order: most
  parked tasks in the 10k case are *unstarted* (spec only, no fiber — M1's
  steal-at-birth makes unstarted the natural parked state for never-run
  tasks); tier/measure stack sizes (02.10e); cap concurrently-*started*
  fibers if measurement demands. The gate must measure RSS AND virtual, vs
  the serial reference (v1 §6.4).
- **R2 (new): fiber/GC coupling predates the scheduler.** Suspended generator
  stacks are already unscanned roots under the stage-1 collector — they
  survive today only because the subtract-refs invariant sees the `Rc` clones
  held on the fiber stack as external references (gc-mmtk-swap §1: "any
  external Rc clone — including native-stack temporaries — shows up as
  sc > internal + 1"). Under MMTk there is no strong-count to observe
  (gc-mmtk-swap §4.1), so 02.10c's conservative fiber-slab scan is not
  optional — it is required the moment the MMTk swap lands, *even without the
  scheduler*. Flag this to the MMTk stage-2 leaf: generator fibers exist now;
  the swap's root shim must include them.
- **R3 (new): pinning is user-visible under compensation-cap exhaustion.** If
  all P workers are pinned by native-blocking tasks and the compensation cap
  is hit, new tasks stall — Loom's exact pinning pathology
  (cr.openjdk.org State of Loom "Pinning"; -Djdk.tracePinnedThreads as their
  diagnostic). Provide an equivalent diagnostic (env-flag trace of
  park/promote decisions) from 02.10b onward, and size the cap from the
  corpus's worst native-blocking case.
- **R4: T2 flux.** generators.rs and interp.rs are under active sibling edit
  (pytest run-phase debugging, NEXT-STEPS.md Wave-9/10). 02.10a's entry gate
  requires that work committed and green; do not dispatch into the dirty file
  set (master.md Test Gate DELTA rule).
- R5-R9: v1 §5's determinism (steal-by-half timing vs 3-run gate),
  statement-atomicity re-audit of the GIL-retry loops, poll-cost, and
  comparison-class notes carry over unchanged; the "resumable-frame refactor"
  risk is *retired* — the codebase chose fibers over reified continuations.

## 6. Recommendation (headline)

**Work-stealing M:N with thread-pinned stackful-fiber tasks (M1), steal
restricted to unstarted tasks, Loom-style compensation threads for
native-blocking, and cooperative statement/back-edge preemption sharing one
poll with the MMTk STW handshake — no signal-driven stack switching, ever.**
Why: it is the only option that reuses the suspension mechanism the runtime
already ships and already depends on for T1/T2 (generators.rs fibers); it
needs zero new unsafe (the `!Send` boundary becomes a *scheduling rule*
instead of a transmute); its GC story is exactly the Go-1.14-proven
conservative-frame-scan compromise, compatible with the locked MarkSweep-first
plan; and its preemption story matches every shipping cooperative runtime of
record (Tokio budgets, Loom no-time-sharing) while our statement-granular
poll gives *better* safepoint latency bounds than Go's async preemption.

## References

Ours (files read on the 2026-10-30 tree):
- `crates/ap-run/src/generators.rs` (module header, thread_locals, gen_yield,
  resume_live, make_fiber — regions, not lines: sibling edits in flight)
- `crates/ap-run/src/interp.rs` (GenFrame enum + PENDING_GEN_ERROR region;
  touch_line/stmt_ordinal :624-652; yield/fiber interplay :2633-2705 region)
- `crates/ap-run/src/threading.rs` (GIL v1 header :16-28, current_os_tid :77,
  LOCALS :184-187, SWITCH_INTERVAL :190-191, spawn+transmute :557-566)
- `crates/ap-run/src/threading_call.rs` (release/acquire_gil pairs — 54
  occurrences, grep count)
- `crates/ap-gc/src/gcmod.rs` (SCAN_CONTENDED :472-499, contention-abort rule
  :865-873, GARBAGE_NOW seq ordering :506-521)
- `docs/02-10-scheduler-design.md` (v1), `docs/gc-mmtk-swap.md`,
  `docs/pon-study-tier1.md` (safepoint/OSR/stackmap regions),
  `docs/t1-gate.md`, `NEXT-STEPS.md` (Wave-9 fiber thread-local notes,
  asyncio two-drivers gap), `plans/antipythany/master.md` (Design intent,
  Amendment A-3, tier table, test protocol),
  `plans/antipythany/02-native-backend/orchestrator.md` (:88-140),
  `plans/antipythany/02-native-backend/PROGRESS.md` (02.07 GIL-v1 design +
  deferred list, :741-801)
- Vendored: `corosensei-0.3.4` crate source (`coroutine.rs` "# Send" docs +
  !Send test; `stack/unix.rs` mmap+guard page; `stack/mod.rs` guard contract)
  — matches `Cargo.lock` corosensei 0.3.4 and THIRD_PARTY_NOTICES (MIT OR
  Apache-2.0).

External (accessed 2026-10-30):
- Go non-cooperative preemption: go.googlesource.com/proposal design/24543
  (cooperative history, 7.8% loop-preemption cost, SIGURG rationale,
  conservative-inner-frame sub-proposal); go.dev/src/runtime/signal_unix.go
  (sigPreempt/doSigPreempt/isAsyncSafePoint); go.dev/src/runtime/preempt.go
- golang.design/under-the-hood en/part3concurrency ch09 (§9.6 signals, §9.7
  async preemption — asyncPreempt injection + conservative parent-frame scan)
- Tokio cooperative budget: tokio.rs/blog/2020-04-preemption (128-op budget,
  cannot-preempt honesty, spawn_blocking); docs.rs/tokio task::coop
  (consume_budget)
- Java Loom: cr.openjdk.org/~rpressler/loom/loom/sol1_part1.html (State of
  Loom — M:N, carrier mount/unmount, ForkJoinPool work-stealing scheduler,
  pinning, no time-slice preemption in any JDK scheduler); JEP 444 quote via
  news.ycombinator.com/item?id=37570723; mail.openjdk.org loom-dev
  2022-April/004145 (Pressler on "not cooperative, not preemptive-by-time");
  rockthejvm.com + javaperformancetuning.com (carrier pool ≤256 compensation,
  pinned-thread diagnostics)
- Rust unstable coroutines trait docs (doc.rust-lang.org unstable-book
  coroutines — Send rules for language coroutines; context for why a
  stackless-executor (M3) future remains plausible but unbuilt)

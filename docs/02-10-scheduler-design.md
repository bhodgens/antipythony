# 02.10 — Go-style runtime scheduler (research design note)

Status: RESEARCH ONLY (no code). Leaf: `plans/antipythany/02-native-backend/orchestrator.md`
§02.10 — implementation only after 02.09 (no-GIL swap) lands; this note is the
implementer brief for that future leaf. Every current-code claim cites a file:line
read on the 2026-10-22 tree.

## 1. Problem: what "Go-like" means here

Owner intent (master.md "Design intent", lines 9-19): not merely removing the GIL
but replacing it with *something better in the direction of Go's runtime* — real
parallel execution, improved memory behavior, performance above CPython.

The three layers and what each contributes:

| Layer | What it gives | Leaf |
| --- | --- | --- |
| GIL v1 | correctness of threaded Python; rendezvous concurrency; zero parallelism | 02.07 (shipped, `crates/ap-run/src/threading.rs:62-133`) |
| No-GIL | N OS threads executing Python truly in parallel (1:1); CPU scaling >= 3x gate | 02.09 (gated) |
| Scheduler (M:N) | multiplex *many cheap Python tasks* onto *N worker threads*: work stealing, parking on blocking ops without burning an OS thread, bounded memory for 10k tasks | 02.10 (this leaf) |

02.09 without 02.10 is already CPython-freethreaded-class: every `threading.Thread`
is an OS thread (today `thread_start` spawns `std::thread` with a 64 MiB stack,
threading.rs:562-565). That does not scale to 10k Python threads (each costs an OS
thread + 64 MiB reservation) and nothing resembles Go's "goroutine per call, cheap".
02.09 + 02.10 is the Go-style shape: a P (parallelism permit) count, per-P run
queues, work stealing, and stack-switching (or resume-able) tasks.

Reference for the target shape: Go's GMP model — G (goroutine, stackful task,
2 KiB initial contiguous stack grown by copy), M (OS thread), P (permit + local
run queue + local caches; count = GOMAXPROCS; empty Ps steal half of a victim P's
queue; scheduler code runs on each M's dedicated g0 stack)
(golang.design/under-the-hood ch. 9; go.dev/src/runtime/HACKING "Stacks"). We
will NOT replicate the copied-stack machinery (§3a); we replicate the GMP
*accounting* — permits, local queues, steal-by-half, park/unpark.

## 2. Python-visible semantics that constrain the scheduler

1. **asyncio is OUT-of-v1** (spec 05, S-05-19, docs/spec/05-builtins-stdlib-v1.md:212-214:
   "asyncio … full event loop lands with the T4 plan; OUT until then"). The
   executor has no await: `Expr::Await` raises (`interp.rs:2112-2114`) and
   `async def` returns a no-op coroutine stub (`interp.rs:3719-3735`). So the
   leaf text's "asyncio loop + threading bridge" scope narrows for v1 to
   **threading-on-tasks**; the scheduler must be *shaped* so a future event loop
   can submit tasks, but the asyncio-corpus half of the gate is vacuous until T4.
2. **Thread-local storage is keyed by OS tid today**: `LOCALS: HashMap<addr,
   HashMap<os_tid, dict>>` (threading.rs:184-187), `local_get/local_set` use
   `current_os_tid()` (threading.rs:965-982). Under M:N, two Python threads share
   one OS thread, so this key becomes *wrong*: the scheduler leaf must re-key TLS
   on the **task's logical thread identity** (`ThreadEntry.ident`, minted at
   threading.rs:255-257), not the OS tid. Same for `GIL_HELD`/`GIL_DEPTH`
   thread-locals (threading.rs:72-75) — those disappear with 02.09 — and for
   `WORKER_INTERP`, a single-slot global that already assumes one worker
   interpreter at a time (threading.rs:200-202; written at every
   `worker_bootstrap`, threading.rs:606).
3. **sys.setswitchinterval is stored but inert** (threading.rs:26-28, 191,
   1053-1055; CPython surface accepted, no scheduling effect). CPython semantics:
   it is the *preemption quantum* for the GIL-recheck loop. The scheduler is the
   natural place to give it meaning: quantized cooperative switch between tasks.
4. **Default thread naming is a process-wide monotonic counter** ("Thread-N
   (target)", threading.rs:30-31, 181, 1068-1079) and Thread repr is pinned
   (T108). Task identity must preserve the counter discipline regardless of
   which worker runs the task — do NOT derive names from worker slots.
5. **Determinism, not ordering, is what the corpus pins.** Every threading case
   is run THREE times and must be byte-identical across runs
   (`tests/t10_threading/test_t10_threading.py:15-16, 179-190`), and the cases
   are constructed to be order-independent: T101 pins a *sorted* aggregate behind
   a Barrier rendezvous ("Determinism: no sleep",
   `tests/corpus/threading/T101-start-join-completion-order.py:4-6`); T121 pins
   only deterministic result bytes, timings measured-not-asserted
   (`tests/corpus/threading/T121-scaling-parallel-cpu.py:1-7`). So the scheduler
   may freely reorder task interleavings — the goldens only pin
   *run-to-run stability of the aggregate*, which byte-exactness of
   lock-protected sections provides. A scheduler that introduces *new*
   nondeterminism inside a single run's aggregate (e.g. a torn dict write
   reaching `print`) fails; a scheduler that changes *which task runs first*
   does not.

## 3. Task-primitive options (the leaf's required tradeoff vs the generator design)

### 3.0 What ap-run's "generator" actually is — the critical fact

The comment header says "Generators (explicit state machine per the pon study
note)" (interp.rs:4134-4136) and `GenFrame::Fn` stores
`entry/func_obj/args/kwargs` (interp.rs:4150-4155) — but the implementation is
neither a state machine nor a suspended stack: **the first pull runs the body to
completion, collecting every yield into a Vec sink** (interp.rs:4147-4149
"v0 runs its body collecting ALL yields into a stream on first pull",
materialize path interp.rs:4182-4278). The machinery is: `yield` raises
`Flow::YieldBack`, `exec_block` catches it per statement and *continues the same
native call* (interp.rs:436-443, 449-454). There is **no suspension primitive in
ap-run at all** — no saved frame, no resume point, no per-generator stack. The
whole interpreter state lives in one `Rc<Interpreter>` graph with `RefCell`
interiors (!Send/!Sync by construction — GIL v1 papers over this with a
`transmute` of the worker closure, threading.rs:449-461 module header 16-24).

Consequence for this leaf: option (b) "continuations" is not "cheaper than
stackful threads because generators already do it" — generators do *neither*,
they materialize. Building a resumable generator and building a task suspension
are the *same* engineering problem. Whoever solves one gets the other; the
scheduler leaf should ride on, not duplicate, that fix (see §4/§6).

### 3a. Stackful green threads (copied native stacks per task)

The Go model proper: each task owns a contiguous machine stack; switch =
save/restore SP+PC. Requirements we would have to add:
- per-task heap stacks with growth-by-relocation (Go: `stackMin = 2048` initial,
  contig stacks copied on overflow, g0 system stack for the scheduler itself —
  under-the-hood 9.3.3, 14.1);
- an `mcall`-style switch, i.e. assembly or a crate like
  `async-std`-era green pthreads — unsafe in stable Rust, and every relocation
  invalidates conservative-stack GC roots;
- **root interaction**: gc-mmtk-swap §4 (docs/gc-mmtk-swap.md:62-84) plans roots
  from *explicit* structures + a shadow stack, not machine-stack scanning — but
  the subtract-refs invariant that the stage-1 collector relies on covers
  native-stack temporaries for free (gc-mmtk-swap.md:22-25). A copied task stack
  breaks "temporaries ride the native stack": relocation must rewrite or
  conservatively re-scan the task's stack slab at stop-the-world. This is the
  single largest risk of (a).
- **cost**: 2-8 KiB/task is cheap *for Go* because Go frames are compiler-sized;
  our `Frame` is 12+ heap fields plus `HashMap` locals (frame.rs:19-51), so a
  task is heavy *twice* — machine stack AND per-frame Rc<Frame> trees on the
  shared heap. The Python call depth (recursion_limit 1000, frame.rs:288, 422-432)
  × multi-MB Frame graphs makes the "bounded memory 10k tasks" gate hard.
- **!Send discipline**: task structs would carry `Rc<Interpreter>`; the audited
  transmute (threading.rs:557-561) is currently OK because the GIL serializes.
  M:N with real parallel Ps requires the 02.09 heap audit FIRST (02.09 leaf text:
  per-object state thread-confined or atomic-wrapped, orchestrator.md:92-96) —
  same precondition as every option, but (a) maximizes the surface exposed to
  parallel Ps at once.

Verdict: highest fidelity to "Go-style", highest unsafe-Rust + GC-relocation
cost. Rejected as the first move; kept as the only shape that would ever
preempt arbitrary C-like native stretches we don't have.

### 3b. Delimited continuations / resume-able executor (async-style transform)

Make `exec_block`/`eval_expr` suspendable at statement boundaries: the eval
loop's "stack" becomes an explicit task-local `Vec<PendingWork>` (frames +
continuation points) living on the GC heap; yield/resume is a pointer swap, no
machine stacks. Go-parity on *memory* (heap-allocated frames, growable), and
preemption lands exactly where we already have the hook: statement granularity,
which is also the granularity at which GIL v1 rotates (threading.rs:188-198
"writes are statement-atomic").
- **What it costs**: the Flow-based control convention (return `Err(Flow)`,
  catch in loops — interp.rs:436-446 for blocks; the same pattern for loops,
  try/except, `with`) must be reified per statement. Today `exec_block` runs a
  whole `Vec<Stmt>` to completion; suspending means each compound statement
  (for/while/try/with/if) needs an explicit continuation record. That is a
  pervasive but *mechanical* refactor of interp.rs (7 717 lines, statement eval
  in exec_stmt at interp.rs:498).
- **Bonus convergence**: this is exactly what real generators and coroutines
  (await) need. The v0 materialization shortcut is *documented as observable-
  divergence* already (interp.rs:4147-4149, 2071-2073 "single-threaded v0, no
  observable difference for the corpus") — and it stops being true the moment
  two threads pull generators concurrently, which 02.09 makes legal. So the
  resumable-frame work is **forced** by 02.09+T1-scrapy ("generators …
  deferreds-style async" is T1's stress column, master.md:69). (b) lets the
  scheduler ride that mandatory fix instead of adding a second suspension
  mechanism.
- **GC**: frames are heap `Rc<Frame>`/`Gc` already — task stacks become an
  ordinary root object (`Task.stack: Vec<FrameRef>` registered as a root dict),
  fully compatible with gc-mmtk-swap §4's explicit-root plan. No conservative
  scan of task state at all.
- **Preemption point**: a poll at the existing per-statement counter
  (`stmt_ordinal` advance, interp.rs:~468-480 `touch_line`) against a
  switchinterval-derived budget + the MMTk stop-the-world handshake flag
  (pon-study-tier1.md:135-141 loop-header poll + acknowledge; MMTk gives us the
  same handshake model, gc-mmtk-swap.md:62-84). Cooperative-but-preemptive:
  a task *cannot* run more than one statement without checking, so safepoint
  latency is bounded by one statement — strictly better than Go's asynchronous
  preemption for GC pause bounds, and it is precisely the orchestrator's
  "preemption at safepoints only" (orchestrator.md:120).

### 3c. 1:1 OS threads + work stealing only at the GC/alloc layer

Keep 02.09's shape (Thread == OS thread), add a `P`-count permit (Go's
GOMAXPROCS bound), per-P *task* queues only for GC marking assist + alloc
buffers, and steal at that layer. Cheapest: threading.rs's worker machinery is
already per-OS-thread; the GC registry drain channel
(gcsupport.rs:1590-1610 `install_worker_thread`) generalizes to per-P channels
naturally. But it delivers no 10k-task story (each Python thread is still an OS
thread + 64 MiB stack reservation, threading.rs:564), no park/unpark savings,
and is not "Go-style" in any user-visible sense. Good *foundation*, not the leaf.

### Which does 02.09's design already assume?

02.09's shape is 1:1 with the GIL removed (orchestrator.md:88-113): it offers
"per-thread nursery/heap OR shared heap + precise root maps"; the doc-consistent
minimal path given gc-mmtk-swap §4 (explicit roots + shadow stack; roots are the
shared `Rc` graph, frame locals covered by subtract-refs/shadow stack) is the
**shared heap + root-set** variant — because per-thread heaps would require the
cross-thread Weak bridge that threading.rs's module header explicitly rejected
(threading.rs:16-24). The 64 MiB OS-thread + shared-graph model is exactly
option (c). So 02.09 lands (c); (b) is a *delta* on top of (c) (same heaps, same
roots; new: explicit per-task frame vector + statement poll); (a) is a *second*
delta on top of (b) (machine stacks; conservative-scan redesign).

## 4. Recommended skeleton and migration path

**Recommendation: (c) as the substrate (inherited from 02.09), then (b) —
resume-able, explicitly-stacked tasks with statement-granularity safepoint
polling — as the 02.10 increment. Not (a).** Rationale: (b) reuses the
*mandatory* resumable-generator/coroutine fix, keeps every GC root inside the
gc-mmtk-swap §4 explicit-root model, makes the 10k-task bounded-memory gate
trivially true (a parked task = a Vec of frame handles + wait token, no OS
thread), and gives strictly bounded preemption latency at the cost of a
mechanical interp.rs refactor. (a) buys nothing Python-visible over (b) —
Python has no `unsafe` mid-statement spans to preempt.

Structures (implementation leaf, all new code in ap-run):

```
scheduler.rs (new)
  struct Permit          // one P: count from APX_GOMAXPROCS / available_cores
  struct RunQueue        // per-P deque of Rc<Task>; steal-by-half on empty
  struct Task            // { thread_ident, state, stack: Vec<FrameHandle>,
                         //   pc cursor, locals TLS map ref, parked_on: ParkKey }
  enum TaskState         // Runnable | Running(P) | Parked(ParkKey) | Done
  struct ParkKey         // reuses the PRIMS addr space: LockPrim/RLockPrim/
                         // CondPrim/EventPrim/SemPrim/BarrierPrim/QueuePrim
                         // condvars (threading.rs:289-346) become park tokens
  fn schedule_on(p)      // pop local; else global queue; else steal half
  fn yield_to_safepoint  // statement-budget check + STW handshake ack
```

- **Blocking ops become park/unpark, verbatim transplant of the GIL release
  list**: every `release_gil(); … ; acquire_gil()` pair in threading_call.rs
  (join 608-610; lock acquire ~719-757; cond wait ~809-832; event wait ~893-921;
  sem ~1090-1094; barrier ~1158-1182; queue get/put ~1224-1258, ~1301-1361,
  ~1399-1417; module header 4-7) maps 1:1 onto `park(ParkKey)` /
  `unpark(task)` — the *set* of switch points is already audited and pinned by
  T101-T115. This is the migration superpower: 02.07 chose blocking-op
  boundaries as switch points; 02.10 keeps the same list and changes only what
  "blocked" costs (a parked Task instead of a blocked OS thread).
- **OS threads**: workers become a fixed pool sized `#Permits + max(parked
  count, native-blocking headroom)`; the 64 MiB-per-Thread spawn
  (threading.rs:562-565) is replaced by pool submission. `thread_start` builds
  a Task and pushes to the global queue instead of `std::thread::Builder::spawn`.
- **TLS re-key** (§2.2): LOCALS inner map keyed by `ident` not os-tid;
  `WORKER_INTERP` single-slot deleted — Task carries its interp handle (post-
  02.09, the graph is Send-safe by audit, so the transmute goes away too).
- **switchinterval**: statement-budget for `yield_to_safepoint` (it finally
  *means* something; accept-side code unchanged, threading.rs:1053-1055).
- **asyncio-ready shape**: `Task` == what an event loop would submit; a future
  T4 loop adds `Task::await_on(future)` park keys without touching the queues.

Migration path (each step independently green):

1. **02.09 lands** (prerequisite): no-GIL, shared heap + explicit roots,
   T121 >= 3x green. Scheduler work is parked until then (orchestrator.md:124,
   139-140).
2. **2a — pool + permit substrate** ((c)-ify): replace per-Thread OS spawn with
   fixed worker pool over RunQueues, GOMAXPROCS-style permit count, steal-half.
   Tasks are still 1 OS-thread-slot per Python thread *by schedule*, but the
   plumbing exists. Gate: full threading corpus + 3-run determinism green.
3. **2b — resumable frames** (the shared generator fix): explicit
   `Task.stack: Vec<FrameHandle>` + statement-cursor suspension; generators
   stop materializing (kills the documented single-threaded divergence,
   interp.rs:2071-2073, 4147-4149). Gate: generator corpus + T10x green; new
   generator interleave cases added (corpus only grows).
4. **2c — park/unpark**: blocking-op list transplant (§above); Python threads
   are now Tasks. Gate: **the 02.10 gate proper** — asyncio slice is vacuous
   (S-05-19), threading corpus green under scheduler; 10k-spawn case completes
   with bounded memory (pool size fixed, Task ≈ a few KB of frames — vs ~64 MiB
   reservation × 10 000 today); T121 scaling stays green.
5. **2d (optional) — time-slice preemption** at the same poll so a CPU hog
   yields at switchinterval, not just at blocking ops.

File-ownership lines for the implementation leaf: `crates/ap-run/src/
scheduler.rs` (new), `interp.rs` (exec_block/exec_stmt poll + resumable
suspension), `threading.rs` (thread_start → pool submit; LOCALS re-key; drop
GIL statics left by 02.09), `threading_call.rs` (release/acquire pairs →
park/unpark), `gcsupport.rs` (per-P registration channels, generalizing
install_worker_thread, gcsupport.rs:1600-1610), `frame.rs` (Frame handles
addressable from Task.stack). `tests/t10_threading` + new
`tests/corpus/threading/T1xx-10k-tasks.py` case authored by the T-track agent,
never by the implementer.

## 5. Risks

- **Byte-exact interleavings vs preemption**: low, because the corpus is
  built order-independent + 3-run-pinned (§2.5) — *provided* statement-atomicity
  of globals writes holds after 02.09 (it is currently *enforced by the GIL*,
  frame.rs:188-198's retry loop assumes statement-atomic writes and a held-borrow
  clears "on the next scheduling turn"). Post-GIL the retry loops (interp.rs:471,
  486, 1221, 4723, 4736) must be re-audited: they are latent scheduler
  dependencies. Any case where a *sorted aggregate* changes means we introduced
  a torn write, not a schedule change — fail loud, don't re-pin.
- **Determinism across runs**: steal-by-half makes *timing* run-dependent; the
  3-run gate (test_t10_threading.py:179-190) tolerates that only because cases
  avoid sleep-ordered observation. Keep the rule: new threading cases must be
  order-independent by construction (the T101 header comment is the template).
  Never enable the scheduler's switchinterval randomization before the corpus
  is checked for sleep-based ordering.
- **GC safepoint latency vs interpreter speed**: statement-granularity polling
  means a poll check per statement — `touch_line`-adjacent, same budget the
  stmt_ordinal bookkeeping already pays (interp.rs:468-480). An atomic-load
  handshake flag (pon pattern, pon-study-tier1.md:135-141) is ~1 ns; against a
  tree-walking tier-0 executor (statement cost ~µs) that is noise. The real
  cost is suspension granularity in 2b, not the poll.
- **Resumable-frame refactor regressions**: `Flow::YieldBack` catching is
  today the *only* mid-block continuation precedent (interp.rs:442); loops,
  try/except, with all unwind via Rust `?`. Reifying cursors for every compound
  statement is where bugs will live; the generator + contextlib corpus slices
  and the scrapy deferreds T1 run are the tripwires.
- **The Rust ecosystem has no stable safe stack-switch** — a point in favor of
  (b), which needs none. If a future leaf ever wants (a) (preempting long
  native stretches — e.g. regex/backtrack in `sre.rs`, 2 698 lines of native
  loop), revisit then; today's native loops are all under our control and can
  poll too.
- **Comparison-class sanity**: Nuitka keeps the GIL (threads = OS threads; no
  scheduler layer). RustPython has no threading scheduler either (GIL-like
  single interpreter). pon's tier-1 is a JIT, not a task scheduler — its
  transferable part is the safepoint/handshake protocol we reuse for
  `yield_to_safepoint` (pon-study-tier1.md:135-160). Go is the only real
  reference; we copy its *accounting* (permit + local queue + steal-half +
  park tokens), not its *stacks*.

## 6. Open questions for the owner (with recommendations)

1. **Do we hold 02.10 until 02.09 lands, or build 2b (resumable frames) early
   as a standalone generator fix?** Recommend: split — dispatch 2b now as
   "generator materialization → resumable" (it independently removes a
   documented divergence and unblocks T4-asyncio prep); queue 2a/2c/2d after
   02.09. (The leaf text already allows "implementation only after 02.09" —
   2b is arguably not 02.10 implementation but a T1-adjacent correctness fix.)
2. **Shared heap vs per-P heaps after 02.09?** Recommend: shared heap + explicit
   root sets (gc-mmtk-swap §4 trajectory). Per-P heaps would force moving
   Python objects between Ps on task handoff — incompatible with (b)'s
   heap-resident Task.stack unless objects are pinned; only Go-style stacks
   (a) pair cleanly with per-P caches, and we rejected (a).
3. **Should `threading.current_thread().ident` remain distinct from any OS
   thread after M:N?** Recommend: yes — keep minted `ident` (threading.rs:255)
   as the logical key everywhere (TLS, naming, repr), and make `native_id`
   a "which worker is running me right now" query whose value may *change*
   between statements (CPython-free divergence to document in compat-contract).
4. **Gate hardware**: the 10k-task bounded-memory check measures RSS of the
   runner. Recommend pinning it as a ratio against the serial 10k-thread
   reference run, mask-timing style (tests/README rule: bench artifacts are
   trend-only), so CI runners of different RAM don't flake.
5. **asyncio gate text**: orchestrator.md:122 says "asyncio + threading corpus
   slices green under the scheduler". Recommend: annotate the orchestrator that
   the asyncio half is vacuous pre-T4 (S-05-19) so the leaf doesn't get marked
   incomplete for a slice that cannot exist yet — or pre-stage an await-raising
   corpus case (interp.rs:2112) as the asyncio stand-in for v1.

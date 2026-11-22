# 02.10 impl design — stages 2b (explicit task stacks + statement poll) and 2c (park/unpark + 10k-task gate)

Status: IMPLEMENTATION-READY DESIGN (docs only, no code). Expands
`docs/02-10-scheduler-design.md` §4 (v1) as revised by
`docs/02-10-scheduler-design-v2.md` (fiber-era, M1 = thread-pinned stackful-fiber
tasks). Covers stages **2b** and **2c** of the staged plan below; stage 2a
(permit/pool substrate) is summarized only as the compatibility target. Every
current-code claim cites file:line read on **main @ 5e8ccac** (2026-11-20).

---

## 0. Where stage 2a actually stands (dependency status)

`plans/antipythony/02-native-backend/NEXT-0210A.md` does **not exist** on this
tree — no 2a scheduler stage has landed as such. What HAS landed is the
suspension primitive the whole design rides on (v1's open question 1 resolved
by history, v2 §0):

- **PEP-342 lazy generators as stackful fibers** — commit 7c0a645 (2026-10-25),
  now `crates/ap-run/src/generators.rs` (826 lines): `GenFiber =
  Coroutine<GenInput, GenOutput, (), PooledStack>` (generators.rs:54),
  `make_fiber` lazy-start (generators.rs:811-826), `gen_yield` suspend
  (generators.rs:133-186), `resume_live` (generators.rs:356-563).
- **Fiber stack sizing + pooling** — F1 (8 MB default reservation,
  `gen_stack_size()`, generators.rs:578-584; the 64 MiB era is gone) and E6
  (`PooledStack` size-classed thread-local free list, generators.rs:586-769;
  measured shallow-fiber commit ~330 KB, generators.rs:576 comment).
- **Fiber identity thread-locals** — `ACTIVE`/`PARKED`/`CURRENT`
  (generators.rs:74-92), process-atomic `NEXT_FIBER` minter (generators.rs:97,
  the 02.09a P1-rest fix). The design **depends on thread affinity** today
  (v2 §0 item 4) — 2b keeps that dependency and makes it a *scheduling rule*.
- **No-GIL substrate** — 02.09 landed: `Gc<T> = Arc<T>`, worker closures Send
  by construction (the `threading.rs:873-876` spawn carries no transmute; the
  comment at threading.rs:864-869 records the deletion), blocking ops park on
  primitive Mutex+Condvar (threading_call.rs:6 module header).

**Consequence for 2b/2c:** the "explicit task stacks" of 2b are NOT v1's
reified-continuation executor (v2 §1 M3 rejected it) — they are the *explicit
scheduler-side task records* (`Task`, §2 below) whose execution state is the
task's corosensei fiber, stealable only before first start (M1's
steal-at-birth). "Statement poll" is a new scheduler-side check at the existing
per-statement bookkeeping site. The stage numbering here matches the dispatch
(2b = task stacks + poll; 2c = park/unpark + 10k gate); v2's leaf table
(§4, 02.10a-e) maps onto it as: 2a = v2 02.10a (pool substrate, own leaf),
2b = v2 02.10a-poll + 02.10d-quantum fused, 2c = v2 02.10b + the memory-gate
half of 02.10e. The GC-side safepoint leaf (v2 02.10c) is NOT covered here
beyond the poll-word hook it will consume (§3.3).

---

## 1. Stage 2a compat contract (what 2b assumes exists)

> **02E RECONCILIATION (2026-11-21, verified on main @ 8e96a8a):** this section
> as written is FALSE on the landed tree. What 0210a actually landed (per
> NEXT-0210A.md) is the resumable-GENERAATOR fiber substrate only —
> `thread_start` still does `std::thread::Builder::new().stack_size(64 << 20)`
> per Python Thread (threading.rs:873-876 @ 8e96a8a), no worker pool, no
> RunQueue, no Permit exists, and `touch_line` is ALREADY the T121-fix
> lock-free shape (per-frame `line_track` cells; the three-Mutex §7 premise
> is stale — §7 items 3-4 are RETIRED as work items, 2b.3 reduces to
> verification + measurement). The 2b/2c implementer therefore folds the
> minimal 2a (worker pool + Task record + steal-at-birth, gate: threading
> corpus + 3-run determinism) INTO this leaf's 2b.1 commit as the substrate
> for the explicit Task/RunContext records; the §2 structures below apply
> verbatim on top of it. This note REPLACES the contract text above for
> implementation purposes.

2a (permit/pool substrate, per v2 §4 row 02.10a) delivers, and 2b codes
against:

1. `thread_start` no longer spawns a `std::thread` per Python Thread — it
   constructs a Task and enqueues it (today's spawn site:
   `threading.rs:873-876`, `std::thread::Builder::new().stack_size(64 << 20)`).
2. A fixed worker pool of `P = APX_GOMAXPROCS | available_parallelism()`
   OS threads, each owning a local `RunQueue` (steal-half of *unstarted* tasks
   only).
3. Worker bootstrap = today's `worker_bootstrap` (threading.rs:906-971)
   generalized: `install_worker_thread` (gcsupport.rs:1943-1956), interp
   install (threading.rs:943-944), body run, excepthook, `mark_done`,
   reclaim, uninstall — now in a loop over tasks instead of once per OS thread.
4. The main thread remains the pool's worker 0 and additionally runs the
   module body; `wait_for_non_daemons` (threading.rs:1091-1116) becomes
   "drain until no non-daemon Task is unfinished" (same done-flag mechanics,
   `ThreadEntry.done: Arc<AtomicBool>`, threading.rs:125).

2b/2c do NOT change the pool contract; they change what a Task *is* (explicit
suspended-execution record with a poll loop) and what "blocked" costs
(scheduler park instead of a blocked OS thread).

---

## 2. Stage 2b structures (field-level, compatible with the landed fiber shape)

New file `crates/ap-run/src/scheduler.rs`. Names in the landed code are cited
next to each field. Everything here is plain data + atomics — the `!Send`
fiber handles are held ONLY inside per-worker (thread-confined) slots, never
in the shared scheduler tables.

```rust
// ---- crates/ap-run/src/scheduler.rs (new) -------------------------------

/// Parallelism permit (Go's P / GOMAXPROCS accounting). 2a owns this type;
/// repeated here as the compat surface 2b codes against.
pub struct Permit {
    pub idx: u32,                     // worker slot index, 0 = main thread
    pub os_tid: AtomicU64,            // threading::current_os_tid() of the
                                      //   OS thread currently mounted here
                                      //   (threading.rs:86-106 mint)
    pub queue: RunQueue,              // local deque, below
}

/// Per-P local run queue. The deque holds Arc<Task> handles — SENDABLE,
/// because an unstarted task carries only its spec (no fiber). A started
/// task's fiber lives in the worker's MOUNTED table (thread-confined,
/// !Send); the shared structures only ever hold `TaskId`s for those.
pub struct RunQueue {
    inner: Mutex<VecDeque<Arc<Task>>>,   // unstarted (stealable) tasks
    steal_victim_half: fn(usize) -> usize, // steal-half policy (2a)
    parked: ParkedList,                  // 2c: home-P parked list, §4.3
}

/// What a task runs. Exactly the `GenFrame::Fn` / fiber-closure shape that
/// `make_fiber` (generators.rs:811-817) and `gen_fiber_body`
/// (generators.rs:194-200) already consume, plus the Thread-level pieces
/// `worker_bootstrap` carries today (threading.rs:906-913).
pub struct TaskSpec {
    pub target: Value,                    // ThreadEntry: run() callable
                                          //   (threading.rs:908, from
                                          //   thread_start's <target> attr,
                                          //   threading.rs:764-769)
    pub args: Vec<Value>,                 // thread_start <args> (threading.rs:791-794)
    pub kwargs: Vec<(String, Value)>,     // thread_start <kwargs> (threading.rs:795-805)
    pub entry: Option<FnEntry>,           // crate::frame::FnEntry (generators.rs:30)
                                          //   None for plain thread targets
                                          //   (call_value_pub path,
                                          //   threading.rs:951)
    pub addr: usize,                      // Thread PyObject addr (obj_addr,
                                          //   ThreadEntry, threading.rs:113)
    pub serial: i64,                      // ThreadEntry.serial (threading.rs:116,
                                          //   double-start key, threading.rs:716-726)
    pub daemon: bool,                     // ThreadEntry.daemon (threading.rs:118)
    pub timer_interval: Option<f64>,      // Timer <interval> (threading.rs:782-790);
                                          //   consumed by wait_cancellable
                                          //   BEFORE the body (threading.rs:918-941)
}

/// Logical thread identity, minted per Python Thread at start — the value
/// `new_ident_pub()` (threading.rs:471-473, NEXT_IDENT atomic
/// threading.rs:270) returns today. Becomes the TLS/lock-owner key in 2c (§5).
pub type Ident = i64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// In some RunQueue, fiber never started (stealable).
    Unstarted,
    /// Fiber started, currently executing on its home P.
    Running { home: u32 },
    /// Fiber started and suspended at a scheduler poll point (quantum
    /// expiry, 2b) or a blocking op (2c ParkKey). Resumes ONLY on `home`
    /// (!Send pinning, v2 §1 M1).
    Suspended { home: u32 },
    /// 2c: suspended at a blocking op, registered in the park table.
    Parked { home: u32, key: ParkKey },
    /// Body finished; done flag flipped (ThreadEntry.done protocol,
    /// threading.rs:120-125). Registry row kept until join/eviction.
    Done,
}

/// The scheduler's task record. Inline size target <= 256 B (§6 arithmetic).
pub struct Task {
    pub id: u64,                      // process-atomic mint (NEXT_FIBER style,
                                      //   generators.rs:97)
    pub ident: Ident,                 // minted at thread_start (threading.rs:807)
    pub name: String,                 // ThreadEntry.name (threading.rs:117;
                                      //   NAME_COUNTER discipline preserved,
                                      //   v1 §2.4 — never derived from worker)
    pub state: AtomicU8,              // TaskState as u8 discriminant; the
                                      //   payload (home/key) lives in the
                                      //   worker-confined MOUNTED/PARKED tables
    pub spec: TaskSpec,               // moved out on first start? NO — kept
                                      //   (bootstrap_main replays Timer
                                      //   interval; join needs addr/serial)
    pub done: Arc<AtomicBool>,        // ThreadEntry.done (threading.rs:125)
    // ---- 2b: quantum accounting (§3) ----
    pub stmt_budget: Cell<u32>,       // statements left this quantum
    pub quantum_expiry: Cell<Instant>,// derived from SWITCH_INTERVAL
    // ---- 2c: park bookkeeping ----
    pub park_seq: AtomicU64,          // wakeup epoch (lost-wakeup guard, §4.4)
}

/// A STARTED task's execution state. NEVER crosses a thread boundary —
/// lives only in the home worker's table. This is the type that makes the
/// `!Send` corosensei fiber sound under M:N (v2 §1 M2 rejection honored).
pub struct RunContext {
    pub fiber: GenFiber,              // generators.rs:54 type EXACTLY —
                                      //   Coroutine<GenInput, GenOutput, (), PooledStack>
    pub fiber_id: u64,                // NEXT_FIBER mint (generators.rs:818)
    pub super_ctx: Option<(Arc<ap_obj::classes::TypeObject>, Value, bool)>,
                                      // GenState.super_ctx (generators.rs:71),
                                      //   pushed/popped around each resume
                                      //   slice (generators.rs:493-517)
    pub started: bool,                // GenState.started (generators.rs:59)
}

/// The scheduler input handed back INTO a task fiber on resume. The
/// generator machinery keeps its own GenInput; the scheduler's protocol
/// wraps it — a thread target's fiber consumes SchedulerIn directly, a
/// generator's resume stays inside the task body and is untouched.
pub enum SchedulerIn {
    Start,                            // first resume: run the body
    Resume,                           // continue after quantum yield / park
    Cancelled,                        // Timer cancel surfaced as a raise
                                      //   (wait_cancellable semantics,
                                      //   threading.rs:987-1008)
}

pub enum SchedulerOut {
    YieldScheduler,                   // quantum expired (2b): state -> Suspended
    Park(ParkKey),                    // blocking op (2c): state -> Parked
    Finished,                         // body done -> Done + mark_done
}
```

**Thread targets vs generator fibers — the one suspension mechanism rule.**
A `threading.Thread` target runs inside a fiber built by the SAME
`make_fiber` machinery (a `TaskFiber = Coroutine<SchedulerIn, SchedulerOut,
(), PooledStack>`); generators created *inside* the task body keep their own
`GenFiber`s unchanged — nested fibers work exactly as nested generators do
today (ACTIVE/PARKED identity stacks, generators.rs:74-92). This is the
"explicit task stacks" of 2b: `Task` + `RunContext` are the explicit,
scheduler-visible record; the fiber machine stack remains the implicit
execution state, pinned to `home`.

---

## 3. The statement poll (2b): insertion points and checks

### 3.1 The poll word

```rust
/// Process-global safepoint word. ONE Acquire load per statement serves
/// both the quantum check and the future MMTk STW handshake (v2 §2.3;
/// pon pattern, docs/pon-study-tier1.md:135-141).
static SAFEPPOINT: AtomicU64 = AtomicU64::new(0);
// bit 0: STW requested (set by the future gc collector leaf, v2 02.10c)
// bit 1: global quantum-expiry broadcast (set by a timer thread OR checked
//        against per-task expiry; bit 1 is the coarse fast path)
```

`yield_to_safepoint(task) -> bool` (true = must suspend):
1. one `Relaxed` load of `SAFEPOINT`; if bit 0 → STW path (acknowledge and
   wait for resume — the future gc leaf owns the wait protocol; until it
   lands, bit 0 never sets and the check is dead-weight-free).
2. decrement `task.stmt_budget`; if zero → recompute
   `quantum_expiry = now + SWITCH_INTERVAL` (`SWITCH_INTERVAL` f64-bits
   atomic, threading.rs:278, setter threading.rs:1797; default 5 ms =
   CPython's `sys.getswitchinterval()` default) and suspend.
3. A statement in this runtime costs ~µs (tier-0 tree-walk), so the default
   quantum is ~5 000 statements; 1 ms ≈ 1 000. The budget check is one
   decrement + compare — noise next to the three Mutex acquisitions
   `touch_line` already pays (§3.2, and see §7 for why those three locks
   dominate).

### 3.2 Exact insertion points (line-cited @ 5e8ccac)

| # | Site | Lines | What to insert |
| --- | --- | --- | --- |
| P1 | `exec_stmt` entry, immediately after the existing bookkeeping call | `interp.rs:1091` (`touch_line(interp, Some(frame), stmt_source_line(interp, s));`) | `maybe_poll(interp, frame)` — the primary per-statement poll. Fused function: poll FIRST, then touch_line, so a suspended task never advances the global ordinal mid-quantum-check. |
| P2 | `touch_line` body | `interp.rs:1040-1083` (the three Mutex acquisitions: `stmt_ordinal` 1056-1064, `stmt_lines` 1067-1073, `frame_stack` 1074-1079) | NOT modified in 2b (behavior preserved); §7 sequences its lock-free refactor ahead of or with 2b. The poll rides the same statement granularity but is a separate check. |
| P3 | `run_loop_body` per-iteration head | `interp.rs:2094-2110` (`for s in body { match exec_stmt(...) }`) | Back-edge poll per loop iteration. Strictly redundant with P1 while P1 exists (every loop body statement passes through P1) — keep P3 as the ONLY poll if profiling shows P1's per-statement cost matters; Go pre-1.14 precedent: function-prologue/back-edge polling was adequate for scheduling (v2 §3). Default in 2b: P1 only. |
| P4 | `Stmt::Line` marker arm | `interp.rs:1097-1113` | No poll (marker statements are free); P1 already fired for it. |
| P5 | Generator suspension sites | `interp.rs:4085-4102` (`Expr::Yield` fiber arm: line-cell pop → `gen_yield` → re-push) and `gen_yield` itself (generators.rs:133-186) | The fiber suspension IS a poll point already: on every resume-in, reset `stmt_budget` (a fresh quantum for the resumer's slice). No new check needed — resume_live (generators.rs:356) becomes a second place that re-arms the budget. |
| P6 | Long native loops we own | `sre.rs` regex engine, mbc codec tables (docs §v1 §5 flagged it) | OUT of 2b scope; such stretches run on the compensation lane in 2c (§4.5), matching v2 §3's verdict. |
| P7 | Blocking-op sites | threading_call.rs (§4.2 table) | These become *cooperative* suspension points in 2c — the poll that matters there is the park itself, not the budget. |

**Poll semantics invariant (byte-exactness):** a poll may suspend a task only
at a statement boundary, where the interpreter holds NO RefCell borrow guard
across the poll (exec_stmt is between statements) and no primitive Mutex is
held (§4 enforces: every park site documents its held-locks set). This keeps
the "safepoint latency bounded by one statement" property (v1 §3b) and keeps
suspended fibers scannable state (a suspended-at-poll fiber has the same
shape as a suspended-at-yield fiber, which the stage-1 collector already
survives via subtract-refs; v2 R2 notes MMTk will need the conservative
fiber-slab scan regardless — that is the gc leaf's problem, flagged in v2
§2.2).

---

## 4. Stage 2c: park/unpark transplant

### 4.1 ParkKey — the union over the primitive address space

```rust
/// Park slot identity. Derived from the PRIMS table keys
/// (threading.rs:581-583) — one variant per blocking primitive kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParkKey {
    Lock(usize),        // LockPrim addr (threading.rs:512-515)
    RLock(usize),       // RLockPrim addr (threading.rs:523-526)
    Cond(usize),        // CondPrim addr (threading.rs:528-536)
    Event(usize),       // EventPrim addr (threading.rs:538-541)
    Sem(usize),         // SemPrim addr (threading.rs:543-548)
    Barrier(usize),     // BarrierPrim addr (threading.rs:550-558)
    QueueGet(usize),    // QueuePrim addr, not_empty waiters (threading.rs:560-569)
    QueuePut(usize),    // QueuePrim addr, not_full waiters
    QueueJoin(usize),   // QueuePrim addr, all_tasks_done waiters
    Join(ThreadKey),    // join_wait (threading.rs:1025-1089): keyed by
                        //   (obj_addr, serial) — the double-start-safe
                        //   identity (threading.rs:716-726)
    TimerCancel(usize, i64), // CancelEvent key (threading.rs:282-289)
    DaemonDrain,        // wait_for_non_daemons (threading.rs:1091)
}
```

### 4.2 The transplant: today's blocking sites → park sites

The v1 §4 migration superpower survives the 02.09 flip: the *set* of blocking
sites is unchanged, only the mechanism inside each changes. Today each site
blocks its OS worker thread on the primitive's Condvar (threading_call.rs:6
module header). In 2c each site instead calls `scheduler::park(key, || {
predicate-check-and-take }, deadline)`, which: registers the task under the
prim's state Mutex (§4.4), then suspends the fiber via the scheduler protocol
(`SchedulerOut::Park`), freeing the worker.

| Prim | Blocking site (threading_call.rs @ 5e8ccac) | Wakes whom | Unpark trigger site |
| --- | --- | --- | --- |
| Lock | `lock_acquire_blocking` :924-932 (cv.wait :927); timed arm :786 | ALL waiters (release notifies all, `lock_release` :1078-1087) | `lock_release` :1083-1085 — after `*locked=false`, unpark ALL `ParkKey::Lock(addr)` waiters; they re-race on resume |
| RLock | `rlock_acquire` :934-952 (cv.wait :945); timed :858 | ALL (release = full only; `rlock_release_full` :954-966 notifies all :964; `rlock_release_one` :1063-1076 notifies only on count 0 :1073) | `rlock_release_full` :961-964 / `rlock_release_one` :1070-1073 |
| Cond | `cond_wait` :1090-1132 (wait_timeout :1118; release-save :1096-1103, re-acquire :1130); `wait_for` loop :1013-1033 | notify(k) unparks exactly k (:1035-1048, waiters counter :1040-1042); notify_all unparks all (:1049-1057) | `cond_method` "notify"/"notify_all" :1035-1057 |
| Event | `event_wait` :1179-1207 (wait_timeout :1198) | ALL waiters (`set()` notify_all — same shape as Lock) | `event_method_kw` "set" arm |
| Semaphore | sem acquire loop :1256-1271 (wait_timeout :1265); release notify_one per permit :1281-1287 | exactly `n` waiters per release(n) | sem "release" arm :1274-1288 |
| Barrier | `barrier_method` "wait" :1300-1420 (fill/drain state machine :1310-1319, timeout _break :1405-1419) | ALL (both drain release and _break notify_all) | last party in / `abort()` / timeout _break |
| Queue | put :1666-1712 (not_full wait :1690), get :1713-1756 (not_empty wait :1743), join :1774-1783 (all_tasks_done :1779) | put wakes ONE getter (:1728 `not_empty.notify_one`); get wakes ONE putter (:1755); task_done-to-zero wakes ALL joiners (:1774-1783) | the matching opposite-side completions |

`join_wait`/`join_wait_flag` (threading.rs:1025-1089) and
`wait_for_non_daemons` (threading.rs:1091-1116) transplant the same way:
the joiner parks on `ParkKey::Join((addr, serial))` / `DaemonDrain`; the
waker is `mark_done` (threading.rs:964 call site) — unpark all waiters keyed
on that (addr, serial). Timer `wait_cancellable` (threading.rs:987-1008)
parks on `ParkKey::TimerCancel(addr, serial)`; `cancel_timer` unparks.

**Who wakes whom, mechanically:** the waker never touches a fiber. It (a)
mutates the prim state under the prim's own Mutex, (b) takes the parked
task IDs out of the prim's waiter slot in the PARK_TABLE (lock-ordered
INSIDE the prim Mutex — §4.4), (c) pushes them to each task's home-P
`parked` list (RunQueue.parked, §2) and notifies that P's condvar. The home
P, when its run queue is empty, drains its parked list and resumes those
fibers (`SchedulerIn::Resume`). Because a started task never migrates (M1),
the parked list push is `TaskId + park_seq` only — plain integers, Send —
while the fiber itself never leaves the home worker's MOUNTED table.

### 4.3 Compensation lane (native-blocking, unparkable)

Sites that block in native syscalls we do not control (file IO, subprocess,
the vendored-stdlib calls that today release the thread entirely) cannot
suspend a fiber mid-syscall. They promote: the task's fiber stays mounted on
its worker (Loom pinning), the pool spawns/activates a compensation worker up
to the cap (v2 §3: `P×4` default, tunable `APX_COMP_CAP`), and the pinned
task is marked `Running{home}` with a `pinned_native` flag so the diagnostic
(v2 R3: the `APX_SCHED_TRACE` park/promote trace) can report pinning. This
lane is 2c's answer to sre.rs-class stretches (P6).

### 4.4 Lost-wakeup analysis (the load-bearing protocol)

Classic lost wakeup: parker checks the predicate (false), then — before
registering — the waker flips the state and notifies; parker then sleeps
forever. The protocol that makes every row of §4.2 safe:

1. **Register under the prim's state Mutex.** `park(key, check, deadline)`
   FIRST locks the prim's state Mutex (the SAME mutex the waker's state
   mutation takes: `LockPrim.locked` :513, `RLockPrim.st` :524,
   `CondPrim.waiters`+backing-lock protocol :534, `EventPrim.st` :539,
   `SemPrim.value` :544, `BarrierPrim.st` :554, `QueuePrim.items`/`unfinished`
   :561-563). Under that Mutex it (a) runs `check` — if the predicate is now
   true it takes the resource and never parks; (b) otherwise inserts
   `(task_id, task.park_seq)` into `PARK_TABLE[key]`.
2. **Release the prim Mutex, THEN suspend.** Between (1b) and the suspend
   there is no window the waker can miss: any waker that flipped the state
   did so under the same Mutex, either before our `check` (we see it true)
   or after our registration (it finds us in `PARK_TABLE` and unparks us).
   Suspending after registration is safe because suspension itself loses no
   messages — the resume channel is the home-P parked list, and unpark
   always pushes there unconditionally.
3. **Waker order:** prim Mutex → state mutation → PARK_TABLE drain → parked-
   list push → notify. Lock order is globally `prim Mutex → PARK_TABLE →
   parked list`; no site ever takes them in reverse (audited per §4.2 rows;
   PARK_TABLE itself is one process-global Mutex — contention is per-prim-
   event, not per-statement).
4. **Spurious resume is harmless.** On `SchedulerIn::Resume` the fiber
   re-enters the prim method's loop (the `while` predicate loops already
   present at :926, :944, :1113, :1262-1271, :1310, :1684-1700, :1736-1752
   become resume-loop bodies), re-locks the prim Mutex, re-checks. A stale
   wakeup just re-parks. `task.park_seq` guards the corner where an old
   unpark (from a previous park on the same key by the same task) races a
   fresh registration: resume delivery checks the seq matches the
   registration it was minted for; mismatch = ignore (the fresh park will be
   woken by its own waker, since registration happened under the prim Mutex).
5. **Per-prim corner cases carried over from the 02.07-audited list:**
   - *Cond release-save ordering*: `cond_wait` must register on
     `ParkKey::Cond` BEFORE fully releasing the backing lock
     (`rlock_release_full` :1096-1097 / `lock_release` :1098-1099), the
     standard CV invariant — a notify between release and register would
     otherwise be lost. Today's code orders this correctly via
     cv.wait-atomically-releases; the transplant preserves the order by
     registering under `waiters` Mutex before release-save (:1104-1107
     increments `waiters` first — keep that as the registration step).
   - *notify(k) exact-count*: `CondPrim.waiters` (:534) already counts;
     unpark drains min(k, registered) entries and leaves the surplus
     registered. The post-resume decrement (:1123-1128) moves into the
     scheduler's resume path.
   - *Barrier generation*: unpark-all on generation change only; a waiter
     resuming into a NEW generation (`my_gen != st.2`) re-parks — the
     existing `wdone` check (:1414-1416) is exactly this predicate.
   - *Queue put/get pairing*: one notify_one per item (:1728, :1755)
     transplants to one unpark per state transition; surplus waiters stay
     parked (their resume-loop re-parks).
   - *Double-park prevention*: a task parked on `ParkKey::Lock(L)` cannot
     run code that parks on `Join(...)` — it's suspended. But a WAKER can
     hold one prim Mutex and unpark tasks whose resume path takes another
     prim Mutex — resume happens on the HOME P later (step 4.2), never
     inside the waker's lock scope, so no waker-side deadlock. This is the
     decisive advantage of deferred resume over direct resumption.
   - *Barrier action under M:N*: the last party runs the barrier `action`
     (call_value_pub at :1319-1330 region) while OTHER parties are parked on
     `ParkKey::Barrier` — under the scheduler this is safe (parked tasks
     hold no thread), whereas today it burns `parties-1` OS threads. No
     lock is held across the action call in the current code (the state
     transitions to draining before the action runs); preserved as-is.

### 4.5 What 2c does NOT transplant

- `std::thread::sleep`-class waits inside vendored stdlib that are bounded
  and rare — they keep blocking the worker (compensation lane absorbs).
- The stage-1 GC's collector pass (process-global COLLECT mutex,
  threading.rs:73-75) — collector-side, not mutator park.
- `STACK_POOL` (thread_local, generators.rs:627-630) stays thread-local by
  the pinning rule — regions never move between workers while a fiber is
  live on them; the pool's own comment already notes regions *could* move
  (generators.rs:599-601) but 2c does not rely on it.

---

## 5. LOCALS TLS re-key (OS tid → logical ident) + lock-owner re-key

### 5.1 The bug 2c must not ship

Under M:N, `current_os_tid()` (threading.rs:86-106) is the identity of the
*worker*, not the Python thread. Two consumers key per-thread state on it
today:

1. **LOCALS** — `LOCALS: HashMap<local_addr, HashMap<os_tid, Gc<PyDict>>>`
   (threading.rs:273-274); `local_get`/`local_set` (threading.rs:1432-1445)
   read/write via `current_os_tid()`. Under M:N two tasks sharing a worker
   would SHARE one threading.local namespace (silently wrong), and a task's
   namespace would follow its worker on preemption.
2. **RLock/Cond ownership** — `RLockOwner::Owned(u64)` holds an os_tid
   (threading.rs:517-526); `rlock_acquire`/`rlock_release_full`/
   `rlock_release_one`/`cond_owned` compare it against `current_os_tid()`
   (threading_call.rs:935-951, :954-966, :1063-1076, :906). Under M:N this
   is BROKEN two ways: (a) two tasks on one worker: task B "owns" task A's
   RLock (same tid) and re-enters it; (b) preemption mid-critical-section
   plus migration of the *home P's other tasks* is fine, but any future
   un-pin would orphan the owner. Re-key is mandatory, not optional.

### 5.2 The re-key design

New worker-scoped accessor, installed at task mount, cleared at unmount:

```rust
// scheduler.rs
thread_local! { static CURRENT_TASK_IDENT: Cell<Ident> = const { Cell::new(0) }; }
pub fn current_logical_ident() -> Ident;  // 0 = no task mounted (pure main)
```

- **LOCALS**: inner key becomes `Ident`. Migration path for existing rows:
  the only pre-2c producers are the main thread and 1:1 workers, whose
  idents are minted at `bootstrap_main` (threading.rs:392) and
  `thread_start` (threading.rs:807) respectively — introduce
  `LOCALS_LOGICAL` alongside `LOCALS`, switch `local_get`/`local_set`
  (threading.rs:1432-1445) to `current_logical_ident()` with a one-release
  cutover (no runtime dual-read: any probe that used threading.local ran
  under one key or the other, never mixed within a run). The eviction sweep
  (threading.rs:1636-1643 "LOCALS: outer keys absent from the registry")
  keys off the same registry walk — inner-key type change is invisible to
  it.
- **RLockOwner**: `Owned(u64 os_tid)` → `Owned(Ident)`. All four compare
  sites (§5.1 item 2) switch to `current_logical_ident()`. The main thread
  mounts ident 0→ its bootstrap ident before module body (module body can
  take RLocks — contextlib does). This is a **2c gate item**: the RLock
  corpus cases (T10x) fail loudly under M:N if any compare site is missed —
  a missed site under 1:1 workers is INVISIBLE (ident==worker's thread only
  task), which is exactly why the re-key lands WITH 2c, not before: 2c's
  corpus runs two tasks per worker and would catch it.
- **native_id** (`ThreadEntry.native_id`, threading.rs:127;
  `get_native_id_value` threading.rs:1422-1426): becomes "which worker is
  running me right now" — reads `Permit.idx → os_tid` of the home P at
  query time. v1 §6.3 recommendation adopted: may change between statements
  for stolen-then-started tasks... actually NO task migrates after start
  (M1), so `native_id` is stable per started task = its home P's tid; only
  Unstarted→Started transitions change it once. Document in
  docs/compat-contract.md: `threading.Thread.native_id` = home worker OS
  tid, assigned at first run, stable thereafter (CPython: the OS thread's
  tid — same stability, different lifetime; corpus only checks > 0 and
  self-equality, threading.rs:305-306 comment).
- **stamp_worker_identity** (threading.rs:307-348): retired in 2a — the
  placeholder-then-stamp dance exists because the spawner doesn't know the
  worker tid yet (threading.rs:808). With a pool, the home P stamps its own
  Permit.os_tid once; Task entries carry home P index instead of a tid.
- **CURRENT_INTERP** (interp.rs:11594, install at threading.rs:944): stays
  per-worker TLS (one shared Interpreter post-02.09) — unchanged; a mounted
  task inherits the worker's installed interp. WORKER_INTERP-as-single-slot
  is already gone (runtime_state.rs table keyed by interp addr,
  runtime_state.rs:13-32; the header's 02.09-b contract note is the
  trajectory 2a follows).

---

## 6. The 10k-task bounded-memory gate (2c exit arithmetic)

Task record size (aarch64, computed field-by-field):

- `Task` inline: 8 (id) + 8 (ident) + 24 (String name) + 1+pad (state) + 8
  (done Arc) + 4+pad (budget) + 8 (expiry) + 8 (park_seq) ≈ **72 B**
- `TaskSpec` inline: 32 (Value target) + 24 + 24 (Vecs, empty for the spawn
  shape) + 8 (Option<FnEntry>) + 8+8 (addr, serial) + 8 (timer) ≈ **112 B**
- Heap per task: name String ~24 B; args/kwargs payloads for the 10k shape
  (spawn-nothing closures) ~16 B → ~40 B
- **≈ 224 B/task inline+heap → 10 000 unstarted tasks ≈ 2.2 MB.** Negligible.

The dominant term is STARTED fibers' stack reservations (v2 R1):

| Configuration | VA @ 10k started | RSS @ 10k started |
| --- | --- | --- |
| Today: 10k OS threads × 64 MiB (threading.rs:875) | **625 GiB** + 10k kernel threads | ≥ touched pages × 10k |
| Started fibers × 8 MiB PooledStack (`gen_stack_size`, generators.rs:578-584) | **78 GiB** | ~3.2 GiB (330 KB touched/fiber, generators.rs:576) |
| Started fibers × 1 MiB (if the 02.10e tiering lands) | 9.8 GiB | same touched-page RSS |
| Unstarted tasks (spec only — M1 steal-at-birth) | 224 B each | ~0 |

**Why the gate is passable at 2c even before stack tiering:** the 10k-spawn
gate workload (T-track authored, order-independent per the T101 template) is
spawn-many, park-most: the M1 rule means a task that has never been
scheduled holds NO fiber and NO stack — 2c's park-on-first-blocking-op plus
steal-at-birth keeps concurrent-started-fiber count ≈ concurrency, not ≈ 10k.
For the pathological all-started case the gate MUST still hold, so 2c adds a
started-fiber admission control: `APX_MAX_STARTED` (default: unlimited —
measure first; the knob exists so the gate can be made red→green without a
code change if a corpus shape exceeds VA headroom).

**Gate form (v1 §6.4 + v2 R1):** measure BOTH RSS and VA of the runner for
the 10k case, expressed as a ratio against the serial 10k-thread reference
run on the same machine and session (trend-only per tests/README bench
rule). Pass: RSS ratio ≤ 0.5 of reference AND VA ≤ 64 GiB absolute
(headroom guard), OR — if the reference itself OOMs, which it will on most
CI boxes — the reference leg is recorded as skipped and the absolute RSS
budget (≤ 4 GiB at the 8 MiB default) is the gate. 3-run determinism and
T121 stay green as usual.

---

## 7. T121 interaction: does M:N change the serialization analysis?

**Finding being interfaced with** (plans/antipythony/02-native-backend/
T121-BISECT.md, 2026-11-19/20): per-statement line bookkeeping — `touch_line`
acquiring `stmt_ordinal` + `stmt_lines` + `frame_stack`, three process-global
Mutexes per executed statement (interp.rs:1056-1079) — serializes all workers
proportionally to WORK. Scaling is workload-invariant (~1.02-1.06x in
release regardless of N); reaching ≥3x requires making the bookkeeping
per-frame/thread-local and lock-free (their follow-up 1). The 3x reading was
only ever visible on the debug profile where per-statement compute dwarfs
lock cost.

**Does the scheduler change this? Direct answer: NO — and slightly for the
worse if 2b lands on top of the current bookkeeping.**

1. The serialization is per-executed-statement against shared globals. M:N
   executes the same statements, just from different tasks; the mutex
   traffic is identical in count and now interleaves across MORE logical
   threads (task switches don't reduce statement count). The T121 ceiling is
   a property of the bookkeeping, not the threading model.
2. The scheduler ADDS one per-statement check (the poll, §3). At one
   Relaxed load + decrement it is ~2 orders of magnitude below one uncontended
   Mutex acquisition — the poll itself is not the problem; the point of §3.1's
   single-word design is to not ADD a second per-statement lock.
3. **The scheduler is, however, the natural HOME of the T121 fix.** The
   bisect's required change — per-frame lock-free lineno bookkeeping — needs
   a per-mutator (not per-OS-thread) notion of "current frame chain" that
   other threads can read for f_back stubs (frame.rs:553's `frame_stack` is
   shared precisely because readers exist cross-thread). Under 2b, "the
   currently-executing statement" has an authoritative per-task home:
   `Task` + the mounted `RunContext`. The refactor becomes: move
   `stmt_ordinal` (interp.rs:605) and the ordinal table read (interp.rs:
   1067-1073) into per-task state, keep `frame_stack`/`py_frames` (frame.rs:
   553-559) as the only shared readers, and have traceback writers use
   `frame.last_line` (frame.rs:92-95 per-frame cell — already the G4-repair
   shape) as primary. T121-BISECT already scoped this as "needs its own
   gate" — 2b inherits that gate.
4. **Sequencing decision (binding for the landing plan):** the per-task
   lineno refactor lands BEFORE 2b's poll, or in the same leaf with the poll
   built on the refactored shape. Landing 2b's poll on the current
   three-Mutex `touch_line` would cap T121 at the same ~1.06x release
   ceiling and make the scheduler's quantum meaningless in release profile
   (a quantum of 5 000 statements each paying 3 global mutexes measures the
   mutexes, not the schedule). Conversely, the scheduler does not need to
   WAIT for T121 ≥3x green — the lineno refactor is independent code whose
   gate is the traceback corpus (s016 lineage, NEXT-S016 record at HEAD
   5e8ccac) plus the T121 scaling measurement.
5. GC handshake: bit 0 of `SAFEPOINT` (§3.1) is the future gc leaf's STW
   request. The contention-abort backstop (`SCAN_CONTENDED`, threading.rs:
   73-75 posture; gcmod.rs per v2 §2.3) stays; with a real handshake it
   should never fire — v2 02.10c's "counter reads 0 in green runs" gate is
   unchanged by anything in this doc.

---

## 8. Staged landing plan (commit sequence + gates)

Entry condition for both stages: **2a landed and green** (pool substrate,
steal-at-birth, full threading corpus + 3-run determinism + T101-T122 + s038
green — v2 §4 row 02.10a) AND the T2/generator work settled (v2 R4: no dirty
generators.rs/interp.rs at dispatch).

### Stage 2b — explicit task stacks + statement poll

| Step | Commit | Content | Gate |
| --- | --- | --- | --- |
| 2b.1 | feat(sched): Task/RunContext + fiber-mounted task execution | §2 structs; thread targets run in `TaskFiber`s via the make_fiber machinery; MOUNTED table per worker; steal restricted to Unstarted (already 2a's rule — now load-bearing); `SchedulerIn/Out` protocol | cargo test -p ap-run; threading corpus T101-T122 3-run deterministic; generator corpus (G001 family) green — fibers now nest task→generator two deep, nested-suspend shapes are the tripwire (v1 §5 refactor risk lives here) |
| 2b.2 | feat(interp): SAFEPPOINT word + yield_to_safepoint | §3.1 word + budget; P1 insertion at interp.rs:1091 (poll fused ahead of touch_line); budget re-arm at resume_live (P5); SWITCH_INTERVAL finally read by the runtime | ap-conform adaptive + threading slices byte-identical; NEW corpus cases (T-track authored, order-independent): CPU-hog-yields-to-peer shape asserting completion-aggregate only (never sleep-ordered — v1 §5 rule); T121 measurement recorded (expected unchanged — §7.1) |
| 2b.3 | perf(interp): per-task lineno bookkeeping (T121 follow-up 1) | §7.3-7.4: stmt_ordinal → per-task; frame.last_line primary; shared frame_stack retained for cross-thread readers only | Traceback corpus + s016 slice byte-identical (the lineno-semantics gate T121-BISECT demanded); T121 scaling re-measured in a quiet window (release + debug profiles both recorded — bisect follow-up 3's methodology fix); target: ratio moves toward the architecture ceiling; if < 2x, the residual is filed, NOT chased in this leaf |
| 2b.4 | docs: compat-contract native_id + switchinterval semantics | §5.2 native_id text; setswitchinterval now a real quantum | notices --check; doc gate |

**2b exit gate:** all of the above green + 3-run determinism + no corpus case
newly sleep-order-dependent (explicit checklist pass over new cases).

### Stage 2c — park/unpark + 10k gate

| Step | Commit | Content | Gate |
| --- | --- | --- | --- |
| 2c.1 | feat(sched): ParkKey + PARK_TABLE + home-P parked lists | §4.1, §4.2 mechanics, §4.4 protocol (register-under-prim-Mutex, seq guard, deferred resume) | unit tests: lost-wakeup adversarial cases per prim kind (waker-before-register, register-before-flip, spurious resume, stale-seq); cargo test -p ap-run |
| 2c.2 | feat(threading): transplant Lock/RLock/Cond/Event | §4.2 rows 1-4; cond release-save ordering preserved (§4.4 corner list) | T101-T115 corpus + t10_threading suite, 3-run; RLock re-key (§5.2) lands HERE — two-tasks-per-worker stress case (new, T-track) proves owner identity is logical not os-tid |
| 2c.3 | feat(threading): transplant Sem/Barrier/Queue + join/timer/daemon | §4.2 rows 5-7 + Join/TimerCancel/DaemonDrain; wait_for_non_daemons → scheduler drain | queue/barrier corpus slices; scrapy T1 probe (Queue-heavy) RC=0; determinism 3-run |
| 2c.4 | feat(sched): compensation lane + APX_SCHED_TRACE | §4.3 promote/cap + pinning diagnostic (v2 R3) | native-blocking corpus case completes; trace exercised under APX_SCHED_TRACE=1 |
| 2c.5 | test/gate: 10k-task bounded-memory gate | §6 arithmetic + ratio-vs-reference gate + APX_MAX_STARTED knob | the 02.10 gate proper: 10k-spawn completes, RSS ≤ 4 GiB (8 MiB stacks) or ratio ≤ 0.5 vs reference, VA ≤ 64 GiB; T121 green; asyncio slice annotated vacuous (v1 §6.5 / S-05-19) |

**2c exit gate = the 02.10 leaf gate** (orchestrator.md §02.10, as amended by
v1 §6.5's vacuity annotation): threading corpus green under the scheduler,
bounded-memory 10k case, T121 stays green, gc slice green with
contention-abort counter at 0.

Deliberately NOT in 2c: MMTk STW wiring (gc leaf, consumes §3.1 bit 0),
fiber stack tiering (02.10e), asyncio park keys (T4).

---

## 9. Risks (carried + new)

Carried from v1 §5 / v2 §5 (status under this design):
- **Determinism vs steal-half timing** (v1 §5): unchanged; corpus is
  order-independent + 3-run-pinned; new cases must follow the T101 template.
- **Resumable-frame refactor regressions** (v1 §5): RETIRED by the fiber
  choice (v2 §5), BUT re-appears in 2b.1 as *nested-suspension* risk:
  task-fiber wrapping generator-fiber is a two-deep ACTIVE/PARKED stack
  shape exercised only lightly today (twisted Deferred bridge rode exactly
  this and needed the super_ctx relay fix, a7ff45b history). The scrapy
  deferreds probe is the tripwire.
- **R1 64 MiB stacks** (v2 §5): superseded — F1/E6 already shrank pools to
  8 MiB reservation / ~330 KB committed (§6 table); residual risk is the
  all-started 10k pathological case, handled by APX_MAX_STARTED.
- **R2 fiber/GC coupling** (v2 §5): unchanged — suspended-at-poll fibers
  have suspended-at-yield shape; MMTk conservative slab scan remains the gc
  leaf's mandatory item.
- **R3 pinning visibility** (v2 §5): addressed by 2c.4's diagnostic + cap.
- **R4 sibling flux** (v2 §5): re-verified at dispatch — three sibling
  agents hold ap-run/ap-parse; this doc is docs-only and cites 5e8ccac
  lines. Line cites go stale on any interp.rs edit; the impl leaf must
  re-anchor P1/P2 line numbers on its own HEAD (the ANCHORS are semantic:
  "touch_line call in exec_stmt", not the integers).

New (this doc):
- **N1 — RLock owner re-key is a silent-failure class** (§5.1): a missed
  compare site is invisible under 1:1 and wrong under M:N. Mitigation: the
  re-key lands only WITH 2c's two-tasks-per-worker stress case; grep-audit
  all `current_os_tid()` compare sites (threading_call.rs:906, :935, :955,
  :1064 + threading.rs owners) as a checklist gate in 2c.2.
- **N2 — poll placed before touch_line changes ordinal gaps**: a suspended
  task must not have consumed an ordinal for a statement it didn't execute;
  fusing poll-ahead-of-touch_line (P1) preserves the invariant "one ordinal
  per executed statement"; the traceback corpus gate (2b.3) catches
  violations.
- **N3 — quantum fairness vs byte-exact aggregates**: switchinterval-driven
  suspension introduces timing-dependent task interleavings the corpus
  never had (GIL v1 switched only at blocking ops). Mitigation: default
  quantum stays 5 ms (coarse); the deterministic-aggregate property (v1
  §2.5) is preserved as long as primitives stay statement-atomic — which
  §4.4's Mutex discipline strengthens (a parked task can no longer be
  preempted mid-prim, because it was parked AT the prim).
- **N4 — WORKER_RECLAIM assumes main-thread drain** (threading.rs:337-360:
  "Call ONLY on the main thread with the GIL held"): under the pool, "main"
  becomes worker 0; reclaim must drain on ANY worker at task-retire
  safe points (payloads are plain Arc drops). 2a must already have moved
  this; 2b verifies it in the 2b.1 gate.

---

## 10. References

Current code (main @ 5e8ccac, 2026-11-20): `crates/ap-run/src/generators.rs`
(fibers, PooledStack, sizing); `crates/ap-run/src/interp.rs` (exec_block
:925-1015, touch_line :1040-1083, exec_stmt :1085+, Yield fiber arm
:4085-4102, run_loop_body :2094, Interpreter shared state frame.rs:499-628);
`crates/ap-run/src/threading.rs` (current_os_tid :86-106, ThreadEntry
:112-136, LOCALS :273-274, SWITCH_INTERVAL :278/:1797, prims :512-569,
PRIMS :581-583, thread_start :714-899, worker_bootstrap :906-971, join
:1010-1089, wait_for_non_daemons :1091-1116, LOCALS accessors :1432-1445,
ident/native_id :1416-1426, evict sweep :1636+); `crates/ap-run/src/
threading_call.rs` (blocking ops per §4.2 table); `crates/ap-run/src/
frame.rs` (Frame :30+, Interpreter :499-628, enter_call :891);
`crates/ap-run/src/runtime_state.rs` (interp-keyed registries); `crates/
ap-run/src/gcsupport.rs` (install_worker_thread :1943-1956).

Docs: `docs/02-10-scheduler-design.md` (v1 — GMP framing, §2 constraints,
§5 risks, §6 open questions), `docs/02-10-scheduler-design-v2.md` (M1
recommendation, §2 GC coupling, §3 preemption verdict, §4 leaf table, §5
risks), `docs/gc-mmtk-swap.md`, `docs/pon-study-tier1.md`, `plans/
antipythony/02-native-backend/T121-BISECT.md` (the §7 source),
`plans/antipythony/02-native-backend/orchestrator.md` (§02.10 gate text).

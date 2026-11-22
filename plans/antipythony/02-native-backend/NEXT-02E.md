# NEXT-02E — Leaf 02E session record (scheduler stages 2b + 2c)

Worktree: `~/.git/worktrees/leaf-02e`, branch `tier1-02e` off `main`
(`8e96a8a`). Machine policy honored: one cargo lane, CARGO_BUILD_JOBS=4,
CARGO_TARGET_DIR=$PWD/target-02e, quiet-window T121 (load 1.2-1.5).

Commits (4, oldest first):

1. `a43d993` feat(sched) 2b.1+2b.2: scheduler Task records + worker pool +
   fiber-mounted tasks + SAFEPPOINT statement poll (+ LOCALS/RLock re-key,
   ParkKey/PARK_TABLE, ParkGuard compensation lane — inseparable from the
   pool's correctness)
2. `dd1543d` feat(threading) 2c.2+2c.3: park/unpark registration transplant
   across all blocking prims
3. `b60640e` feat(sched) 2c.5: per-task recursion accounting + 10k-task
   bounded-memory corpus case (T133)
4. (this commit) docs: compat-contract scheduler semantics + session record

## What landed

**Design reconciliation FIRST (the dispatch's §1 contract was false):**
0210a landed resumable GENERATOR fibers only — `thread_start` still spawned
one 64 MiB OS thread per Python Thread; no pool/RunQueue/Permit existed.
Recorded as a blockquote amendment in
`docs/02-10-scheduler-impl-2b2c.md` §1; the minimal pool substrate is
folded into 2b.1 per that note. Also retired: §7's three-Mutex touch_line
premise (T121-fix already landed the lock-free per-frame shape @ main
8e96a8a) — 2b.3 reduced to verification + measurement (see below).

**2b.1 — Task records + pool + fiber-mounted execution** (NEW
`crates/ap-run/src/scheduler.rs`, ~850 lines): Task/TaskSpec/TaskState
records; `TaskFiber = Coroutine<SchedulerIn, SchedulerOut, (), PooledStack>`
(the SAME 0210a fiber machinery; generators created inside a task body are
untouched — disjoint yielder id spaces); P-worker pool
(APX_GOMAXPROCS | available_parallelism, 64 MiB worker stacks); per-P
RunQueue with steal-half; thread-local READY deque per home worker
(started tasks NEVER migrate — steal-at-birth is structural: unstarted
tasks are plain Arc<Task> records, started fibers are !Send and live only
in the home worker's deque; zero unsafe beyond generators.rs's audited
yielder-pointer pattern). `thread_start` now submits a Task instead of
spawning an OS thread; the registry protocol (entry slot, ident, native
stamping, mark_done, join/excepthook byte shapes) is unchanged. Finalizers
are panic-safe (corosensei propagates body panics through resume; the
worker survives and completes the done/join protocol) and complete the
registry protocol on stack-reservation failure so joins cannot hang
(C-28). Task tail payloads move to the main-thread reclaim queue
(E-safety protocol). Census uninstall is reserved for temp-worker exit
(per-task uninstall would leak per-worker registration rows).

**2b.2 — SAFEPPOINT + statement poll**: process-global
`scheduler::SAFEPOINT: AtomicU64` (bit 0 reserved for the future MMTk STW
handshake, never set in this leaf); `maybe_poll()` fused AHEAD of
touch_line in exec_stmt (N2 invariant: a suspended task never consumes a
line ordinal for a statement it did not execute). Quantum =
switchinterval in statement units (~5000 at the 5 ms default), re-armed at
mount and after every resume. On expiry the task fiber suspends INTO the
worker loop and is resumed round-robin — `sys.setswitchinterval` finally
influences runtime scheduling.

**2c.1-2c.3 — park/unpark transplant**: ParkKey union (Lock/RLock/Cond/
Event/Sem/Barrier/QueueGet/QueuePut/QueueJoin/Join/TimerCancel/DaemonDrain)
+ PARK_TABLE with register-under-prim-Mutex + park_seq epochs (the §4.4
lost-wakeup protocol) at every blocking site in threading_call.rs
(acquire/release drains, notify(k)-exact-count with park_refix surplus,
barrier generation parks, queue put/get/join, join_wait_flag keyed by the
done-flag Arc address). ParkGuard = compensation lane (2c.4 folded in):
when all P workers are parked in prim waits and unstarted tasks remain,
temporary workers spawn up to APX_COMP_CAP (default P×4) and idle-exit
after 1 s. Barrier(64) with P=8 completes in ~14 ms (the T113-class
parties>P deadlock tripwire).

**2c re-key (with 2c.2, per the doc's sequencing)**: LOCALS inner key
os_tid → logical ident (`threading::tls_key()` =
`scheduler::current_logical_ident()`, os-tid fallback on main;
bootstrap_main mounts the main ident); RLockOwner::Owned re-keyed at all
four compare sites (rlock_acquire / rlock_release_full / rlock_release_one
/ cond_owned — grep-audited, checklist gate N1).

**2c.5 — 10k-task bounded-memory gate**: T133-10k-tasks-bounded.py
(10 000 threads spawn-park-release-join, per-task TLS isolation proof,
sorted aggregate, pinned vs CPython 3.14.7 with .meta provenance; pytest
triple + stability cases pass). The pre-scheduler tree reserved 64 MiB OS
stack per started Thread (625 GiB VA at 10k — impossible); under the pool
an unscheduled task holds NO fiber (~hundreds of bytes of Task record);
the full T133 case runs in ~2 s. APX_MAX_STARTED admission backstop
(default unlimited — measured without needing it).

**GATE-FOUND BUG (the 10k gate caught a latent pre-existing bug)**: the
process-global recursion counter (frame.rs `depth: Mutex<usize>`) counted
ALL workers' in-flight frames together — at high task concurrency fresh
calls raised spurious RecursionError from OTHER tasks' frames (the
frame.rs field note predicted exactly this class). Fixed in b60640e:
enter_call/exit_call key the limit on the mounted task's OWN depth
(scheduler TASK_DEPTH thread-local; CPython per-thread semantics); main
thread keeps the legacy global path; native_frame_cap() extracted as the
shared hard-cap arithmetic. En-route subtlety: the first cut leaked a
+1 into TASK_DEPTH on the main thread (exit_call then decremented the
wrong counter — main-thread global depth only grew); caught by the corpus
run and fixed with an explicit has_task() gate.

## Gates (all measured on the final tree, release, target-02e)

- Threading corpus: **21/22** (T109 xfail pre-existing, identical on base
  per the 0210a record; T133 new, PASS)
- T10x pytest (APX_IMPL set): **118 passed / 10 skipped** (5 new T133 cases)
- T121 scaling (scripts/t121_rebaseline.py, release, sized leg N=750k,
  quiet window load 1.2-1.5): **1.237x / 1.357x / 1.320x** across three
  best-of-3 sessions — gate ≥1.0x GREEN under the scheduler (pre-scheduler
  trunk reference: 1.36x, e30d705 record; statistically indistinguishable
  — the scheduler does not regress the T121 shape, which is dominated by
  the T121-fix lock-free bookkeeping, not scheduling)
- 10k-spawn bounded memory: T133 completes ~2 s, bounded RSS (the gate's
  absolute-RSS leg recorded informally: /usr/bin/time peak footprint
  ~1.3 GB on the failing-pre-fix run including the pathological stderr;
  post-fix well under the 4 GiB budget — see honest gaps for the
  formalization caveat)
- cargo test -p ap-run --release: **206 passed / 0 failed** (18 suites;
  the two initial "failures" were the missing ap-run-eval harness binary
  — the documented stale-harness GOTCHA, fixed by building it)
- Wider corpus: values 48/48, exceptions 18/18, stdlib 48/48, controlflow
  1/1, generators GE 7/7, classes 28/33 (c002/c018/c030/c031/c032 — every
  failure identical on base per the 0210a record → zero new regressions)
- 3-run determinism: T101-T133 aggregates byte-identical across runs;
  quantum-yield exercised under APX_SCHED_TRACE (36 yields / 3 hog
  threads / 3-run identical output)

## Honest gaps / deferred

- **Parked tasks still hold their worker OS thread.** The §4.2 transplant
  landed the full REGISTRATION protocol (ParkKey/PARK_TABLE/epoch guards/
  drain-on-wake), but a task blocked in a primitive Condvar wait cannot
  physically unwind its fiber mid-native-wait, so it keeps its worker
  blocked (the compensation lane absorbs the parallelism loss). This is
  the one substantive gap vs the impl doc's "parked task costs no thread"
  claim: the 10k gate still passes because the T133 shape parks on Events
  which are… worker-held too. The gate passes on the Task-record +
  admission-control arithmetic, NOT on free-thread parking. Follow-up:
  fiber-unwinding park (suspend SchedulerOut::Park at the prim site) —
  the registration layer is exactly the surface it will consume.
- **T121 absolute number unchanged (~1.3x release), not the 3x target.**
  The scheduler neither helps nor hurts; the residual is the T121-fix
  leaf's own filed follow-up (next serializers), out of scope here.
- **T133 expected triple pinned from OUR shape, verified order-independent
  (sorted aggregate, no sleeps) but not yet re-verified against CPython on
  a different machine** — the meta records CPython 3.14.7 as the
  recording reference per the repo's standing 3.14.x debt note.
- **Formal RSS-gate script not landed.** The gate's ratio-vs-reference leg
  (v1 §6.4) needs a reference-runner that OOMs on most boxes anyway; T133
  completes with bounded memory and the absolute budget holds, but the
  automated gate script (T-track authorship per the corpus rule) is
  deferred — the pytest triple pins completion + TLS isolation only.
- **Switchinterval randomization** (v1 §5 rule) stays OFF — new cases are
  order-independent by construction, no sleep-ordered observation was
  introduced (checklist pass over T133 done).
- **Sibling stash near-miss (operational, for the orchestrator):** a
  `git stash pop` in the MAIN checkout's stash list (the T121fix leaf's
  old WIP referencing frame.rs/interp.rs) briefly produced merge
  conflicts in THIS worktree; resolved by `git restore` to HEAD (verified
  `git diff dd1543d` clean before continuing). That stash entry
  (stash@{0} on main: "mbc_* = SIBLING's, do not drop") is UNTOUCHED and
  still present.

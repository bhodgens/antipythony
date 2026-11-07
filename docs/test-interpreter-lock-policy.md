# Meept/apx test-interpreter lock policy

Applies to the Rust workspace at `~/git/antipythony` and any repo whose
test suites spawn heavyweight interpreter probes (meept, antipythony).

## Rationale
2026-11-06/07 incidents: many parallel `cargo test` lanes + `cargo build`
jobs saturated the 24-core/64GB Mac; `kernel_task` memory compression
starved WindowServer (1009 live processes in the 04:23 spindump), the
userspace watchdog fired, and the machine hung until a power cycle. The
fork-bomb fix (0676439) and RSS fences (9f7e191) cover runaway probes;
they do NOT cap aggregate build/test load.

## Rules
1. NEVER run more than one heavy `cargo test` lane at a time on this
   machine. A second lane may compile; it must wait for the first to
   finish before running its test binaries.
2. Overnight/autonomous children running cargo suites set
   `CARGO_BUILD_JOBS=4` (24-core box; full -j24 builds + suites is the
   load that caused the stall).
3. Differential/gate suites that spawn interpreter subprocesses must
   acquire the interpreter gate; a held gate means WAIT, do not start
   another suite that will queue on the same lock.
4. If a gate-wait exceeds ~10 minutes, check the holder with
   `sample <pid>` before assuming progress; a shrinking proptest case
   is legitimate (wait), a parked semaphore with ~0 CPU is a wedge
   (kill the holder, re-run).
5. Orchestrator-held verification runs (proc_* background tests) follow
   the same rule: one at a time, queued via notify-on-complete.

## Gate takeover semantics (robustness-ci leaf 00-ci-core/LEAF-gate-watchdog, 2026-11)

The `interpreter_gate()` seams (ap-run/ap-cli/ap-obj `tests/common/mod.rs`,
plus the inline copies in xtask_gates / harness_selfcheck /
ast_getattr_fence) no longer block forever on the shared
`/tmp/aprun_gate_global.lock` flock:

- **Holder record.** Immediately after acquiring the flock, the holder
  writes `AP_GATE_HOLDER pid=<pid> start=<secs>` into the lock file
  (ftruncate + write + fsync). `start` is the process start time
  (`proc_pidinfo(PROC_PIDTBSDINFO)` on macOS, `/proc/<pid>/stat` field 22
  on Linux). The record is never cleaned up on release; readers detect
  staleness.
- **Bounded wait.** Waiters poll `LOCK_EX|LOCK_NB` every 500ms instead of
  blocking. `AP_GATE_WAIT_SECS` (default 1800) bounds the wait; on expiry
  the waiting test FAILS with
  `interpreter gate held by pid N with no progress for M min; kill it and
  re-run` instead of queueing the whole suite fleet behind a wedged
  holder (the 2026-11-06 repr_fuzz wedge: 3h45m, ~0 CPU).
- **Stale-holder takeover.** While waiting, the waiter probes the recorded
  PID. `kill(pid, 0) == ESRCH` with the recorded start time confirming the
  PID is not reused (a reused PID has a different start time and is NEVER
  taken over) means the holder is dead; since the kernel closes a dead
  process's fds, a live flock from it is impossible, so the waiter retries
  `LOCK_EX|LOCK_NB` and takes over, printing
  `gate: took over stale lock from pid N` to stderr. If the non-blocking
  flock still fails, the record was stale but the lock is live (a fresh
  holder that has not rewritten it yet): keep waiting.
- **No false takeover.** A live, CPU-busy holder (shrinking proptest) has
  a matching start time and is waited for, never stolen from.
- **Old behavior.** `AP_GATE_WAIT_SECS=0` restores the unbounded blocking
  flock for anyone who wants it.

## Incident log
- 2026-11-06 ~11:24: fork-bomb stall (fixed 0676439, fenced_child_cmd).
- 2026-11-07 04:23: WindowServer watchdog stall from 1009-process
  flood; power cycle 12:04. Root cause: uncapped parallel cargo lanes.

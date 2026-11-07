# Leaf spec: suite-level watchdog for the interpreter gate

## Problem (observed 2026-11-06/07)

`interpreter_gate()` (crates/ap-obj/tests/common/mod.rs:116) acquires
`/tmp/aprun_gate_global.lock` with a blocking `flock(LOCK_EX)` and the
holder keeps it until process exit (deliberately leaked File, line 133).
There is no timeout and no holder-liveness metadata:

- 2026-11-06: a `repr_fuzz` process held the gate ~3h45m while internally
  wedged (parked semaphore, ~0 CPU). Every other gated suite queued
  behind it for the full duration.
- Any genuinely hung test binary poisons ALL interpreter-spawning suites
  repo-wide until manually killed.

Legitimate long holds exist (proptest shrinking a failing case runs
minutes at high CPU), so a naive short timeout on the WAIT side is wrong.

## Required behavior

1. **Bounded wait in `interpreter_gate()`**: replace `flock(LOCK_EX)`
   with a poll loop — try `LOCK_EX | LOCK_NB`, on `EWOULDBLOCK` sleep
   500ms and retry until `AP_GATE_WAIT_SECS` (default: 1800).
2. **Holder liveness record**: immediately after acquiring the lock,
   write the holder's PID + boot-uuid (or process start time from
   `libproc`/`proc_pidinfo`, or `/proc`-equivalent) into the lock file
   (ftruncate + write + flush) BEFORE dropping into test execution.
   Release path (process exit) need not clean up; PID staleness is
   detected by readers.
3. **Stale-holder takeover**: while waiting, read the recorded PID.
   If the recorded holder no longer exists (kill(pid, 0) == ESRCH, and
   PID wrap guarded by recorded process start-time mismatch), break the
   flock by stealing it: acquire LOCK_EX|LOCK_NB (the dead holder's fd
   is closed by the kernel at process death, so a live flock from a
   dead process is impossible — if LOCK_NB still fails, the record is
   stale but the lock is live: keep waiting). Emit a stderr warning
   line `gate: took over stale lock from pid N` when takeover happens.
4. **Suite-level deadline safety net (second, independent mechanism)**:
   in the two differential-suite entry points that spawn ap-run probes
   (ap-conform harness + repr_fuzz reference runs), a suite that has
   made no forward progress for 15 minutes (no child running AND no CPU
   delta on self) must abort the suite with a clear panic message
   naming the gate, instead of waiting forever. Simplest acceptable
   form: watchdog thread in `interpreter_gate()` that, if still waiting
   at deadline, samples the holder PID's CPU once; if the holder used
   <1s CPU across the sample window and has no live child (recheck via
   pgrep-style scan is overkill — CPU-only check acceptable), log and
   proceed with the takeover path from (3) regardless of flock state by
   failing the CURRENT test with an actionable error instead of
   blocking:
   `panic!("interpreter gate held by pid N with no progress for M min; kill it and re-run")`.

## Constraints

- Tests/gates are never weakened: this changes lock plumbing only, no
  test bodies, no expectations.
- No new dependencies (libc + std only; libc is already in use at
  common/mod.rs:131).
- The lock file path `/tmp/aprun_gate_global.lock` (via temp_dir) is
  unchanged, so all suites interoperate without a flag day.
- PID reuse guard: record `kv_pid <start_time_secs>` where start_time
  comes from `proc_pidinfo` on macOS (Linux: `/proc/<pid>/stat` field
  22). Mismatched start time = stale record.
- AP_GATE_WAIT_SECS=0 restores the old infinite-block behavior for
  anyone who wants it.

## Acceptance

- Unit/integration test: process A takes gate, writes PID, then exits
  WITHOUT releasing explicitly; process B (spawned test or manual
  binary run) acquires within one poll interval after A's death.
- Suite with a live, CPU-busy holder still waits (no false takeover):
  verified by holding the gate from a busy-loop process for 60s while
  a waiter polls — waiter must NOT take over before 60s elapses.
- `cargo test -p ap-obj --test repr_fuzz` still passes end-to-end
  (gate behavior transparent to the suite).
- Full behavior documented in docs/test-interpreter-lock-policy.md
  (already exists; extend its "Rules" with the takeover semantics).

## Priority rationale

Filed after the 2026-11-06 wedge (3h45m gate hold by a parked
repr_fuzz) and the 2026-11-07 machine stall. The fork-bomb and RSS
fences (0676439, 9f7e191) protect the machine from probes; this leaf
protects the SUITE FLEET from a poisoned shared lock.

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

## Incident log
- 2026-11-06 ~11:24: fork-bomb stall (fixed 0676439, fenced_child_cmd).
- 2026-11-07 04:23: WindowServer watchdog stall from 1009-process
  flood; power cycle 12:04. Root cause: uncapped parallel cargo lanes.

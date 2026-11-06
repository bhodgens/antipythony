# Test-harness resource & concurrency review — 2026-11-06

Scope: every test harness in the repo, audited after the 2026-11-06 fork-bomb
power-cycle (root cause: `ast_getattr_fence::fence_kill_path_is_live` re-
executed the test binary unfixed) and the 2026-11-05/06 jetsam events
(74–242 GB recorded footprints from unfenced differential suites and ad-hoc
/tmp harness binaries). Baseline: commits 0676439 + 9f6e2b9 (in-process RSS
self-guard, fenced_output, suite-level interpreter_gate, fork-bomb fix).
This review covers everything ELSE that spawns processes or allocates.

Machine: macOS M2 Ultra 64 GB. Audit + fixes commit(s): see "Fixes committed".

## Per-harness audit

| Harness | Spawns interpreter? | Fenced (RSS+deadline)? | Single-tenancy gated? | Verdict / action |
|---|---|---|---|---|
| ap-run/tests/classes_differential.rs | yes (ap-run-probe + CPython ref) | yes (run_aprun_eval fence) | NO → **FIXED**: gate in all 29 tests |
| ap-run/tests/gc_differential.rs | yes | yes | NO → **FIXED**: gate in 2 tests |
| ap-run/tests/re_differential.rs | yes | yes | NO → **FIXED**: gate in 42 tests |
| ap-run/tests/method_surface_differential.rs | yes | yes | NO → **FIXED**: gate in 34 tests |
| ap-run/tests/name_resolution_differential.rs | yes | yes | NO → **FIXED**: gate in 24 tests |
| ap-run/tests/metaclass_arity_differential.rs | yes | yes | NO → **FIXED**: gate (1 test) |
| ap-run/tests/metaclass_enum_differential.rs | yes | yes | NO → **FIXED**: gate in 6 tests |
| ap-run/tests/import_differential.rs | yes | yes | yes (prior work) | OK |
| ap-run/tests/import_guard_differential.rs | yes | yes | yes (prior work) | OK |
| ap-run/tests/lxml_differential.rs | yes | yes (fenced_output) | yes (prior work) | OK |
| ap-run/tests/fix_pass_differential.rs | yes | yes | 2/3 tests → **FIXED**: gate added to bugc_import_warnings_succeeds |
| ap-run/tests/re_fuzz.rs | yes (ap-run-eval + CPython per case) | deadline-only per case (AP_FUZZ_TIMEOUT_MS, kill+skip), no RSS cap | NO → **FIXED**: gate in both spawn-running tests | RSS cap left as-is: re_fuzz deliberately distinguishes REF_TIMEOUT vs ENGINE_TIMEOUT outcomes; a killing RSS fence would change the nightly-lane contract. Deadline + subprocess-per-case + gate bound it. |
| ap-cli/tests/packaging_differential.rs | yes | yes | yes (prior work) | OK |
| ap-cli/tests/stdlib_modules_differential.rs | yes | yes | yes (prior work) | OK |
| ap-cli/tests/ast_getattr_fence.rs | yes (ap-run-probe) | yes (RSS 512MB + deadline, ps-sampled) | NO → **FIXED**: local gate + preflight in both tests; **also reordered the HOG-mode check before gate acquisition** (the hog child re-executes this test; taking the flock first deadlocked the child → the fence self-test hit the deadline arm instead of the memory arm; caught by test run during this review) |
| ap-gc/tests/gc_invariants.rs | no (in-process, small allocations) | n/a | n/a | OK — no subprocess, no large in-process allocation |
| ap-obj/tests/cpython_differential.rs | CPython ref only | NO (bare .output(), unbounded) | NO → **FIXED**: gate + preflight + 60s bounded wait (AP_REF_TIMEOUT_SECS) around cpython() |
| ap-obj/tests/repr_fuzz.rs | CPython ref only (batched stdin) | NO (bare spawn+wait, unbounded) | NO → **FIXED**: gate + preflight + bounded wait in ref_batch; in-process impl side allocates only CHUNK=50 values per case |
| ap-conform/tests/harness_selfcheck.rs | yes (ap-conform → impl+ref processes) | ap-conform itself has a per-process 10s timeout (default) | NO → **FIXED**: gate + 120s bounded_output on all 3 tests |
| ap-parse/tests/* (corpus, fstring_debug, goldens, scopes) | no (in-process) | n/a | n/a | OK |
| ap-ir/tests/verify.rs | no (in-process) | n/a | n/a | OK |
| xtask/tests/xtask_gates.rs | no (xtask → cargo metadata subprocess) | NO (unbounded .output()) | NO → **FIXED**: gate + 180s bounded_output on all spawns |
| crates/ap-run/src/bin/ap_run_probe.rs | is the interpreter | in-process RSS self-guard (AP_MAX_RSS_GB, default 25% RAM) | n/a | OK (prior work) |
| crates/ap-cli/src/bin/ap_run_eval.rs, ap_obj_eval.rs | is the interpreter | RSS self-guard (prior work) | n/a | OK |
| crates/xtask/src/cli_docs.rs (bin, not test) | spawns CLI bins (arg-print only) | no deadline | n/a | Accepted: runs only under explicit `cargo xtask cli-docs`, single-shot, not a test harness; documented residual |
| crates/xtask/src/stdlib_provenance.rs (bin) | `python -c sysconfig` probe | no deadline | n/a | Accepted: one tiny one-shot probe under explicit xtask invocation; documented residual |
| .github/workflows/ci.yml, nightly-fuzz.yml | run the suites above | suites carry their own fences | suite gates serialize cross-process | OK locally; GH-runner note below |

Recursion / self-re-execution: the only re-execution path in the repo is
ast_getattr_fence's hog self-test; it is guarded by AP_AST_FENCE_HOG=1 +
`--exact` filter, and after this review the guard is checked before any
lock acquisition. No other harness re-executes itself or a test binary.
Unbounded waits: eliminated — every remaining `Command` wait in test code
is either fenced_output/run_aprun_eval (RSS+deadline), re_fuzz run_capped
(deadline), the new bounded_output/reference-deadline wrappers, or
ap-conform's internal per-process timeout.

Worst-case footprint of `cargo test --workspace`: cargo runs test BINARIES
in parallel, but every binary that spawns an interpreter now takes the
shared `/tmp/aprun_gate_global.lock` for its whole lifetime, so at most ONE
interpreter-spawning binary is active at a time repo-wide; within it, heavy
probes serialize on `/tmp/aprun_heavy_gate.lock` (or are CPython-only,
megabytes). Non-spawning binaries (ap-parse, ap-ir, ap-gc) are small and
in-process. Bounded worst case: one debug-build interpreter under the
6 GB/240 s fence + one RSS self-guard (25% RAM) + test-binary overhead.

## Fixes committed (this review)

- Gate (`interpreter_gate`) added to: classes/gc/re/method_surface/
  name_resolution/metaclass_arity/metaclass_enum differential suites,
  re_fuzz (both spawn-running tests), fix_pass (3rd test), ast_getattr_fence
  (local seam, both tests), ap-obj common (gate+preflight+reference
  deadline) wired into cpython_differential + repr_fuzz::ref_batch,
  harness_selfcheck (local seam + bounded_output), xtask_gates (local seam
  + bounded_output).
- Preflight (memory-pressure) added to ast_getattr_fence, ap-obj suites,
  harness_selfcheck is gated+bounded (ap-conform already time-bounds
  children internally).
- ap-obj + ap-conform + xtask gained a `libc` dev-dependency for the flock
  seam (same crate version already in the lock graph).
- ast_getattr_fence: HOG-mode check moved ahead of gate acquisition
  (self-deadlock found and fixed by running the fence self-test).

## Residual risks

- /tmp harness binaries OUTSIDE the repo (w112_bin, t1gate-bin, t1i-bin,
  cont-bin, ap-run-probe ad-hoc copies, run116.bin): unrecorded, unfenced,
  uncapped. Hardening recipe (one line): never launch a bare interpreter
  copy by hand — `AP_MAX_RSS_GB=2 AP_PROBE_TIMEOUT_SECS=240 timeout 300
  ./target/debug/antipythony probe.py` or, better, write the probe as a
  `#[test]` so it inherits fenced_output + interpreter_gate.
- re_fuzz has no per-case RSS cap (deadline only); a memory-growth engine
  bug that does NOT hang can still reach the in-process rss_guard ceiling
  (25% RAM) before the deadline — acceptable given ap-run's self-guard is
  wired into ap-run-eval.
- GitHub runners: the default probe fence is 6 GB / 240 s; standard GH
  ubuntu runners have ~7 GB RAM, so a pathological probe can OOM the runner
  itself (not the local machine). If that bites, set AP_PROBE_MAX_RSS_GB=2
  in ci.yml.
- xtask cli_docs / stdlib_provenance bins spawn subprocesses without
  deadlines; only reachable by explicit manual `cargo xtask` invocation.

## Rules for new harnesses (binding)

1. Every interpreter/CLI spawn goes through a fence: `fenced_output` /
   `run_aprun_eval` (RSS cap + deadline + preflight), or at minimum a
   deadline-bounded wait. A bare `.output()`/`wait_with_output()` in test
   code is a bug.
2. Every test that spawns anything takes `common::interpreter_gate()` first
   (the shared /tmp/aprun_gate_global.lock, held to process exit).
3. No harness may re-execute itself or any test binary; if a self-test
   needs a child, guard it with a mode env-var checked before ANY lock or
   side effect, and pass an `--exact` filter.
4. Probes run as subprocesses (heap dies with the process), never in-process
   in the test binary; poll RSS via proc_pid_rusage/VmRSS and fail closed on
   sampling failures.
5. New knobs default safe: caps/deadlines on, env-overridable upward, and
   the memory-pressure preflight must be able to REFUSE to start.

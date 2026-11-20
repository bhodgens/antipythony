# T121 bisect: scaling-gate regression root-caused (leaf T121-bisect, 2026-11-19/20)

Appends the trail of T121-REVERIFY-0208f.md. Worktree: ~/.git/worktrees/leaf-t121b,
branch tier1-t121b @ c51473f (fix commit on top of main 74ab286).

## TL;DR verdict

The 0.74x "regression" on main has TWO stacked causes, both now understood:

1. **Measurement-profile mismatch (the REVERIFY finding itself):** the 02.09b
   gate record (3.31x, serial 0.992s / parallel 0.300s) was measured with the
   **DEBUG** build — the pytest harness `_impl_cmd()` defaults to
   `target/debug/antipythony`. The 02.08f re-verify measured the **RELEASE**
   build (serial 0.114s). Release serial is ~9x faster than debug serial; the
   ratio is profile-relative. There was never a 9x serial speedup between
   87ffb23 and main — release serial at 87ffb23 TODAY measures 0.110s, i.e.
   release was already "fast" at the gate record's commit.
   Verified: debug build at 87ffb23 in a quiet window reproduces 3.0-3.36x
   (serial 1.016-1.020 / parallel 0.303-0.340) — the recorded gate number is
   real, for the debug profile.
2. **A REAL parallel-leg regression landed at the 02.08f merge c7671d7:**
   debug parallel 0.30-0.34 @ 87ffb23/aaafc6d -> 0.51-0.61 @ c7671d7/main
   (+70%); release parallel 0.11 -> 0.16-0.21. Root cause (read from the
   merge diff, confirmed by measurement): **02.08f s020's `pull_iter` CallFn
   probe** acquires the SHARED process-global `interp.iterators` Mutex PLUS
   the iterator state Mutex on EVERY iterator pull — and `Stmt::For` pulls
   per loop step. 4 workers x 60k steps = ~480k extra acquisitions on ONE
   global mutex. The workload creates no CallFn iterators, so the probe was
   pure overhead.

**FIX LANDED** (c51473f): `CALLFN_ITER_EXISTS: AtomicBool` in iter.rs, set
once by the `iter(callable, sentinel)` builtin; `pull_iter` skips the probe
unless a CallFn iterator exists process-wide. Effect (quiet window):
- release ratio on main: 0.72x -> **1.06x** (parallel best 0.163 -> 0.103,
  back at/below the 87ffb23 band 0.109-0.162);
- debug pytest scaling gate: ~1.5-1.7x -> **2.03-2.47x**.

## Gate verdict: RED (unchanged at the assertion level)

`test_scaling_gate_3x_on_4cores_antipythony` post-fix: 2.03/2.47/2.21x
(best-of-3 runs, load 3-7) — still < 3.0. At 87ffb23 the same test measures
3.0-3.36x. Residual post-fix delta vs 87ffb23 (~15-25% debug parallel) is
NOT fully attributed; the floor is architectural:

**Architectural ceiling (settled methodology finding):** per-statement
global bookkeeping (`touch_line` acquires stmt_ordinal + stmt_lines +
frame_stack — three process-global Mutexes per executed statement, the
02.09b P7 decision-2 shape) serializes all workers proportionally to WORK.
Workload-scaling experiment (workload N x10 to 600000 so release serial
~1.1s): ratio stayed 1.02x — the ratio does NOT approach 3x with bigger
workloads; contention scales with the work. The 3x reading was only ever
visible on the DEBUG profile, where unoptimized per-statement compute
dwarfs the lock cost. **Reaching >=3x in release requires making the
per-statement line bookkeeping per-frame/thread-local (lock-free), not a
bigger workload.** That is an architecture change filed for a follow-up
leaf (touches traceback lineno semantics — needs its own gate).

## Bisect trail (honest notes)

- Bisect #1 (e77084b..main, release, ratio>=3.0 judge): INVALID — the
  release ratio never reaches 3.0 anywhere in the range (all commits ~0.6-1.0),
  so the asserted-good endpoint e77084b does not reproduce the record.
  "First bad a69a9d9" was an artifact of the untestable endpoint.
- Bisect #2 (87ffb23..main, release, parallel<=0.150 judge): FALSE POSITIVE
  (converged on 19955d3, a wasm-only commit whose binary is byte-identical
  to its parent — proven by md5; ap-cli does not depend on ap-codegen-wasm
  at that commit). Cause: threshold inside the load-noise band.
- Bisect #3 (87ffb23..main, release SERIAL<=0.115 judge): converged on
  **c7671d7** (the tier1-0208f merge). CONFIRMED by direct A/B/A measurement
  of debug-parallel: 87ffb23 0.302-0.325 / aaafc6d 0.307-0.356 /
  c7671d7 0.510-0.599 / main 0.516-0.583 (interleaved, quiet window).
  The serial-leg threshold split (0.1146 vs 0.1150) was marginal, but the
  parallel A/B is unambiguous.

## Numbers (release, quiet window, best-of-3, in-process perf_counter)

| commit               | serial  | parallel | ratio  |
|----------------------|---------|----------|--------|
| 87ffb23 (gate record)| 0.110   | 0.109    | 1.01x  |
| aaafc6d (REVERIFY pt)| 0.114   | 0.154    | 0.74x  |
| main pre-fix 74ab286 | 0.117   | 0.163    | 0.72x  |
| main + fix c51473f   | 0.109   | 0.103    | 1.06x  |

Debug profile (pytest harness profile of record): 87ffb23 3.0-3.36x;
main pre-fix 1.5-1.7x; main post-fix 2.03-2.47x.

## Gates held

- `cargo test -p ap-run --release`: 18 suites, 196 passed / 0 failed /
  1 ignored, rc=0 (log ~/.git/worktrees/leaf-t121b/gate-ap-run-release.log).
- Threading corpus slice via ap-conform: 17/18 — the 1 fail is the
  documented pre-existing T109-exception-in-thread red (unchanged).
- `pytest tests/t10_threading -q`: 97 passed / 1 failed / 10 skipped —
  the 1 fail is the >=3x scaling gate itself (2.03x), the subject of this
  leaf; everything else in the slice is green.

## Follow-ups filed

1. Per-statement global line bookkeeping -> per-frame lock-free (the real
   3x unlocker; needs traceback lineno semantics gate).
2. Residual ~15-25% debug-parallel delta vs 87ffb23 post-fix (low priority;
   release profile fully recovered).
3. Harness methodology: the gate should pin WHICH build profile it measures
   (debug vs release) — the record vs re-verify mismatch came from this.

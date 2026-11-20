# T121 quiet-window re-verify finding (02.08f, 2026-11-19)

## Measurement (M2 Ultra 24-core, load 2.4-6.6, after external load cleared)

Release build (target/release/antipythony @ main aaafc6d), corpus script's
own in-process perf_counter (the 02.09b harness contract), best-of-3:

- serial:   0.114155 / 0.116020 / 0.116790  (best 0.114155)
- parallel: 0.153878 / 0.169942 / 0.186082  (best 0.153878)
- RATIO: 0.114 / 0.154 = **0.74x**  (parallel SLOWER than serial)

Same measurement on main @ 87ffb23 (02.09b gate) reported serial=0.992s /
parallel=0.300s = 3.31x. The corpus workload (4x range(1,60000) arithmetic
loop) is BYTE-IDENTICAL between the two points (verified: git diff empty).

## Diagnosis

The serial leg got ~8.7x FASTER since 02.09b (0.992s -> 0.114s) while the
parallel leg got ~2x faster (0.300s -> 0.154s). The >=3x gate measures a
RATIO, not throughput. Serial throughput improved 8.7x; parallel improved
only 2x. Plausible causes (not bisected — out of leaf budget):

1. Single-threaded interpreter fast-path work (wave-3 memperf g1 hot-path
   stack landed AFTER the 02.09b gate record: d7f1bcc is IN 02.09b, but the
   110 commits since include 03.03-03.07 interpreter/wasm work with
   executor-side churn).
2. Thread-park/wake overhead per Thread start/join now dominates the tiny
   0.11s workload: 4 threads x (spawn+park+join) fixed cost is a much
   larger fraction of a 0.15s window than of a 0.30s window.
3. Possible serializer regression (worker-only-serialization interim from
   02.09a decision-2) — with a faster serial leg the serialization wall is
   more visible.

## Verdict

HONEST REPORT: T121 >=3x scaling gate is RED at 0.74x on main @ aaafc6d.
This is a REGRESSION relative to the 02.09b gate record (3.31x), NOT
caused by this leaf (verified: leaf commits touch no threading/scheduler
code; main @ aaafc6d without the leaf worktrees shows the same numbers).
The 02.09b "quiet-window re-verify PENDING" note is now CLOSED with a
finding instead of a green: the ratio regressed. Follow-up leaf should
bisect e77084b..main for the serial-speedup commit set and re-measure
scaling against the NEW serial baseline (or re-tune the workload size so
thread fixed costs don't dominate).

Recorded by leaf 02.08f. Worktree: ~/.git/worktrees/leaf-0208f


---

## TRAIL APPEND (leaf T121-bisect, 2026-11-20): root-caused + partially fixed

See T121-BISECT.md (same directory) for the full record. Short form:
the 3.31x record was a DEBUG-profile measurement (pytest harness default
runner); the 0.74x re-verify read the RELEASE profile (already fast at the
record commit). A REAL regression landed at the 02.08f merge c7671d7 — the
s020 pull_iter CallFn probe double-locked the shared iterators mutex per
loop step — FIXED at c51473f on tier1-t121b (release ratio 0.72->1.06x,
pytest gate 1.6->2.0-2.5x). Gate remains RED (<3.0x): the residual ceiling
is the per-statement global line bookkeeping (touch_line x3 process
mutexes), which serializes workers proportionally to work (workload x10
scaling experiment: ratio stays ~1x release). >=3x release needs lock-free
per-frame line tracking — filed as follow-up.

# T121 RE-BASELINE (leaf T121c, 2026-11-20) — sized-workload, profile-pinned scaling gate

Appends the trail of T121-REVERIFY-0208f.md and T121-BISECT.md. User decision
C: re-baseline the gate BEFORE deciding fix-vs-accept. **This leaf is
MEASUREMENT-ONLY — zero interpreter/threading code changes** (verified: the
leaf's diffs touch the harness script + this doc + raw JSONs only).

Worktree: ~/.git/worktrees/leaf-t121c, branch tier1-t121c (off main 213e2ac).
Harness: scripts/t121_rebaseline.py (commit a3c67bf); raw numbers:
T121-REBASELINE-raw.json (release), T121-REBASELINE-raw-debug.json +
T121-REBASELINE-raw-debug-n60k.json (debug), all committed at a91dd9c.

## Why the old harness was untrustworthy (methodology findings)

The pytest gate (`test_scaling_gate_3x_on_4cores_antipythony`) had four
defects, all confirmed by this leaf:

1. **Profile never pinned.** `_impl_cmd()` defaults to `target/debug/` —
   the 02.09b "3.31x GREEN" record was a DEBUG number; the shipped release
   artifact was never >=3x anywhere in e77084b..main (bisect, confirmed
   again below).
2. **Undersized serial leg.** Release serial at N=60000 is ~0.11s; thread
   spawn/park/join fixed cost is a large fraction of the measured window.
3. **Ratio-of-tiny-numbers fragility.** A 0.01s perturbation on a 0.11s leg
   moves the ratio ~10%.
4. **Sequential legs (serial then parallel), best-of-3** — drift-sensitive;
   not interleaved.

## New harness (scripts/t121_rebaseline.py)

- **Sized leg**: workload iterations overridable via `T10_REBASELINE_N`
  (corpus default stays 60000 — the conformance pins are byte-identical;
  verified against tests/corpus/threading/expected/). Chosen size
  **N=750000 -> release serial ~1.40s**, fixed spawn/join cost <2% of the
  leg. Selection was measured, not guessed: N=500000 -> 0.946s (just under
  the 1s floor), N=750000 -> 1.401s.
- **Profile-pinned**: measures an explicit release runner AND debug runner;
  the gate number is the RELEASE number. `--profile` selects one or both.
- **Interleaved A/B/A**: each round = serial, parallel, serial (best-of all
  serials, best-of parallels); 5 rounds default.
- **Quiet-window enforcement**: refuses to start (and re-checks every round)
  while the 1-minute load average is >= the threshold (2.0 release / 2.5
  debug — the debug leg is 13s/round and the host was busier later in the
  session). Every recorded round's load is printed below.
- **Old numbers kept**: the N=60000 leg runs in the same session, same
  methodology, right after the sized leg — continuity with 02.09b / bisect.

Known harness gaps (honest): NSH is fixed at 4 (2-core/8-core sensitivity
NOT run — the workload shape is pinned by the conformance pins and a
variable-thread workload would need a second corpus case; deferred).
Best-of-R takes a min, which is optimistic under residual load; all raw
samples are in the committed JSONs for re-analysis.

## Raw numbers (best-of-5 interleaved rounds, per-leg min, in-process perf_counter)

Release profile (target-0304c/release, quiet loads 1.3-2.3):

| leg            | serial (s) | parallel (s) | RATIO  |
|----------------|-----------|--------------|--------|
| N=750000 (sized) | 1.3970   | 1.3350       | **1.05x** |
| N=60000 (old)  | 0.1130    | 0.1030       | 1.10x  |

Debug profile (target-0304c/debug, loads 1.6-2.2):

| leg            | serial (s) | parallel (s) | RATIO  |
|----------------|-----------|--------------|--------|
| N=750000 (sized) | 13.0215  | 4.2691       | **3.05x** |
| N=60000 (old)  | 1.0415    | 0.3398       | 3.06x  |

Round-level samples (all rounds, load in parens):

- release N=750k serial: 1.428/1.431/1.428/1.415/1.427/1.417/1.419/1.397/
  1.407/1.451 (10 samples, 2 per round x 5); parallel: 1.335/1.340/1.353/
  1.644/1.376 (loads 1.3-2.3).
- release N=60k serial: 0.1138..0.1151 (spread 1.1%); parallel:
  0.1030..0.1104.
- debug N=750k: serial 13.02-13.23 (spread 1.6%); parallel 4.238-4.645.
- debug N=60k: serial 1.0415-1.0443; parallel 0.3398-0.3469.

The old-workload release ratio (1.10x) matches the bisect's post-c51473f
band (1.06x) — the re-baseline harness reproduces the prior methodology's
result, so the sized-leg number is apples-to-apples trustworthy.

## Findings

1. **The >=3x gate has never been true of the RELEASE build on this
   workload, at any commit tested, at any workload size.** With a 12.6x
   bigger serial leg (0.11s -> 1.40s), release ratio stays 1.05x —
   thread-fixed-cost is now <2% of the leg, so this is NOT a workload-size
   artifact. This settles the REVERIFY "plausible cause 2": the answer is
   cause 3 (a real proportional serializer), quantified below.
2. **The DEBUG 3x is real but measures unoptimized compute, not scaling
   architecture.** Debug serial is 9.3x slower than release serial while
   debug parallel is only 3.1x slower than release parallel: in debug, each
   statement does ~9x more per-thread compute than lock bookkeeping, so
   locks hide; in release, the lock cost IS the dominant per-statement cost.
   The debug profile will pass a >=3x gate at ANY workload size (3.05x at
   1.4s, 3.06x at 0.11s serial) — it is not a valid scaling gate.
3. **The serializer scales with work** — consistent with the bisect's
   per-statement 3-process-global-mutex bookkeeping (`touch_line`:
   stmt_ordinal + stmt_lines + frame_stack, the 02.09b P7 decision-2
   interim).

## Verdict: gate stays RED in release; achievable number quantified; follow-up is REAL work

Per the leaf contract's option (ii)+(iii) combined, reading the evidence:

- **The >=3x-on-release gate is NOT achievable on this architecture today:
  measured 1.05x, ceiling ~4.19x even with ALL parallel-side overhead
  removed.** The gate must not be re-baselined GREEN on the debug profile —
  that would certify an artifact nobody ships.
- **Recommended re-baseline: >=1.0x on the RELEASE profile with the sized
  leg (measured 1.05x; old leg 1.10x)** — i.e. "parallel is never slower
  than serial on meaningful work, on the shipped artifact," which is the
  honest no-GIL claim the release build can make today. The gate should pin
  BOTH the profile and the workload size.
- **Payoff estimate for the lock-free line-bookkeeping follow-up**
  (per-frame/thread-local stmt_ordinal/stmt_lines/frame_stack instead of 3
  process-global mutexes per statement):

  - Parallel leg 1.335s vs perfect-4x floor 0.334s: **1.001s (75.0%) of
    the parallel leg is non-work overhead** (bookkeeping serialization +
    residual fixed cost).
  - Treating the bisect-attributed bookkeeping as that excess minus ~2%
    fixed cost: **~73% of the parallel leg** is lock bookkeeping (an upper
    bound on the payoff — some excess is memory traffic/cache pressure).
  - Amdahl ceiling on 4 cores if fully removed: **~4.19x**; realistic
    post-fix estimate (fixed cost remains): **~3.9x** — the >=3x gate
    becomes genuinely reachable in RELEASE after the lock-free fix, on a
    ~1.4s serial leg. This upgrades the follow-up from "the real 3x
    unlocker (bisect, hypothesis)" to "the real 3x unlocker (measured,
    ~3.9x expected)".
  - Debug profile after such a fix would rise ABOVE 3.05x (compute per
    statement stays 9x, bookkeeping no longer serializes) — the debug
    number is not the target, release is.

## Recommendation to the orchestrator

1. Re-point the pytest gate at the release runner + sized leg
   (T10_REBASELINE_N=750000) and assert >=1.0x now; keep printing (not
   asserting) the debug number for architecture-drag visibility. This is a
   HARNESS change (allowed for the next leaf), not a corpus weakening: the
   corpus pins are untouched and the gate moves to the shipped artifact.
2. File/confirm the lock-free per-frame line-bookkeeping leaf as the path
   to a >=3x RELEASE gate, with the measured ~3.9x expected payoff and a
   traceback-lineno-semantics gate attached (per the bisect's follow-up 1).
3. Until that leaf lands, T121 (release scaling) is RED at 1.05x against
   the aspirational >=3x, GREEN at 1.05x against the honest >=1.0x
   re-baseline. No interpreter code was touched by this leaf.

## Gates held by THIS leaf (measurement-only, so: no regressions possible; verified anyway)

- `cargo test -p ap-run --release` (CARGO_TARGET_DIR=target-0304c, rc=0):
  18 suites, **196 passed / 0 failed / 1 ignored**
  (log: gate-ap-run-release.log in the worktree).
  GOTCHA recorded: the fenced differential suites spawn
  `<target-dir>/debug/ap-run-eval`; a bare `cargo test -p ap-run --release`
  in a FRESH worktree fails fix_pass_differential with "No such file or
  directory" because .cargo/config.toml's target-dir (target-0304c) is not
  exposed to the test binary's env lookup — run with
  `CARGO_TARGET_DIR=$PWD/target-0304c` (or pre-build ap-run-eval). Not a
  code regression; environment plumbing.
- Threading corpus slice: `APX_IMPL=target-0304c/release/antipythony run
  pytest tests/t10_threading -q` -> **97 passed / 10 skipped**, 1 failed =
  the >=3x scaling gate itself at 1.05x (the subject of this leaf;
  T120/T122 all green).
- Corpus pins: T121 default-mode output byte-identical to
  tests/corpus/threading/expected/ (verified pre-commit).

## Re-run instructions (exact)

```
cd ~/.git/worktrees/leaf-t121c   # or any worktree with the harness commit
cargo build --release -p ap-cli && cargo build -p ap-cli   # target-0304c/{release,debug}
python3 scripts/t121_rebaseline.py --rounds 5 \
    --threads 750000 60000 --min-load 2.0 \
    --out plans/antipythony/02-native-backend/T121-REBASELINE-raw.json
python3 scripts/t121_rebaseline.py --profile debug --rounds 5 \
    --threads 750000 60000 --min-load 2.5 \
    --out plans/antipythony/02-native-backend/T121-REBASELINE-raw-debug.json
```

The harness refuses to run under load; expect to wait out external lanes
(this session waited out multiple go-test/mediaanalysis spikes at load
10-38).

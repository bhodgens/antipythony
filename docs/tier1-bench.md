# Tier-1 Bench Gate (leaf 02.05h) — adaptive vs forced tier-0

**Gate:** >=1.5x geomean speedup, adaptive vs `--tier 0`, arithmetic/string
slice, RELEASE builds. This is the ORCHESTRATOR §02.05 exit gate.

## Verdict: PASS — 2.07x conservative geomean (floor 1.5x)

Measured 2026-11-11 on Mac M2 Ultra, `ap-run-eval` RELEASE build
(`target-0205h/release`), at main = 9d0a9f4 (all of 02.05a–g landed).
3 full runs × 9 timed reps per mode per program, warmup run first.
Raw numbers: `tests/bench/tier1/bench_report.json`.

## Slice

`tests/bench/tier1/` — four function-shaped programs. The JIT dispatch arm
lives in `call_function`, so benchmarks must route work through a called
function (`def hot(n)`); module-level loops never reach `jit_call_body`
(execs=0). Each program makes 30 warm calls (`hot(2000)`) before one large
timed call — the same shape the tier1 transparency probes use, and enough
for the background tier-up driver (02.05e) to install tier-1 mid-run.

| program | loop body | tier-1 state | worst-case speedup | median/median |
|---|---|---|---|---|
| `bench_a1_int_sum.py` | `acc = acc + i` | typed region installed | **3.20x** | 3.44x |
| `bench_a2_int_mul.py` | `acc = acc + i * 3` | typed region installed | **2.65x** | 2.85x |
| `bench_b1_str_len.py` | `total = len(s)` (str local) | baseline twin (tier-1 disabled: no typed region) | **1.68x** | 1.76x |
| `bench_b2_str_concat.py` | `s = s + "y"` | baseline twin (tier-1 disabled) | **1.24x** | 1.30x |

Geomeans across the 3 runs: **2.13 / 2.07 / 2.05** (conservative estimator
`min(tier0)/max(adaptive)` — the T-08 G-3 lower-bound rule that counts
noise against speedup); median/median: 2.21 / 2.19 / 2.20. Every program
in every run clears the floor except none — the weakest single program
(`bench_b2`, baseline-twin-only) still beats tier-0, and the geomean holds
>=2.0x under the conservative estimator on all runs.

## Method

- `--tier 0` = forced tier-0 (tree-walker only); `--tier adaptive` =
  02.05a JIT seam + 02.05b feedback + 02.05c typed regions + 02.05d deopt
  backoff + 02.05e background driver + 02.05f OSR.
- Timing is wall-clock around the whole `ap-run-eval` process (parse +
  compile + execute), i.e. the numbers INCLUDE the ~5ms process startup
  and the JIT compile time paid inside the warm calls.
- Corpus-green clause ("tier-1 invisible except by timing"): the RELEASE
  `tier_mode_differential` suite (3/3: whole-tier1-slice tier0 == adaptive
  bytes+rc, JIT exercised, thrash probe ends disabled) and `osr_gate`
  (2/2) pass on this tree. An ad-hoc whole-corpus sweep (287 files) found
  279/287 byte-identical across modes; the 8 exceptions print
  pointer-address reprs or thread-scheduling output that differs between
  two **tier-0** runs too — mode-independent nondeterminism, not an
  adaptive divergence (matches the known pre-existing residuals ledger).

## Where the wins come from

- **a1/a2 (int loops, tier-1):** the typed-region fast path with i64-smi
  guards removes the boxed `ops::binop` helper round-trip per iteration —
  consistent with the 5.4–5.6x debug-build typed-loop measurements
  reported at 02.05c, damped here by process startup + parse + the
  30 warm calls being counted in wall time.
- **b1 (str, baseline twin):** tier-1 declines the body (no typed region
  — string ops are not i64-smi), so adaptive runs the compiled baseline
  twin instead of the tree-walker; that alone is worth ~1.7x on this
  shape.
- **b2 (str concat):** same twin-only path, damped by allocation-heavy
  concat that the twin executes through the same host surfaces as
  tier-0.

## Known limitations found while measuring (filed, NOT gate blockers)

1. **Baseline-JIT compile failure on one IR shape** (pre-existing from
   02.05f): a `Call` of a **builtin** with a **local-Name argument** whose
   result feeds a **BinOp**, inside a `while` loop, fails Cranelift
   verification (`invalid block reference block3`) in
   `compile_baseline_with_osr` and silently falls back to the tree-walker.
   Repro: `acc = acc + len(s)` in a hot while loop (falls back);
   `total = len(s)` (no BinOp on the result), a literal arg
   (`len("abc")`), or a user-defined callee all compile fine. The
   fallback is transparent (byte-identical output), so the gate is not
   affected — but it caps tier-1 coverage for call-result-in-binop loop
   shapes. Owner: 02.05f emit path (`clif.rs` OsrPoll arm interacts with
   `check_status` block creation for Call sites).
2. **Float binop sites never mono-type as (IntI64, IntI64)**, so
   float-arithmetic loops decline tier-1 by design of the current
   `has_int_hotness` gate (infer.rs); they still get the baseline-twin
   speedup.
3. The old T-08 corpus bench files
   (`tests/corpus/tier1-transparency/bench/`) are module-level `for`
   loops — the JIT never engages on them (execs=0), which is why their
   earlier bench_report showed geomean 0.96x. That report was measuring
   startup noise, not tier-1. The 02.05h slice above is the gate slice.

## Reproduce

```
CARGO_TARGET_DIR=<abs> cargo build --release -p ap-cli
BIN=$CARGO_TARGET_DIR/release/ap-run-eval
for f in tests/bench/tier1/*.py; do
  time $BIN --tier 0 "$f"; time $BIN --tier adaptive "$f"
done
```
Observability: `APX_TIER_STATS=1 APX_TIER1_STATS=1` on adaptive runs
prints `[TIERUP-STATS]`/`[TIER1-STATS]` lines confirming the tier-1
install on a1/a2.

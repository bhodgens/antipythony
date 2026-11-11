# Leaf 02.05h — Bench gate (T12): >=1.5x geomean over tier-0

Status: LANDED on `tier1-0205h` (base = main 9d0a9f4, incl. 02.05a..02.05g).

## Verdict: GATE PASSES — 2.07x conservative geomean (floor 1.5x)

Machine: Mac M2 Ultra, 2026-11-11. Binary: `ap-run-eval` RELEASE
(`target-0205h/release`). 3 full runs × 9 timed reps per mode per program,
warmup first. Full numbers: `docs/tier1-bench.md` +
`tests/bench/tier1/bench_report.json`.

## What landed

- **`tests/bench/tier1/`** — the gate slice, four function-shaped programs
  (the JIT dispatch arm lives in `call_function`, so the slice must call a
  `def hot(n)`; module-level loops never reach `jit_call_body`):
  - `bench_a1_int_sum.py` — `acc = acc + i` int loop → tier-1 typed
    region installed, worst-case **3.20x**.
  - `bench_a2_int_mul.py` — `acc = acc + i * 3` → tier-1 installed,
    worst-case **2.65x**.
  - `bench_b1_str_len.py` — `total = len(s)` str loop → tier-1 declines
    (no typed region), runs the compiled baseline twin, worst-case
    **1.68x**.
  - `bench_b2_str_concat.py` — `s = s + "y"` → baseline twin, worst-case
    **1.24x**.
  - `bench_report.json` — raw timings + geomeans + verdict
    (`PASS`, conservative estimator `min(tier0)/max(adaptive)` per the
    T-08 G-3 lower-bound rule; median/median shown alongside: ~2.20x).
  - Each program makes 30 warm calls (`hot(2000)`) before the large timed
    call so the 02.05e background driver installs tier-1 mid-run — the
    same shape the transparency probes use.
- **`docs/tier1-bench.md`** — method, results table, where the wins come
  from, reproduce instructions, and the three limitations found while
  measuring (below).
- This PROGRESS file; PROGRESS-0205.md's T12 open item is closed by this
  leaf.

## Gate evidence

1. **Speed:** geomean across the slice = 2.13 / 2.07 / 2.05 (conservative)
   over 3 runs — >=1.5x on every run, >=2.0x on every run.
2. **Corpus green adaptive ("tier-1 invisible except by timing"):**
   `tier_mode_differential` 3/3 + `osr_gate` 2/2 green in RELEASE on this
   tree. Ad-hoc whole-corpus sweep (287 files, tier0 vs adaptive):
   279/287 byte-identical; the 8 exceptions print pointer-address reprs or
   thread-scheduling output that differ between two **tier-0** runs too —
   mode-independent nondeterminism (the known pre-existing residuals),
   not an adaptive divergence.

## Limitations found while measuring (filed; NOT gate blockers)

1. **Pre-existing 02.05f emit bug:** a builtin `Call` with a local-Name
   argument whose result feeds a BinOp inside a while loop fails Cranelift
   verification (`invalid block reference block3`) in
   `compile_baseline_with_osr` → transparent fallback to the tree-walker.
   Repro: `acc = acc + len(s)` in a hot loop (falls back); `total =
   len(s)`, literal args, or user-defined callees all compile. Byte
   output unaffected. Owner: 02.05f `clif.rs` (OsrPoll arm × `check_status`
   block creation on Call sites).
2. Float binop sites never reach the (IntI64, IntI64) hotness gate, so
   float loops decline tier-1 by design (`has_int_hotness`, infer.rs);
   they still get the baseline-twin speedup.
3. The OLD T-08 bench corpus (`tests/corpus/tier1-transparency/bench/`)
   is module-level `for` loops — JIT execs=0 there, so its historical
   0.96x "bench_report" measured startup noise, not tier-1. The 02.05h
   slice above is the gate slice.

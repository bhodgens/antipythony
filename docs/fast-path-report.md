# Fast path — bytecode dispatch, inline caches, fused opcodes, tier seam

**Leaf:** `plans/fast-path/03-rollout/02-bench-parity` (branch `fp-bench-parity`,
worktree `~/.git/worktrees/leaf-fp-bench`, base `e00fcb4`).
**Closing measurement:** 2026-11-28T00:35:26Z, fresh release binary
`target-fp-bench/release/antipythony`, SHA-256
`068cc7f20c70d167c268be17c787e6bf35cc986661d8406a45746bc186914160`
(mtime 2026-11-27 17:34 MST; no source file newer than the binary).
**Single source of truth:** `tests/bench/primitive/bench_report.json` (64 rows,
0 ERROR). Both charts are generated from that JSON with the committed
generators — no number in this report is hand-transcribed from a chart.

## Verdict up front

The master plan's exit gate is **NOT MET**:

| gate clause | required | measured | verdict |
|---|---|---|---|
| `dict.getitem` / `dict.build`, `list.sort` / `sorted`, `str.join` / `str.split`, `json.loads`: no row below 1.0x CPython | all >= 1.0x | **all 18 walk-bound rows below 1.0x** (0.112x – 0.849x) | **FAIL** |
| simple mean ratio vs CPython | >= 1.5x | **3.292x** | PASS |

All byte-exactness gates are green in both tree0 and bytecode modes.
The primitive bench in bytecode mode produced **64 rows: 28 PASS / 36 FAIL /
0 ERROR**. Chart generators and gate scripts were not modified; the JSON was
not doctored. No default tier was flipped in this leaf (that remains the
USER GATE in `03-rollout/01`).

## Architecture (what the fast path is)

Pipeline: `ruff AST -> ap-ir (Expr tree) -> ap-bytecode (flat Vec<Instr>,
const pool, line table, site table, exception table) -> BytecodeInterp`.

- **Bytecode dispatch** (`crates/ap-bytecode`, `crates/ap-run/src/bytecode_interp.rs`):
  lowerer emits a flat instruction stream; a non-recursive dispatch loop
  executes it with locals in compile-time-indexed slots. The ruff AST and all
  semantics live in shared helpers (`ap-obj` ops, `methods.rs`, interp
  helpers), so the tree-walker and bytecode interpreter call the SAME
  functions — no semantic duplication. Line bookkeeping is a sparse line
  table consulted on exception/warning paths instead of per-node
  `last_line.set()`. Unsupported shapes lower to `FallbackToTreeWalk` at
  function granularity (a loud `APX_BP_DEBUG=1` WARN, never a silent
  miscompile). Generators/async and dynamic exec/eval stay on the tree-walker.
- **Inline caches** (02-perf/01): `LoadGlobal` caches resolved values keyed by
  globals/builtins dict epoch + identity; `LoadAttr` by class payload ptr,
  `type_version`, `surgery_version` and names-dict epoch; `LoadAttrCall`
  caches the (tag, name, argc) dispatcher decision. Caches are per-thread,
  hits take no lock and perform no shared writes, revalidating stamps on
  every hit; fills re-probe the live instance dict to defeat shadowing.
  Measured IC leaf microbench: global read 314ns -> ~57ns (CPython 19ns),
  instance attr 571ns -> ~136ns (CPython 21ns); resolution cost removed,
  loop dispatch floor remains.
- **Fused opcodes / superinstructions** (02-perf/02): an `APX_BP_STATS=1`
  histogram over all primitive probes found the hot triples; three
  histogram-backed fusions (`LoadConst2BinOp` 9.24M occurrences,
  `LoadFastConstBinOp` 2.35M, `LoadFast2BinOp` 0.42M) were added with a
  guarded Smi fast path (`fast_smi_binop`) and generic `eval_binop`
  fallback. Function-shaped microbench wins: 1.12-1.16x over the un-fused
  bytecode binary; four higher-count triples are documented as NOT fused
  (load shuffles / method-call statements — no one-boundary safe rewrite).
- **Per-function tier seam** (03-rollout prerequisite): `TIER_MODE` plus a
  process-global `OnceLock<HashMap<fn ptr, SeamTier>>` override installed
  from `APX_TIER_SEAM_OVERRIDE="bytecode:outer,adaptive:mid,bytecode:inner"`.
  One effective tier is resolved per call from function identity, so a
  bytecode fn can call a tier-1 (JIT) fn can call a bytecode fn in one
  process, under threads. Mixed chain proven byte-identical to CPython with
  `[TIER-SEAM-STATS] bytecode_execs=62 adaptive_execs=31 tree_execs=0`,
  `[TIERUP-STATS] tier1=1`. No override spec = zero hot-path cost; default
  mode unchanged (tree0) — the default flip is a USER GATE.

## Method — reproduction conditions

```sh
cd /Users/caimlas/.git/worktrees/leaf-fp-bench
# one heavy lane, CARGO_BUILD_JOBS=4; fresh target dir (stale-fingerprint gotcha)
env -u APX_TIER -u APX_TIER_STATS -u APX_TIER_SEAM_OVERRIDE \
  CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/target-fp-bench \
  cargo build --release -p ap-cli -p ap-conform -p ap-run --bins
stat -f '%Sm %N' target-fp-bench/release/antipythony   # 2026-11-27 17:34 MST
shasum -a 256 target-fp-bench/release/antipythony      # 068cc7f2...160

# FULL 64-row bench — bytecode is the MODE UNDER TEST, so APX_TIER is SET;
# only the diagnostic env is unset (APX_TIER_STATS would add stderr noise):
env -u APX_TIER_STATS -u APX_TIER_SEAM_OVERRIDE -u APX_BP_STATS \
  -u APX_OSR_STATS -u APX_CALLDBG APX_TIER=bytecode \
  python3.14 tests/bench/primitive/run_bench.py \
    --antipythony $PWD/target-fp-bench/release/antipythony \
    --python /opt/homebrew/bin/python3.14 \
    --out tests/bench/primitive/bench_report.json

# both charts, straight from the new JSON:
python3 tests/bench/primitive/gen_chart.py
python3 tests/bench/primitive/gen_speedup_chart.py
```

- Machine: Mac M2 Ultra (24-core aarch64, NEON), loadavg 7.55 at bench launch
  (quiet-window gate < 10), 35 GB free on `/` (97% used — one target dir,
  no `rm -rf`).
- Reference: `/opt/homebrew/bin/python3.14` = CPython 3.14.7; best-of-5 min
  timing inside each probe; 48 KiB + 1 MiB payloads (P10 JSON buckets
  ~70 KB / ~1.03 MB; P09 sizes are element counts).
- Verdict semantics: PASS = met the row's `target_gbs` (CPython-as-shipped
  for most rows; the M5-Max PR #162443 24/14 GB/s line for b64encode/decode);
  FAIL = measured below target. `ratio_vs_cpython` = antipythony ÷ CPython
  GB/s. Ratio 1.0x CPython is NOT the same as PASS for the four b64 rows.
- Bytecode engagement was proven, not assumed: under the same probe the
  fresh binary emitted `[BC-STATS] compiles=6 fallbacks=3 execs=79` while
  `[TIER-STATS] execs=0` — the bytecode loop really executed, with 3
  functions falling back to the tree-walker (an honest coverage gap).

## Full results — tree0 vs bytecode vs CPython

Tree0 numbers are the previous stored report (`docs/primitive-perf-report.md`,
generated 2026-11-25/26, tier-0 tree-walker, 26 PASS / 38 FAIL / 0 ERROR).
Bytecode numbers are the NEW JSON. Both are best-of-5 on this host; run-to-run
spread is a known caveat (see the gaps table), so improvements below ~10% on
a single run are not claimed as wins.

| id | primitive | size | tree0 vs CPython | bytecode vs CPython | bytecode GB/s (cpy) |
|----|-----------|------|-----------------:|--------------------:|--------------------:|
| P01 | base64.b64encode | 48KiB | 14.51x | 12.699x | 9.59 (0.76) |
| P01 | base64.b64decode | 48KiB | 17.04x | 16.838x | 7.15 (0.42) |
| P01 | binascii.b2a_base64 | 48KiB | 10.71x | 10.000x | 7.61 (0.76) |
| P01 | binascii.a2b_base64 | 48KiB | 13.14x | 12.739x | 5.41 (0.42) |
| P01 | base64.b64encode | 1MiB | 24.36x | 23.749x | 17.21 (0.72) |
| P01 | base64.b64decode | 1MiB | 24.83x | 23.891x | 10.57 (0.44) |
| P01 | binascii.b2a_base64 | 1MiB | 12.30x | 11.776x | 8.57 (0.73) |
| P01 | binascii.a2b_base64 | 1MiB | 20.16x | 19.817x | 8.64 (0.44) |
| P03 | binascii.hexlify | 48KiB | 1.05x | 1.010x | 1.68 (1.66) |
| P03 | binascii.unhexlify | 48KiB | 2.58x | 2.514x | 4.27 (1.70) |
| P03 | binascii.hexlify | 1MiB | 1.21x | 1.170x | 1.95 (1.67) |
| P03 | binascii.unhexlify | 1MiB | 3.74x | 3.664x | 6.44 (1.76) |
| P04 | zlib.compress(level=1) | 48KiB | 2.12x | 2.012x | 2.95 (1.47) |
| P04 | zlib.compress(level=6) | 48KiB | 2.77x | 2.576x | 1.51 (0.59) |
| P04 | zlib.decompress(level=1) | 48KiB | 1.38x | 1.260x | 6.14 (4.87) |
| P04 | zlib.decompress(level=6) | 48KiB | 1.05x | 0.976x | 4.66 (4.78) |
| P04 | zlib.compress(level=1) | 1MiB | 4.54x | 4.587x | 3.71 (0.81) |
| P04 | zlib.compress(level=6) | 1MiB | 7.34x | 7.140x | 3.09 (0.43) |
| P04 | zlib.decompress(level=1) | 1MiB | 1.30x | 1.313x | 11.08 (8.43) |
| P04 | zlib.decompress(level=6) | 1MiB | 1.38x | 1.359x | 11.45 (8.43) |
| P05 | hashlib.sha256 | 48KiB | 0.67x | 0.653x | 1.61 (2.46) |
| P05 | hashlib.md5 | 48KiB | 0.87x | 0.863x | 0.62 (0.71) |
| P05 | hashlib.sha256 | 1MiB | 0.84x | 0.847x | 2.13 (2.51) |
| P05 | hashlib.md5 | 1MiB | 0.92x | 0.914x | 0.66 (0.72) |
| P06 | str.find | 48KiB | 3.26x | 3.127x | 5.56 (1.78) |
| P06 | str.replace | 48KiB | 0.95x | 0.993x | 8.19 (8.25) |
| P06 | str.split | 48KiB | 0.47x | 0.451x | 0.70 (1.55) |
| P06 | str.join | 48KiB | 0.20x | 0.193x | 2.65 (13.72) |
| P06 | str.upper | 48KiB | 2.85x | 2.648x | 8.49 (3.21) |
| P06 | str.find | 1MiB | 4.43x | 4.431x | 7.95 (1.79) |
| P06 | str.replace | 1MiB | 1.89x | 1.915x | 15.92 (8.31) |
| P06 | str.split | 1MiB | 0.51x | 0.509x | 0.82 (1.61) |
| P06 | str.join | 1MiB | 0.22x | 0.230x | 3.15 (13.69) |
| P06 | str.upper | 1MiB | 4.13x | 4.178x | 13.39 (3.21) |
| P07 | int.from_bytes | 48KiB | 1.83x | 1.556x | 1.34 (0.86) |
| P07 | int.to_bytes | 48KiB | 0.76x | 2.652x | 3.48 (1.31) |
| P07 | int(decimal_str) | 48KiB | 2.29x | 2.510x | 0.024 (0.0096) |
| P07 | hex(big_int) | 48KiB | 0.79x | 0.789x | 0.41 (0.52) |
| P07 | int.from_bytes | 1MiB | 2.45x | 2.480x | 2.10 (0.84) |
| P07 | int.to_bytes | 1MiB | 0.80x | 3.896x | 4.99 (1.28) |
| P07 | int(decimal_str) | 1MiB | 0.75x | 4.333x | 0.0065 (0.0015) |
| P07 | hex(big_int) | 1MiB | 0.81x | 0.781x | 0.42 (0.53) |
| P08 | sorted(list[int]) | 48KiB | 0.36x | 0.828x | 0.049 (0.059) |
| P08 | list.sort | 48KiB | 0.12x | 0.168x | 0.0032 (0.019) |
| P08 | sorted(list[int]) | 1MiB | 0.39x | 0.849x | 0.035 (0.041) |
| P08 | list.sort | 1MiB | 0.12x | 0.112x | 0.0019 (0.017) |
| P08 | sorted(list[int])_1M | 1MiB | 0.42x | 0.811x | 0.025 (0.031) |
| P08 | list.sort_1M | 1MiB | 0.14x | 0.146x | 0.0020 (0.0137) |
| P09 | dict.build | 48KiB | 0.13x | 0.181x | 0.081 (0.446) |
| P09 | dict.get | 48KiB | 0.01x | 0.239x | 0.065 (0.273) |
| P09 | dict.getitem | 48KiB | 0.06x | 0.205x | 0.075 (0.367) |
| P09 | dict.iter | 48KiB | 0.07x | 0.255x | 0.150 (0.587) |
| P09 | dict.build | 48KiB | 0.12x | 0.192x | 0.085 (0.441) |
| P09 | dict.get | 48KiB | 0.01x | 0.280x | 0.065 (0.233) |
| P09 | dict.getitem | 48KiB | 0.07x | 0.267x | 0.075 (0.282) |
| P09 | dict.iter | 48KiB | 0.08x | 0.263x | 0.151 (0.575) |
| P09 | dict.build_32k | 48KiB | 0.14x | 0.192x | 0.084 (0.439) |
| P09 | dict.get_32k | 48KiB | 0.01x | 0.279x | 0.065 (0.234) |
| P09 | dict.getitem_32k | 48KiB | 0.07x | 0.268x | 0.075 (0.281) |
| P09 | dict.iter_32k | 48KiB | 0.07x | 0.260x | 0.151 (0.582) |
| P10 | json.dumps | 48KiB | 1.06x | 1.002x | 0.214 (0.213) |
| P10 | json.loads | 48KiB | 0.46x | 0.509x | 0.102 (0.201) |
| P10 | json.dumps | 1MiB | 1.09x | 1.060x | 0.231 (0.218) |
| P10 | json.loads | 1MiB | 0.31x | 0.289x | 0.051 (0.176) |

**Aggregate (all 64 bytecode rows):** simple mean vs CPython **3.292x**,
geomean vs CPython **1.180x**; simple mean vs target 2.123x, geomean vs
target 0.949x. The simple mean passes the >= 1.5x clause; the geomean is
included for honesty (the four b64 rows are 10-24x and dominate the mean).

## Exit gate — exact row ratios (master.md)

| row | ratios measured (bytecode / CPython) | below 1.0x? |
|---|---|---|
| `dict.getitem` (3 arms) | 0.205x, 0.267x, 0.268x | **yes, all 3** |
| `dict.build` (3 arms) | 0.181x, 0.192x, 0.192x | **yes, all 3** |
| `list.sort` (3 arms) | 0.168x, 0.112x, 0.146x | **yes, all 3** |
| `sorted(list[int])` (3 arms) | 0.828x, 0.849x, 0.811x | **yes, all 3** |
| `str.join` (2 sizes) | 0.193x, 0.230x | **yes, both** |
| `str.split` (2 sizes) | 0.451x, 0.509x | **yes, both** |
| `json.loads` (2 sizes) | 0.509x, 0.289x | **yes, both** |

18 of 18 walk-bound rows are below 1.0x. The exit gate's parity clause is
RED; no number was weakened to change that. The bytecode path improved dict
rows materially versus the stale tree0 report (dict.get ~0.01x -> 0.24-0.28x,
dict.getitem ~0.06-0.07x -> 0.20-0.27x) and sorted() (0.36-0.42x ->
0.81-0.85x), but `list.sort` stayed at 0.11-0.17x and join/split/json.loads
did not move materially. This matches the parent's first-hand P08/P09
spot-check on a separate fresh binary (tree0 sorted() 1MiB 0.80x vs bytecode
1.00x then; dict.get tree0 0.01x vs bytecode 0.21x then) within run-to-run
noise.

**Why the gap persists (measured, not excused):** the primitive probe hot
loops are MODULE-LEVEL. Module-level code never routes through
`call_function` (the tier seam's entry point), so `APX_TIER=bytecode` does
NOT put these loops on the bytecode dispatch loop for most rows — they
execute the same tree-walker as tier0. `[BC-STATS]` on the probes shows
function-shaped bodies (the `emit`/`make_data`/`build` helpers) DO run
bytecode (79 execs on P09), but the timed inner loops are mostly in
module-level frames. Fusions therefore cannot lift these rows (cf.
02-perf/02 §4, "primitive rows are NOISE — module-level loops never dispatch
bytecode"). The dict improvement comes from the function-shaped helper
bodies (`get_lookups`, `getitem_lookups`) now executing in bytecode; the
sub-1.0x floor is the same interpreter/registry cost documented in the
primitive-perf campaign, not a bytecode regression.

## Honest gaps and filed follow-ups

| rows | gap | disposition / filed follow-up |
|---|---|---|
| P09 dict ×12 | 0.18-0.28x CPython (improved from 0.01-0.14x) | Still tier-0/dispatch-bound: ~1.5us BoundMethod mint per `d.get` step + ~190ns/step walk floor; `dict.build`/`getitem` need function-shaped probe loops to ride bytecode. Filed: batching/nursery registry insert, lazy BoundMethod mint, module-level bytecode dispatch entry (tier seam extension). |
| P08 list.sort ×3 + sorted ×3 | 0.11-0.17x / 0.81-0.85x CPython | `list.sort` remains dispatch-bound in the sort compare loop (homogeneous int fast-path arm NOT implemented); `sorted()` improved to ~0.8x but still sub-1.0x. Filed: homogeneous int-list compare arm (audit Fix B). |
| P06 str.join ×2 | 0.19-0.23x | Generic-iterable join path (per-item PyString walk); homogeneous list-of-str bulk join not written. Filed (primitive-perf audit). |
| P06 str.split ×2 | 0.45-0.51x | Per-piece Value/Gc allocation. Filed (primitive-perf audit). |
| P10 json.loads ×2 | 0.29-0.51x | Byte-at-a-time recursive descent vs CPython `_json` C scanner; ~985ns per-value registry insert dominates. memchr-based span copies + nursery filed (new-dep user decision). |
| P04 zlib.decompress(level=6) @48KiB | 0.976x (was 1.05x in tree0 report) | Run-to-run noise around parity (zlib-rs vs system zlib); 1MiB arm stays 1.359x PASS. No regression claimed from a single 2% drift. |
| P05 sha256 ×2 / md5 ×2 | 0.65-0.91x | OpenSSL-class backend class (sha2 asm-aarch64 landed; full parity needs an OpenSSL-class backend = new-dep user decision); md5 near-parity, no aarch64 HW path in any Rust crate. |
| P01 b64encode/decode ×4 | 12.7-23.9x CPython but FAIL vs target | Target is upstream PR #162443 measured on **M5 Max**; this M2 Ultra cannot reach 24/14 GB/s. Hardware caveat, filed. |
| P07 hex(big_int) ×2 | 0.78-0.79x | W7 GC/iter changes traded this row; top re-benchmark + fix candidate from the primitive-perf report. |
| Bytecode coverage: 3 fallbacks on P09 | BC-STATS compiles=6 fallbacks=3 execs=79 | Unsupported shapes fall back to tree-walker by design (loud WARN under APX_BP_DEBUG). Extending lowerer coverage is separate leaf work. |
| Default tier flip | not done | USER GATE in `03-rollout/01`; this leaf only measures candidate bytecode mode. |

## Gates (final tree, clean env)

All gates below were run on `target-fp-bench` with
`env -u APX_TIER -u APX_TIER_STATS -u APX_TIER_SEAM_OVERRIDE -u APX_BP_STATS
-u APX_OSR_STATS -u APX_CALLDBG` (the bench leg alone deliberately SET
`APX_TIER=bytecode` — it is the mode under test):

| gate | result |
|---|---|
| `fix_bytecode.py --other <fresh antipythony>` | 13 cases, 0 mismatches |
| `diff_b64.py` | 2132 cases, 0 mismatches |
| `diff_hex.py` | 1976 cases, 0 mismatches |
| `fix_int.py` (CPython vs engine) | 18653 lines, byte-identical |
| `fix_json.py` | 127 cases, 0 mismatches |
| `fix_sort.py` | 42 cases, 0 mismatches |
| `fix_random.py` | pass=true, rc=0 |
| `fix_sha_zlib.py` (818 lines) | tree0 vs bytecode: **0 diff lines**; vs CPython: 62 diff lines (31 CORRUPT rows), ALL the documented zlib-rs corrupt-message residual — zero non-CORRUPT divergences |
| `ap-conform values` tier0 / bytecode | 50/50 / 50/50 |
| `ap-conform stdlib` tier0 / bytecode | 48/48 / 48/48 |
| `cargo test -p ap-bytecode -p ap-run --release` | 22 result blocks (20 suite binaries + 2 doc-tests), 282 passed / 0 failed / 1 ignored (incl. 3 tier_seam_differential + 9 ic_invalidation) |
| `tier_seam_differential` / `ic_invalidation` (subset) | 3/3 / 9/9 |

Logs were captured worktree-locally (`.fp-bench-run.log`, `.fp-diff-gates.log`,
`.fp-sha-conform.log`, `.fp-cargo-test.log`, `.fp-build-release.log`) during
the run and removed before commit; the results are transcribed above.

## Files generated / updated

- `tests/bench/primitive/bench_report.json` — regenerated (64 rows, 0 ERROR).
- `docs/primitive-perf-chart.svg` — regenerated by `gen_chart.py`.
- `docs/fast-path-speedup-vs-cpython.svg` — regenerated by
  `gen_speedup_chart.py`.
- `docs/fast-path-report.md` — this report (new).
- `README.md` perf section + `docs/benefits-vs-cpython.md` — bytecode-mode
  figures and honest gate status.
- `plans/fast-path/03-rollout/NEXT-FP-BENCH.md` — leaf note.

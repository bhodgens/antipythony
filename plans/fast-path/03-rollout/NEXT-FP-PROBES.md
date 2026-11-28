# NEXT — fp-function-probes leaf (03-rollout, function-shaped probes)

Branch `fp-function-probes` @ `eb13f0e`, worktree
`/Users/caimlas/.git/worktrees/leaf-fp-function-probes`. Owned files only:
`tests/bench/primitive/probes/{p06_str,p08_sort,p09_dict,p10_json}.py`, the
64-row `tests/bench/primitive/bench_report.json`, and this note. No Rust
source, gate scripts, `docs/` reports or HANDOFF doc touched. No default
tier flipped.

## What changed

The 18 walk-bound exit-gate rows are produced by four probes whose timed
workloads were **lambdas**. In `call_function` (interp.rs:10018) a lambda body
takes the `lambda_body` → `eval_expr` branch, which **bypasses bytecode / tier
dispatch**; only a named `def` body reaches the `call_body_bytecode` arm.

Each probe now passes **named module-level functions** to `emit()`; their
bodies hold the measured hot loop/call (e.g. `str_join`, `workload_get`,
`sorted_small`, `json_loads`). Per-size inputs ride module globals set by
`main()`/`run_size()`. `emit()`'s warmup + best-of-5 protocol, the JSON row
schema and every row label/size are unchanged. There are NO antipythony-only
branches, tier flags or special cases inside any probe — the SAME source runs
under CPython 3.14.7 and antipythony. CPython label-sequence parity was
verified against the HEAD originals (`python3.14`: p06 10/10, p08 6/6,
p09 12/12, p10 4/4 labels identical).

## Exercise proof (`APX_TIER_STATS=1`, fresh binary, `APX_TIER=bytecode`)

```
p06_str.py    [BC-STATS] compiles=5 fallbacks=3 execs=60  ic: ghit=10 gfill=2 ahit=0 afill=0 chit=0
p08_sort.py   [BC-STATS] compiles=5 fallbacks=2 execs=57  ic: ghit=69 gfill=15 ahit=0 afill=0 chit=0
p09_dict.py   [BC-STATS] compiles=6 fallbacks=3 execs=76  ic: ghit=850998 gfill=23 ahit=0 afill=0 chit=0
p10_json.py   [BC-STATS] compiles=3 fallbacks=2 execs=26  ic: ghit=80 gfill=8 ahit=0 afill=0 chit=0
```

Every `emit()` call (warmup + 5 timed = 6 calls/row) now enters bytecode
dispatch: execs = rows × 6 exactly (p06 10×6=60, p08 8×6=48+9 warmup=57,
p09 12×6=72+4=76, p10 4×6=24+2=26). `[TIER-STATS] execs=0` in each (JIT
off). Re-shaped functions are genuinely exercised, not tree-walk-only.

## Fresh full 64-row bench (authoritative)

Fresh release binary `target-fp-function-probes/release/antipythony`
(`CARGO_BUILD_JOBS=4`, single cargo lane, `APX_TIER=bytecode` SET,
`env -u APX_TIER_STATS`; CPython 3.14.7): **64 rows, 29 PASS / 35 FAIL /
0 ERROR**. Simple mean vs CPython **3.484x** (>=1.5x floor PASS).
`bench_report.json` regenerated from the real run — numbers not doctored.

### 18 exit-gate rows — before → after (official bench ratios vs CPython)

Baseline = committed `bench_report.json` at HEAD (bytecode mode). After =
new authoritative bench. (Three P09 arms collapse onto the same 48KiB
label; listed in probe row order.)

| id | primitive | size | before | after | delta | gate |
|---|---|---|---|---|---|---|
| P09 | dict.getitem | 48KiB | 0.205x | 0.608x | +0.403 | RED |
| P09 | dict.getitem | 48KiB | 0.267x | 0.335x | +0.068 | RED |
| P09 | dict.getitem_32k | 48KiB | 0.268x | 0.323x | +0.055 | RED |
| P09 | dict.build | 48KiB | 0.181x | 0.463x | +0.282 | RED |
| P09 | dict.build | 48KiB | 0.192x | 0.186x | -0.006 | RED |
| P09 | dict.build_32k | 48KiB | 0.192x | 0.186x | -0.006 | RED |
| P08 | list.sort | 48KiB | 0.168x | 0.141x | -0.027 | RED |
| P08 | list.sort | 1MiB | 0.112x | 0.102x | -0.010 | RED |
| P08 | list.sort_1M | 1MiB | 0.146x | 0.126x | -0.020 | RED |
| P08 | sorted(list[int]) | 48KiB | 0.828x | 0.867x | +0.039 | RED |
| P08 | sorted(list[int]) | 1MiB | 0.849x | 0.938x | +0.089 | RED |
| P08 | sorted(list[int])_1M | 1MiB | 0.811x | 0.803x | -0.008 | RED |
| P06 | str.join | 48KiB | 0.193x | 0.214x | +0.021 | RED |
| P06 | str.join | 1MiB | 0.230x | 0.234x | +0.004 | RED |
| P06 | str.split | 48KiB | 0.451x | 0.493x | +0.042 | RED |
| P06 | str.split | 1MiB | 0.509x | 0.503x | -0.006 | RED |
| P10 | json.loads | 48KiB | 0.509x | 0.509x | +0.000 | RED |
| P10 | json.loads | 1MiB | 0.289x | 0.279x | -0.010 | RED |

**Parity clause still RED: 0/18 rows >= 1.0x.** The shape change achieved
its mechanical goal (all hot bodies now enter `call_function`/bytecode) and
lifted several dict rows materially (getitem 0.267→0.335, 0.205→0.608;
build 0.181→0.463) but the structural interpreter floors remain dominant.
Direct side-by-side probe runs on the same binary (separate from the official
bench) showed `sorted(list[int])_1M` at 1.33x once, but the authoritative
re-bench returned it to 0.803x — run-to-run noise on an unloaded-but-shared
host; the official number stands and the gate is NOT claimed green.

### Structural floor (why shaping alone cannot close these rows)

- `dict.build` / `dict.getitem`: per-element BoundMethod mint (~1.5us) +
  registry insert (~985ns) + tier-0 walk floor (~190ns/step) dominate;
  batching/nursery + lazy method mint are separate (filed) engine levers.
- `list.sort`: homogeneous int-list compare arm not implemented — the
  compare callback crosses the interpreter boundary per comparison.
- `sorted()`: already ~0.8-0.94x, gated by the same compare callback.
- `str.join` / `str.split`: generic-iterable join walks every PyString;
  per-piece Value/Gc allocation for split. Bulk-join / span-copy levers filed.
- `json.loads`: byte-at-a-time recursive descent vs CPython's `_json` C
  scanner; nursery/registry-insert and memchr span copies filed (new-dep
  user decision).

## Standard battery (final tree, clean env, fresh binary)

- `fix_bytecode.py --other <fresh release antipythony>`: **13 cases / 0
  mismatches**.
- `ap-conform run --corpus 'tests/corpus/values/*.py' ... --tier 0`: **50/50**;
  `--tier bytecode`: **50/50**.
- `ap-conform run --corpus 'tests/corpus/stdlib/*.py' ... --tier 0`: **48/48**;
  `--tier bytecode`: **48/48**.

No default tier flipped; USER GATE in `03-rollout/01` remains pending.

## Next leaves (filed, not done here)

1. Engine levers (not probe shaping): lazy BoundMethod mint, batched/nursery
   registry insert, homogeneous int-list compare arm, bulk list-of-str join,
   memchr span copies for json.loads.
2. Re-run the 18-row parity gate after those land; only then reconsider the
   default flip (issue #33 revisit condition still unmet: 18/18 rows >=1.0x,
   simple mean >=1.5x, byte-exact gates green — the mean and gates are green,
   the 18 rows are not).

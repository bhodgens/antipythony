# fp-superinstructions — profile + measured report

Branch `tier1-fp-superinst`, from main `fe7cc86`. Worktree
`/Users/caimlas/.git/worktrees/leaf-fp-superinst`.

## 1. Profile FIRST — `APX_BP_STATS=1` opcode histogram

Implementation: `crates/ap-bytecode/src/stats.rs` (pure counter: singles +
adjacent fall-through pairs/triples) wired into the dispatch loop in
`crates/ap-run/src/bytecode_interp.rs`. One env read per frame; when the
knob is off the only cost is one predicted branch (no atomics, locks,
allocations). Emitted to stderr at process exit via the existing
`report_line()` exit hook (`crates/ap-run/src/jit_hook.rs`).

Command (env-sanitized, 65 probe files):

```
env -u APX_TIER_STATS APX_BP_STATS=1 APX_TIER=bytecode \
  target-fp-superinst/release/antipythony run <probe>.py
```

Raw unedited output for every `tests/bench/primitive/probes/*.py` and
every `tests/**/*ic*.py` probe: `superinstructions-histogram-raw.txt`
(644 lines, committed). Aggregate: pairs summed 207,994,700; triples
178,755,934.

### Top 10 singles (summed over all probe runs)

| opcode | count |
|---|---:|
| LoadFast | 52,928,310 |
| StoreFast | 40,144,228 |
| LoadConst | 37,353,762 |
| Jump | 23,842,710 |
| ForNext | 21,519,596 |
| LoadAttrCall | 19,635,769 |
| BinOp | 15,010,859 |
| PopTop | 11,280,554 |
| AugOp | 9,424,192 |
| LoadGlobal | 7,762,328 |

### Top 10 pairs (summed over all probe runs)

| pair | count |
|---|---:|
| StoreFast → LoadFast | 23,394,651 |
| LoadFast → LoadFast | 22,615,142 |
| LoadConst → LoadConst | 17,467,546 |
| LoadFast → LoadConst | 15,897,339 |
| LoadConst → BinOp | 12,877,339 |
| PopTop → Jump | 10,407,821 |
| LoadAttrCall → PopTop | 10,276,163 |
| StoreFast → Jump | 9,950,227 |
| AugOp → StoreFast | 9,530,688 |
| BinOp → LoadAttrCall | 7,960,512 |

### Top 10 triples (summed over all probe runs)

| triple | count |
|---|---:|
| StoreFast → LoadFast → LoadFast | 18,534,892 |
| LoadAttrCall → PopTop → Jump | 10,265,857 |
| LoadConst → LoadConst → BinOp | 9,241,600 |
| AugOp → StoreFast → Jump | 8,522,496 |
| LoadConst → LoadConst → LoadConst | 8,112,340 |
| LoadFast → LoadFast → LoadConst | 8,072,512 |
| LoadFast → LoadConst → LoadConst | 7,968,860 |
| LoadConst → BinOp → LoadAttrCall | 7,960,512 |
| BinOp → LoadAttrCall → LoadAttrCall | 7,960,512 |
| LoadAttrCall → LoadAttrCall → PopTop | 7,960,512 |

## 2. Data-driven shortlist → fusions implemented

Only triples ending in `BinOp` were fused (the only family where the
loader already emits a 3-instruction sequence with an obvious one-boundary
equivalent). Other top triples are load/store shuffles or method-call
statement shapes — different families, no safe one-boundary rewrite
without new semantics; documented as honest gaps below.

| histogram triple | count | fused opcode |
|---|---:|---|
| LoadConst → LoadConst → BinOp | 9,241,600 | `LoadConst2BinOp` |
| LoadFast → LoadConst → BinOp | 2,354,648 | `LoadFastConstBinOp` |
| LoadFast → LoadFast → BinOp | 423,602 | `LoadFast2BinOp` |

Peephole (`crates/ap-bytecode/src/lower.rs::peephole_fuse_binop_triples`)
runs AFTER jump backpatching, refusesto fuse across a pinned (jump-target)
instruction, and remaps every jump target, line-table index, site
instr_idx and exception-table range through one `old→new` table.
`CodeObject::validate` checks const-pool bounds on the fused ops.

### Guard / no-miscompile discipline

All three fused arms call one helper, `fused_binop` in
`crates/ap-run/src/bytecode_interp.rs`. The type specialization is a
Smi/Smi check: `fast_smi_binop(&a, op, &b)` answers only for two Smi Ints;
EVERY other operand shape (bigint, float, str, list, user `__add__` etc.)
falls through to `crate::interp::eval_binop` — the exact helper the
unfused `BinOp` arm calls. The guard IS the semantics, so there is no
unprotected specialization and no miscompile surface. Unbound local reads
in the fused-local arms reproduce the tree-walker's exact
`UnboundLocalError` text.

### Proof the fused opcodes ran (not just claimed)

`APX_BP_STATS=1` single-shape runs:

```
/tmp/bp_constconst.py : LoadConst2BinOp=2,340,000  LoadFastConstBinOp=2,340,000
/tmp/bp_micro.py      : LoadFastConstBinOp=11,700,000  LoadFast2BinOp=4,680,000
```

Every speedup below is credited ONLY to a fused opcode observed at >0 in
these histograms — the false-credit pattern from the IC leaf's chit=0
incident is explicitly avoided.

## 3. Fusions kept / dropped — measured before/after

Baseline binary: `/tmp/antipythony-bp-baseline` (copied from
`target-fp-superinst/release/antipythony` at commit `ea23b54`, histogram
infra only, no fusions). Fused binary: rebuilt at HEAD.

Commands (5 fresh process launches per mode, best-of-9 inside each):

```
env -u APX_TIER_STATS -u APX_BP_STATS APX_TIER=bytecode \
  <binary> run /tmp/bp_micro.py       # const_binop, two_fast, store_load
env -u APX_TIER_STATS -u APX_BP_STATS APX_TIER=bytecode \
  <binary> run /tmp/bp_constconst.py  # const_const
```

| shape (fused opcode) | baseline median | fused median | speedup | kept? |
|---|---:|---:|---:|---|
| const_binop (`LoadFastConstBinOp`) | 0.02685158s | 0.02327733s | **1.154x** | KEPT |
| two_fast (`LoadFast2BinOp` + `LoadFastConstBinOp`) | 0.03182029s | 0.02752321s | **1.156x** | KEPT |
| store_load (`LoadFast2BinOp` + `LoadFastConstBinOp`) | 0.02941683s | 0.02578771s | **1.141x** | KEPT |
| const_const (`LoadConst2BinOp`) | 0.02625450s | 0.02344421s | **1.120x** | KEPT |

Raw 5-run numbers: `/tmp/bp_rigorous.txt` (captured in this session; all
four rows paid, so NO fusion was dropped). The honest-stop contract did
not fire — no candidate resisted two attempts.

### Honest gaps (histogram candidates NOT fused)

- `StoreFast → LoadFast → LoadFast` (18.53M) and `LoadConst → LoadConst →
  LoadConst` (8.11M): pure load shuffles. The interpreter's `StoreFast`
  consumes a stack value and `LoadFast` clones from `locals`; collapsing
  them into one opcode would still need the same clone/store work, so the
  dispatch saving does not obviously pay. Not attempted — no measured
  before/after, no claim.
- `LoadAttrCall → PopTop → Jump` (10.27M) and `BinOp → LoadAttrCall →
  LoadAttrCall` (7.96M): method-call statement shapes. `LoadAttrCall` is
  ALREADY a fused opcode from leaf 01-core/01; `PopTop`/`Jump` fusion
  would be a different family needing new semantics. Not attempted.
- `AugOp → StoreFast → Jump` (8.52M): augmented-assign loop shape;
  `AugOp` already carries the in-place protocol. Not attempted.
- `LoadConst → BinOp` pair (12.88M): covered indirectly by the 3-op
  fusions above where the leading load is const; the bare pair without a
  preceding load is not a lowerer-emitted sequence (stack provenance).

## 4. Primitive bench per-row changes

Ran both binaries over all 9 primitive probes under `APX_TIER=bytecode`,
64 rows total. The probe hot loops are MODULE-LEVEL, so the lowerer never
routes them through bytecode dispatch (`[BC-STATS] execs=0` on p06_str.py
etc. at baseline) — the fusions CANNOT affect these rows. Observed
row-by-row deltas are dominated by process noise (e.g. p06 `str.find`
49152B: baseline 8.29/8.87/8.71us vs fused 11.67/9.0/8.62us across 3
repeats — same distribution, not a regression). Full per-row table:
`/tmp/bp_prim_compare.txt` (captured this session). No primitive row is
claimed as a fusion win.

The measured wins are the isolated function-shaped microbenchmarks in
section 3, where `APX_BP_STATS` proves the fused opcodes executed.

## 5. Gate results (all env-sanitized)

Command prefix: `env -u APX_TIER_STATS -u APX_TIER
CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=/Users/caimlas/.git/worktrees/leaf-fp-superinst/target-fp-superinst`

| gate | result |
|---|---|
| `cargo test -p ap-bytecode --release` | 57 passed / 0 failed (was 52 before adding 5 new fused-opcode tests) |
| `cargo test -p ap-bytecode -p ap-run --release` | **21 suites / 279 passed / 0 failed / 1 ignored** (baseline 274; +5 new tests) |
| `cargo test -p ap-run --release --test ic_invalidation` | **9 passed / 0 failed** |
| `python3.14 tests/bench/primitive/fix_bytecode.py --other .../release/antipythony` | **13 cases / 0 mismatches** |
| `ap-conform values --tier 0` | **50/50 passed** (0 failed, 0 errors, 0 timeouts) |
| `ap-conform stdlib --tier 0` | **48/48 passed** |
| `ap-conform values --tier bytecode` | **50/50 passed** |
| `ap-conform stdlib --tier bytecode` | **48/48 passed** |

All gates were run with the FUSED binary. Debug helper bins
(`ap-run-probe`, `ap-run-eval`) were built into the same target dir before
the release test suite, because the fenced differential harness spawns
`debug/` helper binaries.

## 6. Files changed

- `crates/ap-bytecode/src/stats.rs` (NEW): opt-in histogram.
- `crates/ap-bytecode/src/lib.rs`: module export.
- `crates/ap-bytecode/src/{instr.rs,decode.rs,code.rs}`: 3 new opcodes,
  names, decode rendering, validate checks.
- `crates/ap-bytecode/src/{lower.rs,linetable.rs,exctable.rs,sites.rs}`:
  peephole + remap helpers.
- `crates/ap-bytecode/src/tests.rs`: 5 new tests + updated 6 existing
  expectations for the fused stream.
- `crates/ap-run/src/bytecode_interp.rs`: histogram wiring + 3 fused
  dispatch arms + guarded `fused_binop` helper.
- `crates/ap-run/src/jit_hook.rs`: histogram emission in `report_line()`.
- `plans/fast-path/02-perf/superinstructions-report.md` (this file) and
  `superinstructions-histogram-raw.txt` (raw profile output).

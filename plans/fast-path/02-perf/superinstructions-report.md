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

Command (env-sanitized, one canonical run; 65 probe files):

```
env -u APX_TIER_STATS APX_BP_STATS=1 APX_TIER=bytecode \
  target-fp-superinst/release/antipythony run <probe>.py
```

Raw, unedited histogram output for every `tests/bench/primitive/probes/*.py`
and every `tests/**/*ic*.py` probe: `superinstructions-histogram-raw.txt`
(644 lines, committed alongside this report).

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

## 2. Fusion candidates (data-driven shortlist)

The histogram says the dynamichot sequences are:

1. `LoadConst, LoadConst, BinOp` (9.24M) / `LoadFast, LoadConst, BinOp`
   (2.35M) / `LoadFast, LoadFast, BinOp` (0.42M) — constant/const
   arithmetic. Total `*,*,BinOp` ≈ 12M.
2. `LoadConst, BinOp` (12.88M) as a pair.
3. `LoadAttrCall, PopTop, Jump` (10.27M) — method-call statements in loops.
4. `StoreFast, LoadFast, LoadFast` (18.53M) — pure load/store shuffles.

Decisions and measured before/after numbers are filled in below after the
peephole pass and before/after measurement.

## 3. Fusions kept / dropped

_(to be filled)_

## 4. Gate results

_(to be filled)_

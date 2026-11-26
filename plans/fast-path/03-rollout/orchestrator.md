# FAST-PATH — orchestrator (03-rollout)

Adaptive default flip + the closing bench/report. The flip is gated on
02-perf proving no regression on any row.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-adaptive-toggle.md | bytecode-as-default + composition matrix + escape hatch | 02-perf merged |
| 02-bench-parity.md | closing bench + exit-gate check + report + README | 01 merged |

## Dispatch protocol

1. Sequential.
2. The composition matrix (bytecode + tier-1 + no-GIL + GC nursery)
   is the flip blocker — all four systems interacting must be green.

## Exit gate

master.md's exit gate: no walk-bound row below 1.0x; simple mean
>= 1.5x; full corpus + diff gates green in the default mode.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-adaptive-toggle.md | PENDING |
| 02-bench-parity.md | PENDING |

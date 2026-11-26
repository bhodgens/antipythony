# FAST-PATH — orchestrator (02-perf)

Performance passes over the working bytecode interpreter: inline
caches first (data-driven), then superinstructions driven by a real
opcode histogram.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-inline-caches.md | LoadGlobal/LoadAttr/Call site caches keyed on type_version | 01-core merged |
| 02-superinstructions.md | profile-driven opcode fusion + peephole | 01 merged (fusion needs histogram data) |

## Dispatch protocol

1. Sequential (both touch interp.rs dispatch).
2. The invalidation-correctness unit tests (01) are the plan's biggest
   risk mitigation — a stale cache is a silent miscompile. The 02.05b
   bump-site audit is the verification map.

## Test Gate

Mutation-after-warm tests; no-GIL contention probe; all gates in both
modes; histogram data committed before fusions land.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-inline-caches.md | PENDING |
| 02-superinstructions.md | PENDING |

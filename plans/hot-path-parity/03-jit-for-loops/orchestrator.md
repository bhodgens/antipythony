# HOT-PATH-PARITY — orchestrator (03-jit-for-loops)

For-loop lowering is THE gate: sort/listcomp/join bench shapes are all
for-loops and none reach the JIT until this lands. Bundles the cheap
OSR liveness refinement.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-for-loop-lowering.md | Stmt::For desugar + iter/next seams + Break/Continue + backward-dataflow liveness | none (parallel with 01-nursery) |

## Dispatch protocol

1. Parallel with 01-nursery (file-disjoint: ap-codegen-native vs
   ap-gc/ap-obj).
2. Merge before 02-dispatch/02-typed-dict-regions is dispatched.

## Test Gate

For-loop bodies compile for 5 iterator kinds; StopIteration/break/
continue/else byte-identical; adaptive conform green; measured
for-loop deltas recorded (int-accum >= 1.8x target).

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-for-loop-lowering.md | PENDING |

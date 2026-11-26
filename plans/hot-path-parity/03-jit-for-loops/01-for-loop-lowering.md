# 01-for-loop-lowering — Stmt::For in the JIT function lowering

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-hp-forloop (branch
tier1-hp-forloop, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-hp-forloop everywhere.

Parent: plans/hot-path-parity/master.md. PARALLEL-OK with 01-nursery
(file-disjoint: ap-codegen-native vs ap-gc/ap-obj). ~90K context.
THE GATE LEAF: sort/listcomp/join bench shapes are all for-loops;
none of them can reach the JIT until this lands (tier1 verdict doc
§5.1).

## Problem

cfg::lower_fn treats Stmt::For as Unsupported — the whole function
body lower-rejects to pure tree-walk whenever a for-loop appears.
The while lowering is complete and proven (OSR eligibility-first fix
merged); for-loops need the iterator protocol expressed as boxed
insts.

## Design

1. **Desugar for -> while shape in lowering**: `for x in it:` becomes
   iter = __iter__(it) once; loop { x = __next__(iter) or exit }.
   Lower __iter__/__next__ through ONE boxed helper each (host seam
   like the existing SUBSCRIPT_SET_FN / COMPARE_FULL_FN pattern —
   the tier1 leaf established the pattern; read its commits).
   StopIteration on next -> exit edge (loop break).
2. **Break/Continue**: structured with the existing while machinery
   (block targets); verify with nested loops + else clause.
3. **OsrPoll eligibility**: apply the eligibility-FIRST ordering (the
   merged fix). Expect live-set growth from the iterator binding —
   if OSR_MAX_LIVE trips, either raise the cap or implement the
   liveness refinement below.
4. **Liveness refinement (bundle here)**: replace the conservative
   forward live_in scan with a true backward dataflow pass (locals
   only, cell vars excluded) — cheap, unblocks wider OSR eligibility.

## Files you own

crates/ap-codegen-native/src/{cfg.rs, clif.rs, osr.rs, runtime.rs,
infer.rs}, crates/ap-run/src/jit_hook.rs (seams if needed), new tests
under crates/ap-codegen-native/tests/.

## Tasks

T1 — iter/next host seams + for-desugar lowering for list/tuple/str/
range/dict iterators (boxed protocol; the runtime's iterator table
already unifies these — route through it). T2 — Break/Continue/else
+ nested loops; regression tests. T3 — OSR eligibility-first
integration + liveness refinement (backward dataflow). T4 — measure
with pp2_tier1 harness pattern extended to for-loops: sort probes,
listcomp shape, join/split harness — record tier-0 vs tier-1 per
shape. Targets: for-loop int-accumulate >= 1.8x (parity with the
while number); any shape that was lower-rejecting now compiles.
T5 — full gate battery (7 diff gates, conform tier-0 AND adaptive,
cargo ap-run + ap-codegen-native, GC differentials) + regression
tests (StopIteration mid-loop, generator iterators, break/continue/
else, nested for + OSR). T6 — report
plans/hot-path-parity/03-jit-for-loops/for-loop-report.md.

## Self-verification checklist
- [ ] for-loop bodies COMPILE (no lower-reject) for the 5 iterator
      kinds; regression test per kind
- [ ] StopIteration/break/continue/else semantics byte-identical
- [ ] backward-dataflow liveness landed; OSR eligibility widened
- [ ] measured per-shape deltas recorded; int-accum for >= 1.8x
- [ ] all gates green incl. adaptive conform

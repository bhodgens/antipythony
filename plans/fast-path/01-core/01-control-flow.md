# 01-control-flow — Jump/ForNext/BoolOp/Break/Continue lowering + execution

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-controlflow (branch
tier1-fp-controlflow, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-fp-controlflow everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 00-infra both leaves
MERGED. ~90K context.

## Deliverable

1. **Lowerer**: remove the For/While/Try rejection for loops WITHOUT
   try-blocks (Try stays rejected until 01-core/03). Lower While and
   For to Jump/JumpIfFalse/GetIter/ForNext opcodes. BoolOp to
   short-circuit Jump chains (and/or value rule preserved). Break/
   Continue/else via block-target tracking. Pass/Elif chains.
2. **Execution**: ForNext op (iter.next() via the runtime's iterator
   table, StopIteration -> jump to exit), BoolOp short-circuit eval,
   Jump* ops in the dispatch loop (direct pc assignment).
3. **Coverage gate additions**: fix_bytecode.py gains loops —
   for/while over list/tuple/str/range/dict, nested loops,
   break/continue/else, while-else, boolop value semantics
   (a or b returns a's VALUE), short-circuit side-effect ordering,
   deep nesting, loop over empty containers, while True/break.

## Why this leaf exists

The merged hp-forloop work proved the JIT route gives 2.16x on
for-loops but typed regions don't form (block-0 IterNew). This leaf
is the cheaper sibling win: the bytecode loop runs the same iterator
helpers with a ~10ns dispatch step instead of ~190ns, no JIT compile
cost, no region constraints — and it composes with calls/exceptions
later. Expected effect: the P08 list.sort rows' build-floor (3.9s
listcomp at 1M) and the join/split harness shapes drop dramatically.

## Targets

- 1M-element listcomp build (currently 3.9s under tree-walk): >= 5x
  improvement in bytecode mode (measured via the fixsort2_isolation
  probe pattern).
- While-loop counter stepping: within 3x of CPython (from ~15x).
- All byte-exactness gates green in both modes.

## Files you own

crates/ap-bytecode/src/{lower.rs, interp.rs}, ap-bytecode Instr enum
extensions, fix_bytecode.py probe extensions.

## Self-verification checklist
- [ ] While/For lowered (no FallbackToTreeWalk for loop-only bodies)
- [ ] Try-blocks still fall back (documented, 01-core/03's job)
- [ ] BoolOp value semantics + short-circuit ordering three-way
      identical
- [ ] fix_bytecode.py loop coverage added and three-way identical
- [ ] 1M listcomp >= 5x in bytecode mode
- [ ] standard battery green (7 diff gates + conform + cargo)

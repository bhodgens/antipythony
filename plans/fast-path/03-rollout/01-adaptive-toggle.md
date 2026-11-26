# 01-adaptive-toggle — flip the default to bytecode, adaptive selection

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-adaptive (branch
tier1-fp-adaptive, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-fp-adaptive everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 02-perf both leaves
MERGED. ~60K context.

## Deliverable

1. **Adaptive selection**: functions with bytecode available run
   bytecode mode by default; FallbackToTreeWalk shapes run the
   tree-walker (unchanged). The existing adaptive tier logic
   (tier1 TierUpDriver) composes: bytecode is the new tier-0.5;
   tier-1 JIT still tiers up over it for eligible shapes.
2. **Default flip**: APX_TIER=tree0 available as an escape hatch;
   bytecode becomes the default tier for lowering-covered shapes.
   The flip happens ONLY after 02-perf gates prove no regression on
   any row.
3. **Interaction audit**: bytecode + tier-1 JIT + no-GIL threads +
   GC nursery — run the full stack together (the composition matrix:
   bytecode fn calling tier-1 fn calling bytecode fn, under threads,
   under GC pressure). Any interaction bug is a blocker.
4. **Escape hatch docs**: README + docs update describing the tier
   model (tree0 | bytecode | adaptive | jit).

## Gates

- Full corpus (values + stdlib) green in adaptive mode (bytecode
  default).
- Composition matrix green (bytecode/tier-1/threading/GC combined).
- Primitive bench: no row regresses vs the tree-walk numbers.
- tier_mode_differential extended for the new mode count.

## Files you own

crates/ap-run/src/{jit_hook.rs, interp.rs tier plumbing}, CLI,
docs/tier-model.md (new), README section.

## Self-verification checklist
- [ ] composition matrix green (the four systems together)
- [ ] default flip: no row regresses
- [ ] escape hatch (--tier tree0) works and is documented
- [ ] corpus + diff gates green in the new default mode

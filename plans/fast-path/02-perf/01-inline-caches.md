# 01-inline-caches — name/attr/call site caches keyed on type_version

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-inlinecache (branch
tier1-fp-inlinecache, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-fp-inlinecache everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 01-core all three MERGED.
~80K context.

## Deliverable

The site table from 00-infra (empty slots) becomes real inline
caches. CPython 3.11+ and PyPy both prove this is where the remaining
dispatch cost lives after bytecode.

1. **LoadGlobal cache**: per site, cache (globals_dict_identity,
   builtins_dict_identity, resolved Value). Hit = two pointer compares
   + direct load (kills the two-dict probe per global read).
2. **LoadAttr cache**: per site, cache (receiver_type_id,
   type_version, resolved attr). Invalidated by the 02.05b
   type_version/surgery_version counters (already bumped at all
   mutation sites — audited).
3. **Call site cache**: per site, cache (callee identity, argc) for
   the (native-receiver, known-method) fast path.
4. **Invalidation correctness unit tests**: mutate a class after warm
   (add/remove/set attr, change bases), assert cache misses correctly.
   This is THE risk: a stale cache = wrong attribute = silent
   miscompile. The 02.05b bump-site audit is the map — every mutation
   path must be verified to bump.
5. **No-GIL safety**: the runtime is no-GIL — caches need either
   per-thread tables or sharded locks; measure both, pick the faster,
   document the contention profile.

## Targets

- Warm global read <= 30ns (from ~190ns walk + dict probe).
- Warm attribute read <= 60ns.
- dict.get row in bytecode mode: measurable improvement (the
  bound-method mint + resolution were the walk-bound term).
- Mutation-after-warm unit tests green.

## Files you own

crates/ap-bytecode/src/{caches.rs (new), interp.rs}, site wiring in
lower.rs, mutation-bump verification in classes.rs (read-only audit +
any missing bump fixes), tests.

## Self-verification checklist
- [ ] mutation-after-warm tests (add/remove/set attr, bases change)
- [ ] no-GIL contention probe (4 threads x one site) green
- [ ] warm global/attr targets met
- [ ] all gates green in both modes

# 02-superinstructions — fused opcodes + peephole on the lowerer

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-superinst (branch
tier1-fp-superinst, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-fp-superinst everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 02-perf/01-inline-caches
MERGED (fusion decisions are driven by real opcode-frequency data from
a profiled run). ~60K context.

## Deliverable

1. **Profile first**: run the primitive bench + a representative app
   workload in bytecode mode with an opcode-frequency histogram (a
   APX_BP_STATS=1 counter in the dispatch loop). Report the top-20
   pairs/triples. FUSE WHAT THE DATA SAYS — not a guessed list.
2. **Peephole on the lowerer**: after lowering, a peephole pass rewrites
   common sequences: Load x; Load y; BinOp -> Load2BinOp;
   LoadConst; Call1 -> LoadConstCall1; JumpIfFalse over PopTop ->
   fused branch; StoreFast; LoadFast same-slot -> DupStoreLoad.
3. **Fused opcodes**: implement the data-driven top set. Each fused
   opcode skips the dispatch round-trip for its components (one
   boundary instead of N).
4. **Specialization**: type-specialized BinOpInt_Add etc. guarded by a
   cheap smi/type check with a generic fallback (deopt = re-execute
   generically; NO miscompile risk — the guard is the semantics).
5. Measure before/after per fused opcode; drop fusions that don't pay.

## Targets

- Dispatch overhead reduction proportional to the histogram (top-10
  pairs typically 30-50% of dispatches in dynamic languages).
- Primitive bench bytecode-mode rows: across-the-board improvement;
  report per-row.

## Files you own

crates/ap-bytecode/{lower.rs (peephole), interp.rs (fused arms),
stats.rs}, Instr extensions.

## Self-verification checklist
- [ ] opcode histogram data committed (the profile run output)
- [ ] only data-driven fusions; each with before/after measurement
- [ ] type-specialized ops guarded (deopt = generic re-execution)
- [ ] all gates green in both modes

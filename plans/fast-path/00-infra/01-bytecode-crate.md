# 01-bytecode-crate — Instr enum, lowerer, const pool, line table, site table

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-bytecode (branch
tier1-fp-bytecode, from current main). Own commits, explicit pathspecs.
ONE heavy cargo lane; CARGO_BUILD_JOBS=4; CARGO_TARGET_DIR=$PWD/
target-fp-bytecode everywhere.

Parent: plans/fast-path/master.md. ~120K context (the largest leaf in
the plan — the foundation everything else builds on).

## Deliverable

A NEW crate `crates/ap-bytecode/` containing:

1. **`Instr` enum** — flat, Copy-friendly where possible. Start set
   (from Expr coverage): LoadConst(usize const-pool idx), LoadFast(slot),
   LoadGlobal(name-idx), StoreFast(slot), PopTop, BinOpInt(BinOp),
   BinOpFloat(BinOp), BinOpGeneric(BinOp), UnaryOp(UnaryOp),
   Compare(CompareOp), JumpIfFalse(target), JumpIfTrue(target),
   Jump(target), GetIter, CallFunction(argc), LoadAttr(name-idx),
   StoreSubscript, Subscript, LoadConstNone/True/False/EmptyTuple,
   BuildList(count), BuildTuple(count), BuildDict(count),
   FallbackToTreeWalk. Add ForNext + exceptions in leaf 01-core
   (do NOT lower them yet — lower_fn rejects functions containing
   For/Try with a marker; the function simply falls back).
2. **Lowerer** (`lower.rs`): ap-ir Expr/Stmt tree -> Vec<Instr> +
   const pool + line table + slot assignment (locals via a scan of
   StoreFast names; everything else global/attr). Every node gets a
   line-table entry (sparse: only where line CHANGES). Unsupported
   shapes -> FallbackToTreeWalk at FUNCTION granularity (never
   partially-lowered bodies).
3. **Const pool**: deduplicated (strings, big ints, floats). Big ints
   via ap-obj::Int::parse at decode time, interned.
4. **Line table**: (instr_idx -> line) as a sparse sorted vec; decode
   API `line_for(instr_idx)` used ONLY by exception/warning paths.
5. **Site table**: per Load/LoadGlobal/LoadAttr/Call site, a stable
   site id (u32) assigned at lowering; cache storage lives in
   02-perf/01-inline-caches — here only the id assignment + empty
   per-site slot array.
6. **Tests**: lowerer round-trip tests for every Expr variant
   (accept: some variants lower to FallbackToTreeWalk — assert THAT);
   decode tests; line-table tests.

## Out of scope (later leaves)

The BytecodeInterp dispatch loop (leaf 02-frame-and-entry), control
flow execution, exceptions, caches. This leaf is pure IR->bytecode
translation + data structures. The build must show the new crate
compiling with unit tests green; nothing wires it into ap-run yet.

## Files you own

crates/ap-bytecode/** (new crate), workspace Cargo.toml members entry.
Do not touch ap-run/interp.

## Gates

- `cargo test -p ap-bytecode` green (new unit tests).
- `cargo build --workspace` green (crate integrated as member, unused).
- clippy clean on the new crate.
- Existing test battery untouched (cargo test -p ap-run must be
  unaffected — pure addition).

## Self-verification checklist
- [ ] Instr enum covers the start set; every variant constructed by
      the lowerer is exercised in a test
- [ ] Unsupported shapes lower to FallbackToTreeWalk (function-level)
- [ ] Const pool dedupes; line table sparse and monotonic
- [ ] clippy clean; workspace builds

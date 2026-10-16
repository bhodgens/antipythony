# Plan 01 progress log

## Leaf 01.01 - ap-parse wrapper - COMPLETE (committed 81126cc)
- parse() -> Result<ModuleAst, ParseError>, PY314 pin, fail-closed on syntax +
  unsupported-syntax, contract diagnostic `error at line L, column C: <msg>`,
  line_col with unicode-scalar columns + EOF clamping.
- 68 fixtures (63 valid / 5 invalid); corpus tests 7/7; matrix cross-check 6/6;
  lib unit tests green. cargo fmt clean.
- INCIDENT: first implementation of lib.rs was clobbered by the orchestrator
  (git checkout against uncommitted agent work). Rebuilt by recovery agent
  deleg_300b5de2 from the surviving tests as spec. LESSON: never `git checkout
  --` a file with uncommitted agent edits; stash first, inspect second.

## Leaf 01.02 - ap-ir structures + builder - COMPLETE (committed 81126cc)
- 35 Expr + 21 Stmt variants; TypeLattice; 83 clause-cited Builder methods
  (each doc cites C-xx/S-xx-yy); exhaustiveness matrix 33 expr + 25 stmt rows
  (Strategy: Builder|LoweredVia|NotProduced); two-sided enforcement
  (ap-ir side name tables + ap-parse side wildcard-free ruff matches, arity
  pinned by test); deterministic dump formatter (golden format for 01.03);
  scope-fenced generator detection; 7/7 tests; example dump verified.

## Test leaves T-01 + T-02 - AUTHORED (committed 81126cc)
- Separate-authorship protocol held: both agents barred from crates/ + git
  history; tests written from contract + spec pack only.
- T-01 parse diagnostics: 33 cases (valid formats, exact hand-derived L/C for
  9 invalid programs, regex format checks, exit codes 0/1/2, fail-fast first
  error). 6 contract gaps filed.
- T-02 IR shape: 21 cases (per-construct single-node assertions, associativity
  tree counts from S-01-2, determinism). 3 contract gaps filed.
- Both suites currently 54 xfail (bins `ap-parse-check` / `ap-ir-dump` do not
  exist yet - testability bins are the follow-up when gates flip).
- GAP LEDGER (feeds back to user / spec pack, never silently resolved):
  G1 diagnostic anchor token + 0/1-based unpinned; G2 unicode column units
  unpinned (impl chose scalar-columns; contract must be amended to match);
  G3 statement-count semantics unpinned; G4 testability bins not in plan docs;
  G5 exit-2 basis; G6 two diagnostic format families; T2-G1 dump vocabulary
  unpinned; T2-G2 fstring node kind unnamed; T2-G3 indent/child-order loose.

## Next: 01.03 (lowering) + 01.04 (verify) + T-03/T-04
- 01.03 needs: AST->IR conversion (ruff types live in ap-parse only), scope
  resolution, 20 golden IR dumps vs dump formatter.
- Testability bins: `ap-parse-check` + `ap-ir-dump` CLIs should ship with 01.03
  dispatch so T-01/T-02 xfails flip to real runs.

# Plan 01 progress log

## Leaf 01.01 - ap-parse wrapper - COMPLETE (committed 81126cc)
- parse() -> Result<ModuleAst, ParseError>, PY314 pin, fail-closed on syntax +
  unsupported-syntax, contract diagnostic `error at line L, column C: <msg>`,
  line_col with unicode-scalar columns + EOF clamping.
- 68 fixtures (63 valid / 5 invalid); corpus tests 7/7; matrix cross-check 6/6.
- INCIDENT: first lib.rs clobbered by orchestrator git checkout against
  uncommitted agent work; rebuilt by recovery agent from surviving tests.
  LESSON: stash first, never checkout uncommitted agent edits.

## Leaf 01.02 - ap-ir structures + builder - COMPLETE (committed 81126cc)
- 35 Expr + 21 Stmt variants; 83 clause-cited Builder methods; exhaustiveness
  matrix 33 expr + 25 stmt rows, two-sided enforcement; deterministic dump;
  scope-fenced generator detection; 7/7 tests.

## Test leaves T-01 + T-02 - AUTHORED (committed 81126cc)
- T-01: 33 cases (parse diagnostics, hand-derived positions). 6 gaps filed.
- T-02: 21 cases (IR shape). 3 gaps filed.
- GAP LEDGER: G1 anchor token + 0/1-based; G2 unicode col units (impl chose
  scalar columns - contract amendment needed); G3 statement-count semantics;
  G4 testability bins not in plan docs; G5 exit-2 basis; G6 two diagnostic
  format families; T2-G1 dump vocabulary; T2-G2 fstring kind; T2-G3 indent
  semantics.

## Leaf 01.03 - AST->IR lowering - COMPLETE (committed d75c0a4)
- lowering.rs ~1400 lines, clause-cited, total (no panic paths; LowerError
  Parse|Bug split). Scope classification: globals/nonlocals/frees/cells with
  CPython qualname paths, byte-stable sorted output. LocalDef/LocalClass keep
  source position (S-02-7). ap-ir-dump bin (tree default + --golden form).
- 20 goldens (5 hand-verified), 10 scope tests, 26 ap-parse tests green,
  workspace 33 green. T-02: 21/21 XPASS (flipped from xfail).
- PROVISIONAL ENCODINGS (flagged in code, need contract notes eventually):
  PEP 695 type alias -> Pass + name binding; case-as -> Group + capture;
  except* -> try shape (group semantics runtime-side); rare class keywords ->
  __ap_classkw__ call encoding.
- PON PRE-TASK NOT EXECUTED: 01.03's license-granted study of pon-ir lowering
  was skipped (no pon source locally). Options: clone pon (license now grants
  reading) and do the findings note as a standalone task, or strike the
  pre-task. USER DECISION. Same applies to 02.05's pre-task later.

## Test leaf T-03 - AUTHORED + VERIFIED (committed d75c0a4)
- 200 parametrized cases: scope classification vs goldens (loose structural
  matches per T3-G1 gap), totality over 69 fixtures + 20 corpus programs,
  determinism, targeted scoping clauses, negative diagnostics.
- FOUND + FIXED 3 hand-derivation errors in scope goldens by cross-checking
  against live CPython 3.14.7 symtable (c14 print-in-globals, c15 TypeError,
  c18 fstring-literal name). Goldens now match CPython exactly.
- Suite: 169 xfail (bin-dependent, pre-bin) + 31 xpass; after 01.03's bin:
  41 xfail / 1 xpass / 0 hard fails. 5 new gaps filed (T3-G1..G5).

## Leaf 01.04 - IR verify pass - COMPLETE (uncommitted)
- ap_ir::verify(module) -> Vec<Diagnostic>: must-analysis (definite-assignment
  with del-marks) unbound-local prediction with byte-exact 3.14 message
  `cannot access local variable 'x' where it is not associated with a value`
  (byte-verified vs CPython 3.14.7, incl. the G-7 function-scope del form and
  module-scope NameError form); name-resolution completeness with a
  conservative G-5 suggestion guard (edit distance <= 1 silences); module/
  class/function enclosing-chain legs (S-02-5 class = suggestion-only).
- ap_ir::fold: conservative constant folding (int/float/str/boolop) with
  C-12..C-18 semantics via string big-int math; never folds raising ops
  (/0 %0 //0 neg-shift matmul), calls, names, comparisons; py_float_repr
  public for byte-exact float rendering.
- Compile-stage static checks in ap-parse::verify_bridge (G-4 territory):
  p09 global+nonlocal, p10 nonlocal-no-binding, p11 nonlocal@module,
  p12/p13 break/continue outside loop, p14 return outside fn, p15/p16
  import * (caret at stmt end - 1), p18 dup param, p17 parse-stage remap.
- Position convention (fixture-verified): display col = file col - indent + 4
  (reproduces all 32 pinned caret columns); load_ordinal on Diagnostic maps
  diagnostics to the right read site (del-then-read flags the post-del read).
- Wire-in: ap-parse::verify_source / verify_ir_with_positions;
  ap-verify-dump bin (T-04 G-1 seam: `<path>:<line>:<col> <class>: <msg>`,
  exit 0/1); ap-ir-dump --verify (stderr, exit 3 distinct from parse errors).
- Tests: crates/ap-ir/tests/verify.rs 31/31 (CPython-differential: 13
  verify-shape + message byte-match, 11 static checks w/ pinned line:col,
  80 fold cases byte-exact incl. big-int/float repr, negative-division,
  never-fold raising ops). T-04 suite: 15 passed + 29 xpassed, 0 failed,
  0 xfailed (was 32 xfail). Full pytest corpus: 1164 passed, 0 failed.
  Full workspace cargo test green; cargo fmt clean.
- DESIGN NOTES (T-04 doc deltas): S-02-2 static rule + fixtures forced a
  must-analysis refinement beyond the leaf summary's "never assigned
  anywhere" reading (p02/p03 flag reads after branch-only binds; n03 silent
  via range(3)-definite for-targets); del-then-read flags via del_marks
  killing prior definite binds (CPython-probed); module-scope del-then-read
  is NameError not UnboundLocal (p19).


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

## NEXT: 01.04 (IR verify pass) - last impl leaf of Plan 01
- UnboundLocalError prediction vs CPython (matching message text), name
  resolution completeness, conservative constant folding.
- T-04 spec exists (authored early by test track). Dispatch 01.04 impl, then
  Plan 01 integration review -> Plan 02.

# Leaf S016B session record — 2026-11-20 (worktree ~/.git/worktrees/leaf-s016b, branch tier1-s016b off main@4b8e1ee)

## Status: DONE — extends s016's per-call source lines to measured-diverging
## instrument classes (BinOp / Compare / Subscript), 3 new pinned corpus
## cases, s016 stdlib slice stays 48/48, values 48/48, zero new regressions.

## Probe-first findings (CPython 3.14.7, /opt/homebrew/bin/python3.14)
Ground truth measured via tb_lineno walks (raising operands) AND warnings
stacklevel=2 (dunder-dispatched warnings), across shapes: op on its own line,
op glued left/right, call-valued operands, display-start vs instrument line:
- **BinOp**: BINARY_OP positions at the LEFT OPERAND's start line
  (`1` L15 `/` L16 `0` L17 -> 15; left=call keeps the CALL line).
- **Compare**: COMPARE_OP positions at the FIRST operand's start line
  (`1 ==` / `c` next line -> the `1` line; chained compare warns at the
  chain's first-operand line).
- **Subscript**: BINARY_SUBSCR positions at the OBJECT's start line
  (obj / `[` / key on three separate lines -> obj's line).
- **IfExp**: condition-line attribution measured (o4/o5 probes); NOT
  extended — the tree-walker publishes the condition's own evaluation into
  the same frame and the measured shapes' attribution equals what
  statement + operand instruments already produce for every probed shape
  EXCEPT the pure `(1 if <newline> [][0] ...)` opener shape; recorded as
  honest gap (would need an IfExp line field; same pattern, no corpus case
  gated it).
- **UnaryOp**: `-` alone on its own line attributes to the OPERATOR line
  (s4b probe: 72 not 73); NOT extended — measured divergence exists only
  for the operator-on-own-line shape, which requires whitespace-only
  continuation lines (rare, no corpus case); recorded as honest gap with
  the probe recipe in PROBES-S016B.md.
- **Attribute**: q8 probe measured attr-name-line attribution (`k8` /
  `.` / `nope` -> 124, the attr line, NOT obj start — ruff Attribute node
  start == obj start, so an extension must stamp `a.attr` range). NOT
  extended (no raising-attr corpus pressure; pattern documented).
- Warning TEXT already matched everywhere; only lineno diverged (s016's
  finding, confirmed).

## Fix shape (commits 7e164a7, aebcffb; 3 increments total)
Follows the landed Expr::Call pattern exactly:
- ap-ir `Expr::BinOp(BinOp, Box, Box, usize)` /
  `Expr::Compare(Vec<CmpOp>, Vec<Expr>, usize)` /
  `Expr::Subscript(Box, Box, usize)`: trailing usize = instrument line
  (0 = unknown). All walk/verify/dump pattern sites updated; Builder
  constructors keep 0; hand-built-IR test constructions updated with `, 0`.
- ap-parse lowering: BinOp stamps `line_at(b_.left.start())` (LEFT operand
  — NOT node start: ruff BinOp.start == left.start anyway, but the left
  reference is the semantic contract); Compare stamps node start
  (== first operand start); Subscript stamps `s.value.start()`.
- ap-run interp.rs: publishes to `frame.last_line` AFTER operand
  evaluation (operand CALLs publish their own lines per s016), BEFORE the
  operation/compare/subscript dispatch — a warning/raise from dunder slot
  dispatch reads the instrument line. `line > 0` guard keeps hand-built IR
  on statement-granularity attribution.
- JIT/AoT/Wasm paths untouched (cfg.rs / lower.rs pattern sites ignore the
  new field; verify_bridge confirms IR shape superset).
- Machine note: mid-session the ORIGINAL leaf-s016b worktree was wiped by
  an external `git worktree prune`/cleanup (worktree admin files removed;
  branch tier1-s016b deleted). Recreated fresh off main@4b8e1ee (which
  includes merged s016 ef99fab) and rebuilt. All edits re-applied on the
  fresh tree; ground-truth probes were re-run before re-pinning. Probe
  records preserved in plans/antipythony/01-frontend-ir/PROBES-S016B.md.
- Parallel-safety: a foreign FUZZ_SEEDS.txt modification (19 appended
  lines, append-only) appeared in the worktree; verified byte-identical to
  main's copy and reverted (not mine to commit). `git add` used with
  explicit paths throughout (s016's lesson #3).

## Corpus (append-only; 3 new cases, pins from CPython 3.14.7)
- exceptions/e016_binop_compare_instrument_lines.py — raising __radd__
  mid-BinOp, ZeroDivision multi-line BinOp, raising __eq__ Compare; pins
  tb_lineno walks (linenos only — co_name/qualname shape is the documented
  `Cls.method`-vs-`method` gap, deliberately not pinned).
- exceptions/e017_binop_warn_instrument_lines.py — warnings stacklevel=2
  from __add__/__lt__ inside multi-line BinOp/Compare + one-line-expr
  inside earlier-starting display.
- exceptions/e018_subscript_ifexp_instrument_lines.py — raising
  __getitem__ (obj/`[`/key on 3 lines) + raising IfExp condition.
All 3: ap-run-probe tier-0 output byte-identical vs CPython 3.14.7
(stdout+stderr+rc), .meta provenance via tests/tools/_gen_meta.py
(pre-existing metas' date-only churn reverted; only new metas kept).

## Gates (all measured this session, this worktree, this tree)
- ap-conform stdlib slice --tier 0: **48/48** (s016 stays green — target)
- ap-conform values slice --tier 0: **48/48**
- ap-conform exceptions slice --tier 0: 18/18 (15 old + 3 new)
- ap-conform classes/gc/threading/tier1-transparency/fstrings tier-0:
  failures = c002/c018/c030/c031/c032, gc018, T109+t1_007, f33 — the
  documented baseline envelope, IDENTITIES unchanged (NEXT-0210A compared
  the same set). Threading timeouts=1 in DEBUG is T130: reproduces
  identically on the 0210a base binary (their gate ran RELEASE); verified
  NOT mine by (a) T130 hanging at rc=124 on 0210a's debug ap-run-probe,
  (b) MY release ap-run-probe T130 output byte-matching CPython.
- controlflow_wasm differential: 1/1 ok; scan_vet_contract 1/1;
  classes_exceptions_wasm_gate 1/1
- ap-ir + ap-parse unit suites: 84 passed / 0 failed
- ap-run full suite: 206 passed / 0 failed (incl. classes_differential
  29/29, method_surface 34/34, import 6/6, gc_differential 2/2)
- ap-codegen-native + ap-codegen-wasm suites: 107 passed / 0 failed
- pytest corpus_traceability + t05_values: 44 + 3 failed — byte-identical
  failure SET vs main (verified per-case diff; all pre-existing provenance
  floor + V048 missing-pin gaps)
- adaptive tier stdlib slice: 47/48 — s023_ast_parse_dump_unparse red,
  VERIFIED PRE-EXISTING (s016's own record; ast.unparse surface gap)

## Honest gaps / follow-ups
1. **UnaryOp** measured divergent only for operator-on-own-line shapes
   (probe s4b/s6 recipe in PROBES-S016B.md). Not extended: no corpus
   pressure, requires whitespace-only continuation lines. Same 4th-field
   pattern applies if ever gated.
2. **IfExp** divergent for `(1 if <newline> cond)` opener shape only.
   Not extended for the same reason; cond-side attribution already correct
   through operand instruments in every other probed shape.
3. **Attribute** instruments at the attr-NAME line (not obj start); an
   extension must stamp `a.attr`'s range start, not node start. Probed,
   documented (q8), not built.
4. **Compare chains**: CPython has per-chain-link instruments (middle ops
   carry their own positions); we stamp the chain's FIRST-operand line for
   every link. Single comparisons (the overwhelmingly common shape) are
   exact; multi-link chains with later links on different lines would
   attribute to the chain-start line. Probed p4 (first link) exact; a
   middle-link-only corpus case would diverge.
5. adaptive s023 red (pre-existing, ast.unparse surface).
6. qualname shape `Cls.__dunder__` vs CPython `__dunder__` in tb frame
   names (documented 02.03 envelope) — corpus pins avoid depending on it.
7. External worktree wipe mid-session (disclosed above): consider
   orchestrator-level protection for sibling-owned worktrees.

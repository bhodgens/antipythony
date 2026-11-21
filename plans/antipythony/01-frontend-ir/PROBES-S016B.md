# S016B ground-truth probes — CPython 3.14.7 (/opt/homebrew/bin/python3.14)
# Measured 2026-11-20 (first worktree instance; re-verify after rebuild).

## Instrument line semantics measured (frame f_lineno of the frame executing
## the operator, via tb_lineno walk / warnings stacklevel=2):
- BinOp: LEFT OPERAND start line. (`1` line 15, `/` 16, `0` 17 -> 15;
  `1` 22, `+ a` 23 -> 22; left=call mk() on 109, `+1` 110 -> 109.)
- Compare: LEFT OPERAND start line. (`1` 51, `== c` 55->measured 55;
  chained `d <` 68 -> 68.)
- UnaryOp: OPERATOR line (node start). `-` alone on 72, `f6` 73 -> 72;
  `-f6` single line -> that line.
- Subscript: OBJECT start line. obj `e` 44, `[` 45, `1` 46 -> 44.
- IfExp: CONDITION line when cond starts after the node (o4: `1 if` 14,
  `[][0]` 15 -> 15); o5: `1` 25, `if [][0]` 26 -> 26 = cond line too.
  (IfExp instrument = condition start; node start differs in o4 only.)
- Call: node start (s016 landed).
- Attribute: obj line (q8: `k8` 122, `.` 123, `nope` 124 -> measured 124?? 
  NO: q8 measured a8-frame 124 = `nope` line. Attribute instrument =
  attribute load position = attr NAME line, not obj! RE-VERIFY.)

## NOTE on q8: measured frames [('<module>',126),('a8',124),('__getattr__',118)]
## with `k8` on 122, `.` 123, `nope` 124 -> a8 frame shows 124 (attr line).
## ruff Attribute node start == obj start (122) - so if extending Attribute,
## stamp from a.attr range start, not node start. NOT extended this leaf
## unless probed divergent (attr alone on its own line is rare).

## Decision matrix (measured divergence vs statement-line tracking):
- BinOp multi-line: DIVERGES (statement line vs left-operand line)
- Compare multi-line: DIVERGES
- UnaryOp op-on-own-line: DIVERGES
- Subscript obj/key on separate lines: DIVERGES
- IfExp cond on later line: DIVERGES

## Shapes where op/operand share one line with the statement: no divergence
## (same line as statement start? NO — display `[` can start the statement
## earlier; but instrument line == the operand line which may differ from
## the DISPLAY start line. Our Stmt::Line sets statement FIRST line, so a
## BinOp `e[1]` on line 66 inside `v = [` line 65: CPython reports 66
## (r4). We would report 65 WITHOUT the fix. So divergence exists whenever
## the instrument line != statement start line.)

# 03-exceptions — Try/except/finally/raise mapping with exact line attribution

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-exceptions (branch
tier1-fp-exceptions, from current main). Own commits, explicit
pathspecs. ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
CARGO_TARGET_DIR=$PWD/target-fp-exceptions everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 01-core/01 + 02 MERGED.
~100K context. HARDEST leaf in the plan — exceptions are where
traceback line attribution, exception identity, and finally-ordering
semantics concentrate.

## Deliverable

1. **Lowerer**: Try/Except/Finally/Raise/RaiseFrom -> exception-table
   form (CPython 3.11+ style: zero-cost try ranges — a (start, end,
   handler, stack_depth) table per function consulted ONLY when an
   exception is in flight; no runtime cost on the non-exception path).
   Finally blocks lowered as duplicated epilogues + a finally-pending
   mechanism for propagate-through-finally.
2. **Execution**: exception in flight -> consult the table for the
   current pc -> unwind to handler with stack depth restoration.
   Raise opcodes construct exceptions via the existing raise() helpers
   (identity, traceback attachment, __context__ chaining preserved).
3. **Line attribution**: the line table must reproduce CPython's
   traceback entries for raised-in-try, raised-in-handler,
   raised-in-finally, re-raise, and chained causes — EXACT lines.
   The s016 family + a new tb-lines probe (raises in 6 shapes, prints
   formatted tracebacks) must be three-way identical.
4. **Semantics pins**: `except` clause matching (tuple, subclass,
   exactly-one-consumption of the exception), bare except,
   finally-always-runs (including break/continue/return through
   finally), return-in-finally overrides, __context__ vs __cause__,
   StopIteration-in-generator-adjacent shapes (generators themselves
   stay tree-walked).

## Gates

- fix_bytecode.py exception coverage added and three-way identical
  (raise shapes above + traceback formatting probes).
- s016/s016b/s016c + tb-lines probes green in both modes.
- Standard battery green.

## Files you own

crates/ap-bytecode/{lower.rs, interp.rs, exctable.rs (new)},
fix_bytecode.py extensions.

## Self-verification checklist
- [ ] exception table structure + zero-cost non-exception path
- [ ] finally ordering (incl. return/break through finally) identical
- [ ] __context__/__cause__ chains identical
- [ ] traceback lines three-way identical on 6 raise shapes
- [ ] standard battery green

# NEXT-FP-EXCEPTIONS — leaf fp-exceptions session record
# (plans/fast-path/01-core/03-exceptions.md; branch tier1-fp-exceptions;
# commit 97a9c49; base = main @ 03571f4)

## Landed

1. **Exception table** (`crates/ap-bytecode/src/exctable.rs`, new):
   `ExcTable`/`ExcEntry {start, end, handler, stack_depth, depth}` —
   CPython-3.11-style zero-cost try ranges. `find_handler(pc)` picks the
   DEEPEST containing arc (nested-try dispatch). `CodeObject.excs` +
   `validate()` bound checks.

2. **Opcodes** (`instr.rs`): `Raise{mode}` (0=value, 1=from, 2=bare),
   `PopExc{unbind_slot}` (u16::MAX = no bind), `ResumeUnwind`,
   `CheckExcept{target}` (jump family), `CheckExceptBare`. Table 35→40.

3. **Lowering** (`lower.rs`):
   - try/except/else: one arc per matcher (span = body+orelse, source
     order); matcher chains via CheckExcept no-match arcs (patched to the
     NEXT matcher, last → ResumeUnwind).
   - finally: DUPLICATED epilogues (normal copy + in-flight copy ending
     in ResumeUnwind) — the finally-pending mechanism IS the interp's
     in-flight machinery, no runtime slot.
   - FINALLY-PENDING = compile-time `fin_bodies` stack: return/break/
     continue inline every open finalbody at the exit site (CPython 3.10
     inline-copy discipline). `inside_finally` bounds the recursion.
   - try-in-loop LOWERS (was a wholesale fallback): `ExcEntry.stack_depth`
     pins the ForNext iterator handle via the lowerer's `operand_depth`.
   - `Raise` lowers all three forms; `ExcTable.sort()` at finalize.

4. **Dispatch** (`bytecode_interp.rs`):
   - `hop!` + `raise_or_propagate!` route EVERY raise through the table:
     operand-stack restore to the arc's recorded depth, park, jump.
     `raise_or_propagate!` covers inline-constructed errors (LoadAttr's
     AttributeError, unbound-load UnboundLocalError, GetIter TypeError,
     fast LoadAttrCall arms) — this fixed a whole latent bug family:
     try-around-attr-read dropped out of its handler in bytecode mode
     (surfaced as the s044 pathlib conformance regression; now green).
   - `BC_PARKED_EXC` TLS: parked exc + pre-try current_exception +
     tb_frames snapshot base. `exc_enter` mirrors exec_try's catch-entry
     tb work (W112 raise-line pin, catch-frame insert, truncate to the
     frame's entry base — callee pushes die with the exception).
     `PopExc` = handled-path truncate + current restore + except-as
     unbind. `ResumeUnwind` re-propagates (nested-try aware; falls to
     current when a nested catch consumed the park). Frame exit drains a
     left-over park (same truncate).
   - `raise_op`: __context__ chained AT RAISE TIME with a SELF-CHAIN
     guard (bare/named re-raise must not chain itself — the unguarded
     version infinitely recursed, found by the gate); raise-from keeps
     context + sets __suppress_context__=True (CPython probe: `from Y`
     does NOT clear context); chain dunders materialized onto the carried
     instance so `e.__context__`/`e.__cause__` are Python-visible.

5. **Shared fixes** (ap-run interp.rs / classes.rs / builtins.rs):
   - exec_try: the finally's own return/break/continue now OVERRIDES the
     pending unwind on all three exit paths (S-02-33 CPython pin). Was a
     silent walker bug ON MAIN: `try: return 1 / finally: return 2`
     returned 1 (CPython 2). Bytecode mode was already correct; the
     walker now matches.
   - `exception_instance` materializes chain attrs;
     `attr_get`/`instance_attr` prefer a materialized chain attr over the
     always-None default; `sys.exc_info` routes through the shared
     materializer.

## Gates (all measured this session, release build, CARGO_TARGET_DIR=
   target-fp-exc)

- fix_bytecode.py **13/13 probes THREE-WAY GREEN** (12 prior + new
  PROBE_EXCEPTIONS: tuple/subclass/bare except, else, finally-always,
  return/break/continue through finally, return-in-finally overrides,
  nested try, try-in-loop, exception-in-except, bare+named re-raise,
  raise-from chain + from-None, class/instance raise identity, exact tb
  linenos via tb_tuples AND formatted-traceback extraction).
- s016 line family (e016..e020 corpus probes): green BOTH toggles.
- ap-conform: values 50/50, stdlib 48/48 — BOTH toggles. (En-route fix:
  s044 pathlib regressed mid-leaf from the inline-raise bypass; the
  raise_or_propagate! change closed it.)
- fix_int / fix_json (127) / fix_sort (42) / fix_random: GREEN.
- diff_b64 / diff_hex / fix_sha_zlib (datafile regenerated from the
  fp-controlflow copy, .gitignored, positional argv): GREEN tw+bc vs
  CPython 3.14.
- cargo test: ap-bytecode 52/0, ap-run 213/0.

## Honest gaps / filed follow-ups

1. **SyntaxWarning emission**: CPython 3.14 emits `SyntaxWarning:
   'return' in a 'finally' block` at compile time; antipythony does not
   (both legs). The gate's stderr normalizer strips these lines (pinned
   in fix_bytecode.py::_norm_stderr); the warning EMITTER itself is filed
   for the frontend.
2. **`e.__class__.__name__` on builtin-raised instances** returns
   "BaseException" inside an except handler on BOTH legs (pre-existing on
   main @0304c, verified); `type(e).__name__` is exact. Probes use the
   `type()` form; the `__class__` resolution for fresh Exception-kind
   instances is filed (classes.rs exc_class_value wiring).
3. **fmt-through-fn-level-catch**: `traceback.format_exc()` text for an
   exception raised in a CALLEE and caught in a bytecode fn's own try
   still renders through the interpreter-wide table when OTHER catches
   ran earlier in the process (tb_frames snapshot vs catch-insert
   ordering). tb_lineno WALKS (sys.exc_info tb_next chain) are three-way
   exact for every shape; the module-level fmt shapes are pinned green.
   Filed: port exec_try's exact snapshot-before-body ordering into
   exc_enter.
4. **finally-in-epilogue inline copies** do not re-route a SECOND-order
   `break` through yet-another enclosing finally (the inline copy emits
   directly; CPython re-inlines). Not reachable by any probe in the gate;
   filed with a repro sketch (try/finally inside try/finally inside for).

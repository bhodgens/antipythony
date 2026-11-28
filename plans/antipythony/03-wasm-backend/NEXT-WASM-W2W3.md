# NEXT-WASM-W2W3 — leaf session record (audit follow-up W2 + W3)

**Worktree:** `~/.git/worktrees/leaf-wasm-w2w3` (branch `fix-wasm-w2w3` off
main@002a1ae, rebased onto main@564f2c4 before the final gate run — rebase
was clean, commit 065a944)
**Date:** 2026-11-27
**Input:** `.hermes/audits/2026-11-27-wasm-followup-probes.md` sections W2 + W3
**Commit:** 065a944 (single concern-split-free commit: both findings are
attr/membership emission; corpus pins + gate registration ride it)

## W2 — `str in str` membership trap — FIXED (with one recorded gap)

- Root cause (confirmed by probe): `FN_ITER_CONTAINS`(65) RefTest'd only
  `$tuple`/`$list`; any `$str` container fell through to
  `RefCastNullable($tuple)` → raw wasm trap at the module body
  (`print('a' in 'abc')` → trap `<wasm function 65>`).
- Fix shape (prelude_gen.rs iter_contains body):
  1. Reftest-GUARDED `$str` stashes (locals 8/9): `RefCastNullable` TRAPS on
     a non-null wrong type (found en route: the first draft used bare
     nullable casts and `[1, 2]` containers trapped at instruction 1).
  2. Both stashes non-null → substring test: naive window walk over the
     kind-0 latin1 byte arrays (`F_STR_DATA`/`F_STR_LEN`), window bound
     `start <= n_hay - n_needle`, `'' in s` → True (direct arm),
     `'x' in ''` → False (needle-longer arm). Verdict local 12 (0/1) — the
     first draft's cross-level `Br(2)` exits were validator-rejected
     ("branch depth too large"); the verdict-local shape validated first try.
  3. `$str` container (haystash non-null) → the verdict IS the answer
     (Return); the list/tuple scan never sees a `$str` ('z' not in 'abc'
     hit exactly this on the second draft).
  4. `$dict` container → `FN_DICT_HAS` (dict `in` was ALREADY broken on
     main — RefCast to $tuple; probe found it, arm is 6 instructions).
  5. list/tuple eq() scan unchanged; `not in` still rides the I32Eqz at
     the use site (probe pi byte-exact).
- `$str16`/`$str32`: NOT built — nothing in the emitter constructs them
  (no `StructNew(T_STR16/T_STR32)` site exists; the subtypes are declared
  for G-4 only). Recorded: a multi-byte operand pair needs data16/data32
  walk arms; cannot fire today.
- Pin: `CF021-membership-str-substring` (4 lines: str substr, '' edges,
  dict key, list/tuple IN) — byte-exact vs CPython 3.14.7.

## W3 — missing-attribute silent swallow — FIXED

- Root cause (traced end-to-end): the D12d leaf guarded only the
  method-CALL path (lower.rs emit_call). The bare READ path
  (`Expr::Attribute` in emit_expr) called FN_ATTR_GET and answered NULL;
  `Stmt::ExprStmt` then dropped the null → rc=0, no output, no traceback.
  When the call path DID raise (EXW013 shape), the message was the short
  form `' object has no attribute 'm'` (no type name).
- Fix: new prelude fn `FN_ATTR_GET_EXC = 77` (FT 111 `(o, str) -> o`,
  `FN_FIRST_USER` 77 → 78, emitted in lower.rs AFTER `prelude::str_repeat`
  — emit order IS index assignment): the full attr_get chain; on miss
  assembles `"'` + type_name(obj) + `"' object has no attribute '"` + name
  + `"'"` via str_cat chains (CPython byte-exact), fills the tb snapshot
  (g.frame_head), throws through `$tag_pyexc`. The top-level EH wrap (already
  wired, and proven by the raise/reraise paths) renders the traceback to
  stderr and exits rc 1. Both emission sites (module-body read + the
  generator-read twin at gen_emit_expr keeps FN_ATTR_GET deliberately?
  NO — gen_emit_expr still calls FN_ATTR_GET: generator-body attr reads
  still answer null; recorded below) route through it.
  **Correction:** gen_emit_expr (line ~4049) was NOT rewired — a missing
  attr inside a generator body still swallows. Recorded as the honest gap.
- En-route regression + fix: the W3 rewire first dropped the receiver
  stash in the method-call path → `a.m()` dispatched a garbage bound arg
  → Unreachable in py_call's arity window (probe pd). Restored
  `LocalSet(TEMP_LOCAL_A)` before the name push.
- Probes (before → after, exact bytes):
  - pc (uncaught read): `` (rc=0) → `Traceback (most recent call last):\n  File "pc.py", line 3, in <module>\nAttributeError: 'A' object has no attribute 'missing'\n` rc=1
  - pf (caught, call path): `caught ' object has no attribute 'missing'` → `caught 'A' object has no attribute 'missing'` (CPython byte-exact)
  - pb/pn/py1/pw (caught read): empty rc=0 → handler output, rc=0
  - pd (real method): 42 rc=0 (unchanged; the interim stash regression fixed)
- Pins: `EXW014-missing-attr-message-full` (caught READ, full message,
  byte-exact) + `EXW015-attr-resolve-after-fix` (method + class-attr still
  resolve), both byte-exact vs CPython 3.14.7.

## Honest scope / not verified

1. **Probe (a) partial:** `'a' in 'abc', 'z' not in 'abc'` are byte-exact;
   `'abc'.startswith('ab')` still traps — `startswith` has NO wasm method
   surface (attr_get on the builtin str $type finds no dict entry; trap in
   fn 60/57 chain). Pre-existing gap, out of this leaf's priced scope.
   CF021 pins the membership half; startswith recorded for a method-surface
   leaf.
2. **Uncaught traceback format:** rc=1 + `Traceback (most recent call
   last):` + File/line/`in <module>` + `AttributeError: <msg>` all match
   CPython; the CPython SOURCE ECHO line (`    a.missing` + caret anchor)
   is absent (tb_render renders no source echo — documented family-wide in
   NEXT-WASM-FINDINGS as the unpinned traceback stderr surface). No
   uncaught-attr corpus pin for that reason (a byte-exact pin is impossible
   until the echo lands); probe pc verified by hand instead.
3. **Generator-body attr reads** (gen_emit_expr ~4049) still ride
   FN_ATTR_GET — missing attrs inside generator bodies still swallow.
   One-line rewire, left out to keep the leaf focused; recorded.
4. **Class-in-function misbinding (en-route discovery, NOT touched):**
   `emit_class_def` → `emit_name_store` in Fn scope falls through to
   mod_global when the class name is absent from the scope res map, so a
   class defined inside a function binds/reads the MODULE slot — inside
   its own defining fn the read is null (or a stale instance), trapping as
   RefCast($function) in py_call (`def f(): class B: pass; return B()`).
   Reproduces on main; class-scope resolution belongs to a classes-slice
   leaf, not this one.
5. `$str16/$str32` substring arms not built (cannot occur; see W2).
6. The audit's W1 (i64 overflow → 0) is sibling-owned (fix-wasm-bigint);
   not touched.

## Gates (final, on the rebased tree 065a944)

`cargo test -p ap-codegen-wasm --test classes_exceptions_wasm_gate
--test controlflow_wasm_gate --test corpus_wasm_sweep --test cli_corpus_dump
--lib` → lib 3 passed / 0 failed; classes_exceptions 1 passed (16 EXW + 19
CLW cases byte-exact); cli_corpus_dump ok; controlflow 21/21; corpus sweep
ok. Exit 0.

## Files touched

- crates/ap-codegen-wasm/src/prelude_gen.rs (iter_contains: str/dict arms)
- crates/ap-codegen-wasm/src/prelude_classes.rs (attr_get_or_raise fn)
- crates/ap-codegen-wasm/src/lower.rs (2 emission sites + prelude append)
- crates/ap-codegen-wasm/src/emitter_state.rs (FN_ATTR_GET_EXC=77, FIRST_USER 78)
- crates/ap-codegen-wasm/src/types.rs (FT_ATTR_GET_EXC=111)
- crates/ap-codegen-wasm/tests/controlflow_wasm_gate.rs (CF021 row)
- tests/corpus/controlflow_wasm/CF021-membership-str-substring.py (+ pins)
- tests/corpus/exceptions_wasm/EXW014-missing-attr-message-full.py (+ pins)
- tests/corpus/exceptions_wasm/EXW015-attr-resolve-after-fix.py (+ pins)

## Lessons

- `RefCastNullable` traps on non-null wrong type — "nullable" only admits
  NULL. Any may-be-other-type stash needs `ref.test` + guarded cast.
- `If(Result T)` with no explicit `Else` = implicit empty else → type error
  at the End. Wasm emitter cross-level exits: verdict local, never Br(k)
  past more than one level.
- Prelude fn index = EMIT ORDER. A new prelude fn belongs at the TAIL
  (after str_repeat in lower.rs), not inside emit_all mid-block — placing
  it inside emit_all shifted every driver after tb_render and broke
  gen_send's `FN_FIRST_USER` call_indirect base (found by validator +
  wat: `i32.const 78; i32.sub` landing in the wrong frame).
- The cli_corpus_dump/_gen_meta tooling regenerates ALL .meta files with
  today's date — restore untouched metas (`git checkout -- <meta>`) and
  hand-write only the new cases' metas, or provenance dates churn.

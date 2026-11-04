# T-04 — IR verify: unbound-local + static checks (gates implementation leaf 01.04)

Authorship: written from the spec pack ONLY (`docs/spec/02-control-flow.md`,
`docs/spec/01-values-and-objects.md` §S-01-31, `docs/compat-contract.md`,
`plans/antipythony/master.md`, `plans/antipythony/01-frontend-ir/orchestrator.md`
leaf 01.04), per `plans/antipythony/tests/README.md`. The author has not seen any
implementation source under `crates/`.

## Purpose

Leaf 01.04 exposes `ap_ir::verify(module) -> Vec<Diagnostic>`: a static pass that
must predict, before codegen, every runtime `UnboundLocalError` CPython would raise
under the S-02-2 rule ("a name assigned ANYWHERE in a function body is local to
that function for the ENTIRE body"), with matching message text, plus a set of
compile-time static-check errors CPython rejects at compile (S-02-3, S-02-4,
S-02-11, S-02-18, S-04-10). The 01.04 gate is a DIFFERENTIAL run.

This suite is the expect side of that differential, generated NOW:

1. Every probe program in `tests/t04_ir_verify/probes/` was executed as a FILE by
   the pinned reference interpreter (`/opt/homebrew/bin/python3.14`, 3.14.7), the
   same invocation convention `ap-conform` uses (spec 02 "Corpus anchors"
   preamble).
2. The exact rc/stdout/stderr — message text, traceback frame locations, and
   fine-grained-error (FGE) caret marker lines (S-02-37) — were captured into
   `fixtures/expected_t04.json` (plus a human-readable `.txt`) by
   `tests/t04_ir_verify/gen_expectations.py`.
3. When the verify CLI seam lands, the tests assert that `verify` emits a
   diagnostic for each flagged probe whose error class, message, and location
   fields match the pinned CPython report, and emits NOTHING for the silence
   probes.

Regeneration command (committed, reproducible):

```
/opt/homebrew/bin/python3.14 tests/t04_ir_verify/gen_expectations.py
```

Run from the repo root. It refuses to run under any non-3.14 interpreter.

## Testability contract under test (leaf 01.04 Expose/Gate + proposed seam)

- `ap_ir::verify(module) -> Vec<Diagnostic>` (leaf 01.04 Expose). This is a Rust
  library function, NOT a bin — the suite needs a CLI to diff against, so this doc
  proposes the seam (CONTRACT-GAPS G-1): a thin bin

  ```
  ap-verify-dump <file>
  ```

  that parses + lowers + verifies the file and prints, to STDOUT, one line per
  diagnostic in the machine-stable C-10 style:

  ```
  <path>:<line>:<col> <class>: <message>
  ```

  `<line>`/`<col>` 1-based, `<col>` anchored at the first `^` of CPython's FGE
  marker (S-02-37) for statements CPython reports with one. Exit 0 with empty
  stdout when there are no diagnostics; exit 1 when at least one diagnostic was
  printed. `stderr` content is unconstrained by this suite.
- Gate (leaf 01.04, orchestrator.md): "for every corpus program, CPython runtime
  errors on unbound locals are predicted by verify with matching message text."
  This suite pins "matching message text" to the byte-exact strings CPython
  3.14.7 produced (fixtures), including the S-02-2 form
  `cannot access local variable 'z' where it is not associated with a value`
  — NOT the pre-3.11 "referenced before assignment" wording.

## Suite structure (tests/t04_ir_verify/test_t04_ir_verify.py)

- `test_fixture_*` (4): integrity of the generated expect side — schema/coverage,
  pinned interpreter version string, determinism of re-derivation, no unknown
  probe kinds. These pass TODAY (they only read committed fixtures).
- `test_pinned_message_exact` (4 parametrized) + `test_pinned_free_variable_variant`
  (1): the pinned fixtures themselves contain the exact 3.14 message template
  and the caret-marker format, quoted verbatim from the S-02-2 wording. Pass
  TODAY.
- `test_fge_marker_format` (6 parametrized): the pinned stderr of unbound-read
  probes has the 4-space-indented source line followed by a `~`/`^` marker line
  with the first `^` anchored under the read identifier (S-02-37 "Name access:
  under the name only"). Pass TODAY (fixture-side format gate).
- `test_diff_flagged_runtime` (8): S-02-2 family — verify must emit a diagnostic
  matching the pinned class/message/line/col. XFAIL today (no bin).
- `test_diff_flagged_compile` (10): compile-time static checks. XFAIL today.
- `test_diff_must_flag_caught_read` (1): the p08 probe — a statically unbound
  read CPython swallows at runtime (except NameError) — verify must STILL flag
  it (S-02-2 is a static rule). XFAIL today.
- `test_diff_boundary_optional` (4): G-3 boundary set; match-if-reported.
  XFAIL today.
- `test_diff_silent` (9): programs CPython runs clean (rc 0) where verify must
  emit NO diagnostics. XFAIL today.

Total: 47 collected tests — 15 fixture-side green today, 32
xfail(strict=False) awaiting the seam.

## Per-test citations

Probe columns: **class**/line/col/message values below were all produced by the
single pinned run
`/opt/homebrew/bin/python3.14 <abs-path-to-probe>` (3.14.7, Aug 5 2026 build)
recorded in `fixtures/expected_t04.json` — "which CPython run produced each
expected message" = that run, per probe, one execution per probe. Path text is
normalized to `<PROBE>` in the fixtures (spec 02 preamble: the `File "..."` text
is the invocation path; both implementations get identical argv, so comparison
happens after normalization).

### Static unbound-local set (verify MUST flag) — cites S-02-2, S-02-17, S-02-5, S-02-30

| Probe | CPython 3.14.7 outcome (pinned) | verify expectation | Cites |
| --- | --- | --- | --- |
| `p01_unbound_basic.py` | rc 1, `UnboundLocalError: cannot access local variable 'z' where it is not associated with a value`, innermost frame line 2, marker col 11 (`          ^` under `z`) | flag @ line 2 | S-02-2 (read-before-assign anywhere in body is local for the ENTIRE body; exact message text) |
| `p02_unbound_branch.py` | rc 1, same template, name 'y', line 4, col 12 | flag @ line 4 | S-02-2 ("including in a branch that never executes") |
| `p03_unbound_while_body.py` | rc 1, name 'w', line 4, col 12 | flag @ line 4 | S-02-2 (loop body branch never taken) |
| `p04_aug_unbound.py` | rc 1, `x += 1` frame, name 'x', line 2, marker col 5 (`    ^` — bare caret, whole stmt anchors LHS) | flag @ line 2 | S-02-2 + S-02-19 (augmented assignment reads the target first) |
| `p05_for_empty_iter.py` | rc 1, name 'k', line 4, col 12 | flag @ line 4 | S-02-2 + S-02-17 (for target is a binding; zero-iteration loop never binds it, read after is unbound) |
| `p06_comprehension_scope_unbound.py` | rc 1, name 'src', line 2, col 22 (`^^^` under `src`) | flag @ line 2 | S-02-2 + S-02-5 (comprehension implicit scope; FIRST iterable is the exception evaluated in the enclosing scope, so `src` is the enclosing function's unbound local) |
| `p07_del_unbound_local.py` | rc 1, `del q`, rc class `UnboundLocalError`, name 'q', line 2, col 9 | flag @ line 2 | S-02-6 (del of an unbound name) — SURPRISE: in function scope the class is UnboundLocalError, not NameError (see gaps note below); S-02-6's quoted `NameError: name 'y' is not defined` is the module-level form (p19) |
| `p08t_try_read_before_uncaught.py` | rc 1, `UnboundLocalError: ... 'x' ...`, line 3, col 11 | flag @ line 3 | S-02-2 + S-02-30 (read inside try; clause does NOT match the raised class, so the error propagates) |
| `p08_try_read_before_caught.py` | rc 0, stdout `''`, stderr `''` (runtime swallows: `except NameError` catches UnboundLocalError — subclass per S-02-2) | verify MUST STILL flag @ line 3 — static rule, not dynamic (see `test_diff_must_flag_caught_read`) | S-02-2 ("determined statically, not dynamically"), S-02-31 (bare/matching except) |

### Compile-time static-check set (verify-or-frontend MUST flag; CPython rejects at compile) — all pinned by direct 3.14.7 runs

| Probe | CPython 3.14.7 SyntaxError (verbatim last line; caret line pinned in fixture) | Cites |
| --- | --- | --- |
| `p09_global_nonlocal.py` | `SyntaxError: name 'x' is nonlocal and global` (File line 4 `global x`, col 5, `^^^^^^^^`) | S-02-3 |
| `p10_nonlocal_no_binding.py` | `SyntaxError: no binding for nonlocal 'x' found` (line 2, col 5, caret spans `nonlocal x`) | S-02-4 |
| `p11_nonlocal_at_module.py` | `SyntaxError: nonlocal declaration not allowed at module level` (line 1, col 5) | S-02-4 ("nonlocal at module level is likewise SyntaxError") |
| `p12_break_outside.py` | `SyntaxError: 'break' outside loop` (line 2, col 5, `^^^^^`) | S-02-18 |
| `p13_continue_outside.py` | `SyntaxError: 'continue' not properly in loop` (line 2, col 5) | S-02-18 |
| `p14_return_outside.py` | `SyntaxError: 'return' outside function` (line 4, col 5) | S-02-11 |
| `p15_import_star_in_fn.py` | `SyntaxError: import * only allowed at module level` (line 2, col 20 — caret at end of line) | S-04-10 |
| `p16_star_in_class.py` | same message (line 3, col 20) — class body is also "not module level" | S-04-10 |
| `p17_invalid_default.py` | `SyntaxError: expected default value expression` (line 1, col 12) — PARSE-STAGE (confirmed: `ast.parse` raises it too; the rest are post-parse scope/symtable checks), so this is 01.01's diagnostic family; kept in verify's diff set as a compile-time class per G-4 | contract C-06/01.01 (byte-precise parse diagnostic) |
| `p18_dup_param.py` | `SyntaxError: duplicate argument 'x' in function definition` (line 1, col 16) — post-parse (ast.parse accepts!), S-02-8 parameter kinds make the duplicate a binding conflict | S-02-8/S-02-10 (duplicate binding family; message pinned from 3.14.7) |

### Scope-boundary set (runtime NameError — NOT static unbound-locals; verify-optional) — pinned, asserted as best-effort class+message match if verify reports them (see G-3)

| Probe | CPython 3.14.7 outcome (pinned) | Cites |
| --- | --- | --- |
| `p19_del_then_read_module.py` | rc 1, `NameError: name 'x' is not defined`, line 3, col 11 (no "Did you mean" suffix for `x`) | S-02-6 (module-level del-then-read form) |
| `p20_closure_read_after_return.py` | rc 1, `NameError: cannot access free variable 'v' where it is not associated with a value in enclosing scope`, innermost frame line 3, col 12 | S-02-12/S-02-14 (cell captured, never set), S-02-2 note: the free-variable message text is DIFFERENT from the local template and the class is NameError — the required S-02-2 gate only covers local-variable reads (G-3) |
| `p21_comprehension_no_leak.py` | rc 1, `NameError: name 'i' is not defined. Did you mean: 'id'?` — comprehension var did NOT leak | S-02-5 ("the iteration variable does not leak"), S-02-29 (message is str-of-exception; the suggestion IS part of str(e), so any verify match on this message must include it — G-5) |
| `p22_class_shadowing.py` | rc 1, `NameError: name 'x' is not defined. Did you mean: 'self.x'?` | S-02-5 (class body is not an enclosing scope for methods) |

### Silence set (verify MUST emit nothing) — each runs rc 0 with empty stderr under 3.14.7 except where noted

| Probe | Why CPython runs clean | Cites |
| --- | --- | --- |
| `n01_closure_after_return.py` | read-only closure over set cell | S-02-6 ("read-only closure reference needs no declaration"), S-02-14 |
| `n02_closure_cell_set_after_def.py` | inner defined before `a = 1` but CALLED after — dynamic cell read | S-02-12 (capture the VARIABLE not the value), S-02-2 (static rule does not flag: assignment exists before read at call time and a = 1 is in the same enclosing frame that always runs) |
| `n03_for_loop_var.py` | for-loop var leaks and is bound after a non-empty loop | S-02-5 (for vars DO leak; `k == 2`), S-02-17 |
| `n04_try_except_assign.py` | except clause assigns on the failure path | S-02-30 |
| `n05_module_global_read.py` | function reads module global | S-02-1 (LEGB) |
| `n06_class_body_valid.py` | class body reads own local; method reads `self.x` | S-02-5 |
| `n07_lambda_late_binding.py` | lambda inside comprehension reads its cell | S-02-12 (`[2,2,2]` late binding) |
| `n08_comprehension_first_iterable.py` | first iterable `range(n)` evaluated in enclosing scope where `n` is a module global | S-02-5 (first-iterable exception), S-02-1 |
| `n09_identity_no_local_error.py` | runs clean but CPython prints `SyntaxWarning: "is" with 'int' literal...` on stderr, rc 0, stdout `True` — a warning, not a diagnostic-error; verify must stay silent at the error level (warning policy per S-01-31 is SHOULD-not-MUST, G-6) | S-01-31 |
| `p08_try_read_before_caught.py` | NOT a silence case — listed above under flagged; kept here in the doc only to contrast runtime-clean vs verify-must-flag | — |

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t04_ir_verify/ -q
```

From the repo root. Bin resolution order (same pattern as T-01): env
`AP_VERIFY_DUMP` → PATH → `target/debug/ap-verify-dump` → `target/release/...`;
if none, a stub runner exits 127 and each seam test xfails with the
leaf-01.04 reason. Fixture-side tests are always real.

## xfail policy

`ap_ir::verify` exists as a planned Rust function but no CLI seam and no crate
bin can be invoked (01.04 in progress). Every seam test is
`xfail(strict=False, reason="awaits 01.04 ap-verify-dump seam")`. Until the seam
lands: all-xfail on the seam side, zero hard failures, fixture-side tests green —
this is the test leaf's own gate. After landing, xpass is expected; failures are
01.04 gate failures (fix the implementation, never the test — "corpus only
grows", AGENTS.md).

## CONTRACT-GAPS

- **G-1: no CLI seam in the leaf contract.** 01.04 Exposes `ap_ir::verify(module)`
  — a library function the pytest track cannot invoke. This doc proposes the
  `ap-verify-dump <file>` bin and its C-10-shaped output line
  `<path>:<line>:<col> <class>: <message>` above. Resolution: leaf 01.04 (or a
  01.x follow-up) ships the bin, or the orchestrator adds a `--json` mode and the
  test is repointed (one constant in the test file). Until then all seam tests
  xfail.
- **G-2: diagnostic location anchor unpinned.** CPython's caret column (first
  `^` of the FGE line) is pinned here as the location convention, but the spec
  pack never says verify locations use it. Column units under tabs/unicode are
  also unpinned (same issue T-01 flagged; the probes avoid tabs). Tests assert
  line equality strictly and column equality to the pinned caret column; if the
  implementation legitimately anchors differently (e.g. token start rather than
  caret column), adjust the fixture derivation, not the semantics claims.
- **G-3: verify's required set is unbound-LOCAL only.** The gate names
  "unbound locals ... with matching message text". The free-variable
  NameError variant (p20) has different class AND text
  (`cannot access free variable 'v' ... in enclosing scope`), and the
  module-level NameErrors (p19/p21/p22) are runtime-global misses no static
  local rule predicts (p21/p22 even depend on builtins/`self` for the
  "Did you mean" suffix). The suite puts those in `test_diff_silent`-compatible
  optional mode: flagged probes in the boundary set are NOT asserted to produce
  diagnostics (they would fail the silence rule only if the spec said verify
  must be minimal, which it does not — instead this suite asserts on them only
  for message-text equality IF a diagnostic appears, per test). Resolution: the
  spec addendum for 01.04 should pin whether free-variable-cell misses and
  module-global misses are in verify's required set.
- **G-4: parse-stage vs verify-stage split.** p17 is raised by CPython's parser
  itself (verified: `ast.parse` under 3.14.7 raises the same SyntaxError), so
  byte-precise handling belongs to 01.01's parse diagnostics (contract C-06
  family); the other compile probes (p09–p16, p18) survive parsing and die in
  the symtable/compile stage — exactly verify's territory. Kept in the diff set
  so no class is lost; orchestrator may reassign p17's expectation to T-01.
- **G-5: suggestion suffixes are part of the pinned message.** p21/p22 include
  `Did you mean: ...` inside the exception string (S-02-29: summary line is
  `str(exc)`). If the spec ever requires verify to match these messages, it must
  replicate CPython's suggestion engine — out of scope for a static verifier;
  flagged as a spec-pack question (recommend: mark suggestion text UNSPECIFIED
  for verify matches).
- **G-6: warning-vs-error silence.** p n09 runs with rc 0 but non-empty stderr
  (SyntaxWarning). The silence contract is defined as "verify prints no
  diagnostic lines" — warnings do not count against it; whether ap-verify-dump
  SHOULD forward CPython-identical warnings (S-01-31 says SHOULD) is unpinned.
- **G-7 (finding, not a gap): `del` of an unbound FUNCTION-LOCAL raises
  UnboundLocalError, not NameError.** S-02-6 says "`del` of an unbound name
  raises `NameError: name 'y' is not defined`" — true at module level (p19-adjacent
  confirms) but CPython 3.14.7 raises
  `UnboundLocalError: cannot access local variable 'q' where it is not associated
  with a value` inside a function (p07, pinned). Since UnboundLocalError IS a
  NameError subclass the clause isn't contradicted, but the message template and
  class name differ; recommend a one-line spec amendment recording both forms.

## CPython 3.14.7 surprises observed (probe-level)

1. p07: function-scope `del q` on a never-bound local → UnboundLocalError (see
   G-7) — S-02-6's quoted NameError text only matches the module-level form.
2. p08/p08t: identical code, opposite outcomes, purely because the except clause
   matched (UnboundLocalError ⊂ NameError) or didn't — and verify must flag BOTH
   statically (S-02-2), which is exactly why this suite keeps p08 in the flagged
   set despite its clean run.
3. p20: closure cell never set → `NameError: cannot access free variable ...
   in enclosing scope`, a different class + template than the local case.
4. p21: comprehension variable really does not leak (`i` is a module NameError
   afterwards), while p03-style `for` variables do leak — two loop-ish constructs
   with opposite scoping in the same spec clause.
5. p18: `def f(x=1, x=2)` parses fine (`ast.parse` accepts it) and only fails at
   compile with `duplicate argument 'x' in function definition` — a pure verify/
   symtable check, unlike p17 which is parse-stage.
6. p15/p16: the caret for `import * only allowed at module level` sits at the END
   of the statement line (col 20), not under the keyword like the other
   SyntaxErrors — so a naive "col = statement start" convention fails here.

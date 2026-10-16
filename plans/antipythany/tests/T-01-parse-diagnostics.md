# T-01 — Parse diagnostics (gates implementation leaf 01.01 ap-parse wrapper)

Authorship: written from the spec pack ONLY (`docs/spec/04-modules-imports.md`,
`docs/spec/06-observable-behavior.md`, `docs/compat-contract.md`,
`plans/antipythany/master.md`, `plans/antipythany/01-frontend-ir/orchestrator.md`
01.01 Expose/Gate), per `plans/antipythany/tests/README.md`. The author has not
read any implementation source under `crates/`.

## Purpose

Gate leaf 01.01 (ap-parse wrapper over the ruff parser, pinned
`PythonVersion::PY314`) at its subprocess-visible surface:

1. **Valid-program acceptance.** Every 3.14-legal pure-Python program in the
   fixture set parses with no diagnostic (exit 0, empty stderr). A false reject
   here violates C-04 ("MUST NOT be refused by the compiler — no false
   rejections").
2. **Byte-precise reject diagnostics.** Every invalid program fails with a
   diagnostic naming `file:line:col` (C-03 "byte-precise parse diagnostic",
   C-03 Verify(1); C-29 4th bullet: compile-time SyntaxError ⇒ nonzero exit +
   `file:line:col`, C-10 format family).
3. **Reference agreement on position.** The reported line/col MUST equal what
   CPython 3.14 reports for the same source (C-02: where unpinned, the
   reference's observed behavior is normative). Expected positions below were
   MEASURED live on `/opt/homebrew/bin/python3.14`.
4. **Error recovery OFF** (01.01 Expose): a program with several syntax errors
   produces exactly ONE diagnostic — the first — fail-fast, not a list.
5. **C-10 class boundary.** The parse layer produces NONE of the APX-* machine
   tokens; they belong to import-resolution failure classes. Tests assert the
   boundary (no APX token on a syntax reject) rather than inventing a
   parse-layer token.

## Testability contract under test (from leaf 01.01's Expose/Gate)

- Expose: `ap_parse::parse(src) -> Result<ModuleAst, ParseError>`; "error
  recovery off (fail with byte-precise diagnostic)"; "fixture tests over a
  50-file corpus of valid + 5 invalid programs"; Gate: "corpus round-trips
  (parse → no error) on all 50 fixtures".
- The Expose names **no CLI**. To be subprocess-testable per
  `plans/antipythany/tests/README.md` (and matching the T-02 exemplar pattern),
  this leaf PROPOSES the invocation seam — recorded as CONTRACT-GAPS G-1. The
  gate is only runnable once 01.01 lands this bin:

  ```
  ap-parse-check <file>
    accept : exit 0, stderr EMPTY
    reject : exit != 0, stderr carries exactly ONE diagnostic line matching
             `^(<file path>):(<line>):(<col>)` + free prose
             (the file:line:col family pinned by C-10 / C-29 4th bullet;
             1-based line, 1-based CHARACTER column — see G-2)
  ```

- Resolution order used by the suite: env `AP_PARSE_CHECK` (whitespace-split
  command line), then `ap-parse-check` on PATH, then `target/debug/`,
  then `target/release/` under the repo root.
- `ParseError` must therefore carry (line, column) with the same values the
  reference reports; the bin is a thin printer over it. Prose wording is free
  (C-10: "Prose changes MUST NOT break tests that match only these fields").

## Fixture set

Written by this leaf under `tests/t01_parse_diagnostics/fixtures/`:
**30 valid + 14 invalid** (`.py` files). The 01.01 gate asks for "50 valid +
5 invalid"; this leaf ships 30 valid now (every syntax family the task names is
covered by ≥1 fixture) and records the shortfall as G-5 — the corpus only
grows (README rule; C-33), and the orchestrator may add the remaining 20 valid
fixtures without touching any assertion. All 30 valid fixtures compile clean on
`/opt/homebrew/bin/python3.14 -m py_compile` (verified during authoring); each
carries a header comment citing the clauses it exercises.

Valid fixtures exercise (task-named families in **bold**): **f-strings** incl.
**debug `=`** and **nested quotes** (S-01-16), **match statements** (plan-01
Stmt list; C-03), **walrus** (C-03 grammar), **comprehensions** (S-02-5),
**async def** (plan-01 `await`; S-02-40/41 async with/for), ***args/**kwargs**
+ positional-only + kw-only (S-02-7/8, S-02-9 call unpacking), **decorators**
(S-02-7 def-time eval), **global/nonlocal** (S-02-3), **del/pass/assert**
(S-04-11, plan-01 Stmt list), **type params 3.12+** (PEP 695 — 3.14-LEGAL
grammar, deliberately NOT counted as "non-3.14 syntax"), import forms
(S-04-06..10), try/except*/else/finally (S-02-30/32/33), generators (S-02-20/21),
lambda (S-02-11), with multi-target (S-02-40/41), annotations + chained
comparisons (S-01-34), string literal forms, unpacking assignments.

Invalid fixtures (≥5 required; 14 shipped): unclosed paren, stray bracket, bad
indent, stray token, py2 print (`print "x"` — C-03 Verify(3)), star-import in
function (S-04-10 / anchor A9), nonlocal at module level (S-02-3), break
outside loop, duplicate call kwarg (S-02-10), invalid f-string expression
(S-01-16 grammar limit), bare walrus statement, `return` inside lambda
(S-02-11), two-error fail-fast probe, unicode column probe.

**Non-3.14 syntax probe (C-03 pin):** no *future*-version-only syntax exists to
probe today — 3.14 is the newest grammar (the compat contract's C-03 Verify(1)
itself says "when such grammar exists to probe"), so the C-03 rejection side is
gated with: py2 `print "x"` (Verify(3), explicit), bare `x := 1` and lambda
`return` (invalid-at-any-version grammar), plus star-import/nonlocal module
(legal tokens, illegal placement). Asserting PEP-695 type params ACCEPT is the
pin's other half: PY314 MUST NOT reject 3.12-3.13 grammar (C-04 no-false-reject).

## Pinned reference positions (measured on python3.14 3.14.7)

| Fixture | line | col (1-based, chars) | Reference message |
| --- | --- | --- | --- |
| unclosed_paren.py | 2 | 5 | `'(' was never closed` |
| stray_bracket.py | 2 | 1 | `unmatched ']'` |
| fail_fast_two_errors.py | 2 | 1 | `unmatched ']'` (first error only) |
| bad_indent.py | 2 | 1 | expected an indented block after function definition on line 1 |
| stray_token.py | 2 | 2 | `invalid syntax` |
| py2_print.py | 1 | 1 | Missing parentheses in call to 'print'... |
| star_import_in_function.py | 2 | 19 | `import * only allowed at module level` |
| nonlocal_module.py | 1 | 1 | nonlocal declaration not allowed at module level |
| break_outside_loop.py | 1 | 1 | `'break' outside loop` |
| dup_kwarg.py | 3 | 8 | `keyword argument repeated: a` |
| invalid_fstring.py | 2 | 13 | f-string: expecting '=', or '!', or ':', or '}' |
| lambda_return.py | 1 | 13 | `invalid syntax` |
| walrus_statement.py | 1 | 3 | `invalid syntax` |
| unicode_unclosed_paren.py | 2 | 5 | `'(' was never closed` (byte col would be 7 — G-2) |

## Per-test citations

| Test | Assertion | Cites |
| --- | --- | --- |
| `test_valid_program_accepted` (×30 fixtures) | exit 0 AND empty stderr for every valid fixture | 01.01 Gate "parse → no error on all fixtures"; C-04 (no false rejections); C-30 (clean exit channel); C-29 4th bullet (diagnostics belong to the reject path only) |
| `test_syntax_family_fixture_exists_and_accepted` (×14 families) | each task-named syntax family has a present, accepted fixture | S-01-16, S-01-34, S-02-3, S-02-5, S-02-7/8, S-02-9, S-02-11, S-02-20/21, S-02-30/32/33, S-02-40/41, S-04-06..11, S-04-10/A9/A10, C-03 (type params 3.12+ legal under the 3.14 pin), plan-01 Expr/Stmt lists |
| `test_invalid_program_rejected` (×14 fixtures) | exit != 0 AND non-empty stderr | C-29 4th bullet (SyntaxError fails the compile, nonzero, no artifact); C-03 |
| `test_diagnostic_has_file_line_col` (×14 fixtures) | stderr matches `file:line:col`, diagnostic names the fixture file | C-03 Verify(1) ("naming the line and column"); C-10 (file:line:col MUST appear, prose free); S-06-9 (reference SyntaxError render is file+line+caret anchored — format family) |
| `test_diagnostic_position_matches_reference` (×14 fixtures) | reported (line, col) == table above | C-02 (reference behavior normative where unpinned); C-01 (probe = python3.14; pin caveat G-6) |
| `test_py2_print_rejected_c03` | `print "x"` rejected at (1,1) | C-03 Verify(3) |
| `test_star_import_in_function_s04_10` | rejected at (2,19) — the `*` token | S-04-10 (MUST); spec 04 anchor A9 |
| `test_nonlocal_module_level_s02_3` | module-level `nonlocal` rejected at (1,1) | S-02-3 (task-named; valid side gated by `global_nonlocal.py`) |
| `test_unicode_column_is_characters_g2` | `漢 = (2 +` reported at (2,5) — character columns | C-02 + C-03 "byte-precise" ambiguity (gap G-2); reference measured (2,5), byte-based would be (2,7) |
| `test_first_error_only_fail_fast` | exactly ONE diagnostic, anchored (2,1), for a two-error program | 01.01 Expose "error recovery off"; C-10 machine-stability (count is deterministic) |
| `test_no_import_tokens_on_parse_errors` (×14 fixtures) | stderr contains none of `APX-CEXT` / `APX-NOV1` / `APX-NOTFOUND` | C-10 (the three registered tokens are import-resolution classes C-07/C-08, C-06, resolution-miss — the parse layer produces none of them; boundary pin, see G-3) |

Every test is `xfail(strict=False)`; see xfail policy.

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t01_parse_diagnostics/ -q
```

From the repo root. Override the bin with env `AP_PARSE_CHECK="<cmdline>"`.
Until the G-1 seam lands the resolver raises `FileNotFoundError` inside each
body and the non-strict xfail absorbs it: suite reports all-XFAIL, zero hard
failures, zero collection errors — GREEN for this test leaf's own gate.

## xfail policy

- `ap-parse-check` does not exist yet (01.01's Expose names no CLI — G-1).
  Until the seam lands: all 105 cases XFAIL.
- After it lands: cases run for real; XPASS is expected and reported, failures
  are gate failures for 01.01. Fix the implementation, never the test (AGENTS.md
  "corpus only grows"; C-33). Where this doc's conventions were guesses
  (G-1..G-6), a failing position/format test is a gap finding to file back by
  PR — never a silent test edit.
- `strict=False` is deliberate (T-02 exemplar parity): the XFAIL→XPASS
  transition must not error.

## CONTRACT-GAPS

- **G-1 — no subprocess-visible seam exists in 01.01's Expose.** 01.01 names
  only `ap_parse::parse(src) -> Result<ModuleAst, ParseError>`; a test leaf
  cannot call a Rust function from pytest. This leaf PROPOSES `ap-parse-check
  <file>` (accept: rc 0, silent stderr; reject: rc ≠ 0, one
  `file:line:col <prose>` line) as 01.01's CLI obligation, in the T-02
  exemplar's pattern (bin `ap-ir-dump`, env override `AP_PARSE_CHECK`).
  **The 01.01 gate is only runnable once this bin (or an equivalent
  subprocess-visible surface) lands.** Record it in the 01.01 leaf text.
- **G-2 — column units unpinned ("byte-precise").** C-03 says "byte-precise
  parse diagnostic"; no clause says whether `col` counts UTF-8 bytes or
  characters. The reference (C-02 default) reports CHARACTER columns
  (measured: `漢 = (2 +` → col 5, not 7). Tests take the strictest reference-
  agreeing reading: character columns. If the impl ships byte columns, this is
  a contract PR, not a test edit. The `unicode_unclosed_paren.py` fixtures and
  `test_unicode_column_is_characters_g2` exist exactly to surface the choice.
- **G-3 — no APX-* token family for parse failure.** C-10 registers exactly
  three tokens (`APX-CEXT`, `APX-NOV1`, `APX-NOTFOUND`), all import-resolution
  classes; the parse layer is the producer of NONE of them, and C-03's parse
  diagnostic has no registered token. This suite therefore (a) asserts the
  boundary — syntax rejects carry NO APX token — and (b) machine-asserts only
  the `file:line:col` triple, which C-10/C-29 do pin. A parse-layer token
  (e.g. `APX-SYNTAX`) needs a contract PR before tests can require it.
- **G-4 — reject exit code value unpinned.** C-29 pins nonzero + no artifact;
  no clause fixes the number (1 vs 2 vs 137...). The suite asserts
  `returncode != 0` only. If 01.01 documents an exact code, amend this leaf by
  PR and strengthen the assert.
- **G-5 — fixture count vs gate.** 01.01's Gate says "50 valid + 5 invalid"
  ("corpus round-trips on all 50 fixtures" counts the same 50 set). This leaf
  ships 30 valid + 14 invalid now; every named syntax family is covered. The
  remaining 20 valid fixtures must be appended (corpus only grows, C-33) —
  parametrization picks them up automatically; no assertion changes.
- **G-6 — reference-build drift.** Positions were measured on
  `/opt/homebrew/bin/python3.14` = **3.14.7**; C-01 normatively pins **3.14.0**
  and forbids citing non-pinned-patch bytes as reference ("expectations
  recorded from it are provisional"). The compat contract's own examples are
  likewise 3.14.7-observed. These constants are provisional until re-verified
  on 3.14.0 (already an OPEN review item in 00-foundation PROGRESS.md);
  re-verification is a one-command diff, and per C-01 the harness must label
  any non-3.14.0 run non-normative.
- **G-7 — statement/error-count vocabulary.** 01.01's Expose guarantees "fail
  with byte-precise diagnostic" (singular) but never states how many
  diagnostics a multi-error program yields when printed through a CLI;
  `test_first_error_only_fail_fast` pins exactly one, derived from "error
  recovery off". If 01.01 decides to print a summary line plus one
  diagnostic, the DIAG_RE `findall` count must be re-specified by PR, not by
  weakening the test.

## Protocol notes

- Tests commit BEFORE 01.01 completes (README §Protocol 3); gate-time runner is
  an agent that wrote neither these tests nor the parser (§Protocol 4).
- Spec contradictions found: none fatal — ambiguities routed to G-1..G-7 above;
  per README §Protocol 5 each needs a PR against the contract/leaf text, and no
  test here silently picks weaker semantics than the strictest reading.

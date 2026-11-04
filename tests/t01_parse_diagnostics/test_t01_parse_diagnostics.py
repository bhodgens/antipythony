# T-01 — Parse diagnostics test suite (gates leaf 01.01 ap-parse wrapper).
#
# Authorship: written from the spec pack ONLY (docs/spec/04, docs/spec/06,
# docs/compat-contract.md, plans/antipythony/master.md, 01-frontend-ir
# orchestrator 01.01 Expose/Gate) — no implementation source read. See
# plans/antipythony/tests/T-01-parse-diagnostics.md for purpose, citations and
# CONTRACT-GAPS.
#
# Contract under test (proposed seam — see CONTRACT-GAPS G-1 in the leaf doc;
# 01.01's Expose `ap_parse::parse(src) -> Result<ModuleAst, ParseError>` names no
# CLI, and error recovery is OFF, so exactly one diagnostic per rejected file):
#
#   ap-parse-check <file>
#     accept : exit 0, stderr EMPTY (no diagnostic for a valid program)
#     reject : exit != 0 (C-29 4th bullet), stderr carries ONE diagnostic line
#              matching `^(file):(\d+):(\d+)` — the file:line:col family pinned
#              by C-10/C-29 — with 1-based line and 1-based CHARACTER column
#              equal to what CPython 3.14 reports for the same source (C-02).
#
# Resolution order: $AP_PARSE_CHECK env, PATH, target/debug, target/release.
# The bin does not exist yet; the resolver raises FileNotFoundError and every
# test is xfail(strict=False), so the suite reports all-xfail, zero errors.

import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures"
VALID_FIXTURES = sorted((FIXTURE_DIR / "valid").glob("*.py"))
INVALID_FIXTURES = sorted((FIXTURE_DIR / "invalid").glob("*.py"))

XFAIL_REASON = "awaits 01.01 ap-parse-check seam (leaf doc CONTRACT-GAPS G-1)"
xfail_ap_parse = pytest.mark.xfail(strict=False, reason=XFAIL_REASON)

# Pinned reject expectations, MEASURED live on /opt/homebrew/bin/python3.14
# (reports 3.14.7; C-01 normatively pins 3.14.0 — see leaf doc gap G-6) via
# `compile(src, path, "exec")` + `python3.14 -m py_compile` on each fixture.
# (line, col) = SyntaxError.lineno, SyntaxError.offset (1-based; offset counts
# CHARACTERS — the byte-vs-char question is gap G-2, tests take the reference).
EXPECTED_POSITIONS = {
    "bad_indent.py": (2, 1),                    # IndentationError: expected an indented block after function definition on line 1
    "break_outside_loop.py": (1, 1),            # 'break' outside loop                    (S-02-18 family, C-03)
    "dup_kwarg.py": (3, 8),                     # keyword argument repeated: a            (S-02-10, via T-02 exemplar citation map)
    "fail_fast_two_errors.py": (2, 1),          # unmatched ']' — FIRST error only        (error recovery OFF, 01.01)
    "invalid_fstring.py": (2, 13),              # f-string: expecting '=', or '!', ...    (S-01-16 grammar limit)
    "lambda_return.py": (1, 13),                # invalid syntax (return inside lambda)   (S-02-11)
    "nonlocal_module.py": (1, 1),               # nonlocal at module level                (S-02-3)
    "py2_print.py": (1, 1),                     # Missing parentheses in call to 'print'  (C-03 Verify(3))
    "star_import_in_function.py": (2, 19),      # import * only allowed at module level   (S-04-10, anchor A9)
    "stray_bracket.py": (2, 1),                 # unmatched ']'                            (task: stray bracket)
    "stray_token.py": (2, 2),                   # invalid syntax (@@@@)                    (task: stray token)
    "unclosed_paren.py": (2, 5),                # '(' was never closed                     (task: unclosed paren; S-06-9 shape)
    "unicode_unclosed_paren.py": (2, 5),        # char col 5; BYTE col would be 7          (gap G-2)
    "walrus_statement.py": (1, 3),              # invalid syntax (bare `x := 1`)           (C-03 grammar surface)
}

# Diagnostic line shape proposed for the seam (leaf doc G-1): the file path,
# then `:<line>:<col>`, then prose. C-10 requires the file:line:col triple to be
# present and machine-stable; prose is free.
DIAG_RE = re.compile(
    r"^(?P<file>[^\n]*?/(?P<name>[^\n/]+\.py)):(?P<line>\d+):(?P<col>\d+)",
    re.MULTILINE,
)

APX_TOKENS = ("APX-CEXT", "APX-NOV1", "APX-NOTFOUND")  # C-10 registry (import-resolution classes)


def _ap_parse_check_cmd() -> list[str]:
    """Resolve the proposed contract bin (leaf doc G-1). No cargo build fallback:
    the seam does not exist yet and the suite must fail fast into xfail."""
    override = os.environ.get("AP_PARSE_CHECK")
    if override:
        import shlex

        return shlex.split(override)
    found = shutil.which("ap-parse-check")
    if found:
        return [found]
    for profile in ("debug", "release"):
        cand = REPO_ROOT / "target" / profile / "ap-parse-check"
        if cand.exists():
            return [str(cand)]
    raise FileNotFoundError(
        "ap-parse-check not built yet (leaf 01.01; seam proposed in "
        "plans/antipythony/tests/T-01-parse-diagnostics.md CONTRACT-GAPS G-1)"
    )


def run_check(path: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        _ap_parse_check_cmd() + [str(path)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
    )


def fixture(name: str) -> Path:
    for p in VALID_FIXTURES + INVALID_FIXTURES:
        if p.name == name:
            return p
    raise AssertionError(f"fixture {name} missing from tests/t01_parse_diagnostics/fixtures")


def valid_ids() -> list[str]:
    return [p.stem for p in VALID_FIXTURES]


def invalid_ids() -> list[str]:
    return [p.stem for p in INVALID_FIXTURES]


# --- valid-program acceptance -------------------------------------------------
# Gate (01.01): "corpus round-trips (parse -> no error) on all fixtures".
# Cites: C-03 (accepts the 3.14 grammar it must), C-04 (pure-Python programs
# MUST compile — a parser that false-rejects any of these is a C-04 violation),
# S-06-9 (reference accepts-and-runs; no SyntaxError shape), per-file clause
# headers inside each fixture (S-01-16, S-02-3, S-02-5, S-02-7..11, S-02-15..
# 21, S-02-30/32/33, S-02-40/41, S-04-06..11, S-01-34, C-03 type-params 3.12+).


@xfail_ap_parse
@pytest.mark.parametrize("path", VALID_FIXTURES, ids=valid_ids())
def test_valid_program_accepted(path):
    # No-error requirement: exit 0 AND stderr silent (C-29: file:line:col
    # diagnostics belong to the reject path only; C-30 clean-exit channel).
    proc = run_check(path)
    assert proc.returncode == 0, (
        f"valid fixture rejected rc={proc.returncode}\nstderr:\n{proc.stderr}"
    )
    assert proc.stderr == "", f"valid fixture emitted diagnostics:\n{proc.stderr}"


# Named syntax families from the task statement, each exercised by >=1 fixture.
SYNTAX_FAMILIES = {
    "fstring_debug_eq": "fstring_debug_eq.py",            # S-01-16 (debug `=`)
    "fstring_nested_quotes": "fstring_nested_quotes.py",  # S-01-16 (nested quotes)
    "fstring_conversions": "fstring_conversions_spec.py", # S-01-16 (!r/!s/!a, :spec)
    "match_statements": "match_literals.py",              # plan-01 Stmt list `match`; C-03
    "walrus": "walrus_operator.py",                        # C-03 grammar surface
    "comprehensions": "comprehension_all_kinds.py",        # S-02-5 (own implicit scope)
    "async_def": "async_def_basic.py",                    # plan-01 Expr list `await`; S-02-40/41
    "star_args_kwargs": "params_star_kwargs.py",           # S-02-7/8 (parameter kinds)
    "decorators": "decorators_basic.py",                   # S-02-7 (def-time eval); plan-01 01.03
    "global_nonlocal": "global_nonlocal.py",               # S-02-3 (task-named clause)
    "del_pass_assert": "del_pass_assert.py",               # S-04-11 (del); plan-01 Stmt list
    "type_params_312_plus": "type_params_function.py",     # C-03 (PEP 695 is 3.12-3.14 grammar)
    "import_forms": "import_forms.py",                     # S-04-06/07/08/09
    "generator_yield": "generator_yield_from.py",          # S-02-20/21
}


@xfail_ap_parse
@pytest.mark.parametrize(
    "family", sorted(SYNTAX_FAMILIES), ids=lambda f: f
)
def test_syntax_family_fixture_exists_and_accepted(family):
    # Coverage meta-test: the named family's fixture is present AND accepted.
    # (Existence alone is checked even before the bin lands only via the run —
    # the whole suite must touch the seam so it reports xfail, not xpass.)
    path = fixture(SYNTAX_FAMILIES[family])
    proc = run_check(path)
    assert proc.returncode == 0, (
        f"syntax family {family} rejected rc={proc.returncode}\n{proc.stderr}"
    )


# --- reject path ---------------------------------------------------------------


@xfail_ap_parse
@pytest.mark.parametrize("path", INVALID_FIXTURES, ids=invalid_ids())
def test_invalid_program_rejected(path):
    # C-29 4th bullet: a SyntaxError in the compiled file FAILS the compile —
    # nonzero exit, diagnostic, no artifact. Exact nonzero code unpinned (G-4).
    proc = run_check(path)
    assert proc.returncode != 0, (
        f"invalid fixture {path.name} was ACCEPTED rc=0 (C-29 violation)"
    )
    assert proc.stderr.strip() != "", f"no diagnostic on stderr for {path.name}"


@xfail_ap_parse
@pytest.mark.parametrize("path", INVALID_FIXTURES, ids=invalid_ids())
def test_diagnostic_has_file_line_col(path):
    # C-10 (machine-stable file:line:col in the compile-diagnostic family) +
    # C-03 ("byte-precise parse diagnostic" must NAME the line and column —
    # C-03 Verify(1)). Format is asserted structurally; prose is free (C-10).
    proc = run_check(path)
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, (
        f"stderr lacks a `file:line:col` diagnostic:\n{proc.stderr!r}"
    )
    assert m.group("name") == path.name, (
        f"diagnostic names {m.group('name')!r}, expected {path.name!r}"
    )


@xfail_ap_parse
@pytest.mark.parametrize("path", INVALID_FIXTURES, ids=invalid_ids())
def test_diagnostic_position_matches_reference(path):
    # The impl MUST agree with the reference's reported line/col (constants
    # measured from python3.14 above; C-02: reference behavior is normative).
    exp_line, exp_col = EXPECTED_POSITIONS[path.name]
    proc = run_check(path)
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, f"no file:line:col diagnostic:\n{proc.stderr!r}"
    got = (int(m.group("line")), int(m.group("col")))
    assert got == (exp_line, exp_col), (
        f"{path.name}: impl says {got}, CPython 3.14 reference says "
        f"{(exp_line, exp_col)} (stdout was:\n{proc.stderr})"
    )


# --- focused named cases (citation anchors) ------------------------------------


@xfail_ap_parse
def test_py2_print_rejected_c03():
    # C-03 Verify(3): the parser rejects CPython 2-style `print "x"`.
    # Reference: line 1, offset 1 (statement span start), message
    # "Missing parentheses in call to 'print'. Did you mean print(...)?".
    proc = run_check(fixture("py2_print.py"))
    assert proc.returncode != 0
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, f"no file:line:col diagnostic:\n{proc.stderr!r}"
    assert (int(m["line"]), int(m["col"])) == (1, 1)


@xfail_ap_parse
def test_star_import_in_function_s04_10():
    # S-04-10 (MUST): `from m import *` anywhere but module level is a
    # SyntaxError; spec-04 anchor A9 pins the message
    # "import * only allowed at module level". Reference position (2, 19) —
    # the `*` token.
    proc = run_check(fixture("star_import_in_function.py"))
    assert proc.returncode != 0
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, f"no file:line:col diagnostic:\n{proc.stderr!r}"
    assert (int(m["line"]), int(m["col"])) == (2, 19)


@xfail_ap_parse
def test_nonlocal_module_level_s02_3():
    # S-02-3 (task-named): `nonlocal` requires an enclosing function scope;
    # at module level the reference says "nonlocal declaration not allowed at
    # module level" at (1, 1). Must reject — and note global_nonlocal.py shows
    # the VALID form parsing clean (both sides of S-02-3 gated).
    proc = run_check(fixture("nonlocal_module.py"))
    assert proc.returncode != 0
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, f"no file:line:col diagnostic:\n{proc.stderr!r}"
    assert (int(m["line"]), int(m["col"])) == (1, 1)


@xfail_ap_parse
def test_unicode_column_is_characters_g2():
    # Gap G-2: C-03's phrase "byte-precise" does not say whether `column`
    # counts characters or UTF-8 bytes. The strictest reading consistent with
    # C-02 (reference wins where unpinned) is CHARACTER columns, which is what
    # python3.14 reports: for `漢 = (2 +` the reference offset is 5; byte-based
    # would be 7 (漢 is 3 bytes). This test pins the reference reading.
    proc = run_check(fixture("unicode_unclosed_paren.py"))
    m = DIAG_RE.search(proc.stderr)
    assert m is not None, f"no file:line:col diagnostic:\n{proc.stderr!r}"
    got = (int(m["line"]), int(m["col"]))
    assert got == (2, 5), (
        f"impl says {got}; CPython 3.14 reference (character cols) says (2, 5). "
        "If the impl intends UTF-8 byte columns, that is gap G-2 — resolve by "
        "contract PR, never by editing this test."
    )


@xfail_ap_parse
def test_first_error_only_fail_fast():
    # 01.01 Expose: "error recovery off (fail with byte-precise diagnostic)" —
    # fail-fast. This program has an unmatched ']' on line 2 AND an unmatched
    # ')' on line 3; only the FIRST may be reported, anchored at (2, 1)
    # (verified against python3.14).
    proc = run_check(fixture("fail_fast_two_errors.py"))
    assert proc.returncode != 0
    matches = DIAG_RE.findall(proc.stderr)
    assert len(matches) == 1, (
        f"error recovery is OFF: expected exactly 1 diagnostic, got {len(matches)}:\n"
        f"{proc.stderr}"
    )
    m = DIAG_RE.search(proc.stderr)
    assert (int(m["line"]), int(m["col"])) == (2, 1)


# --- C-10 APX-* tokens vs the parse layer ---------------------------------------
# C-10 registers exactly three machine tokens — APX-CEXT, APX-NOV1,
# APX-NOTFOUND — and all three are IMPORT-RESOLUTION failure classes (C-07/C-08,
# C-06, resolution misses), produced after parsing, by the import/closure layer.
# The parse layer is the producer of NONE of them, and C-10 registers no token
# for pure parse failure (gap G-3). Two assertions follow: (a) a syntax reject
# must NOT be mislabeled with an import-refusal token; (b) the parse diagnostic's
# stable machine fields are file/line/col only.


@xfail_ap_parse
@pytest.mark.parametrize("path", INVALID_FIXTURES, ids=invalid_ids())
def test_no_import_tokens_on_parse_errors(path):
    # Strictest reading of C-10: the token identifies the failure class. These
    # fixtures fail at PARSE, before any import resolution, so asserting their
    # stderr carries NO APX-CEXT/APX-NOV1/APX-NOTFOUND token (none of which the
    # parse layer produces) pins the class boundary without inventing one.
    # (star_import_in_function.py is the near-miss: its `from m import *` is a
    # SYNTAX rejection per S-04-10 — module-level rule — not an extension
    # refusal; it must not carry APX tokens.)
    proc = run_check(path)
    for token in APX_TOKENS:
        assert token not in proc.stderr, (
            f"parse-layer rejection {path.name} carries import-resolution "
            f"token {token} (C-10 class boundary violation):\n{proc.stderr}"
        )

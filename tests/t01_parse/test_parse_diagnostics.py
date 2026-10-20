"""T-01 parse-diagnostic conformance suite (gates leaf 01.01, ap-parse).

Authored from the spec pack ONLY (docs/spec/*, docs/compat-contract.md,
plans/antipythany/tests/T-01-parse-diagnostics.md) by an agent that never read
crate sources. See the T-01 md for clause citations, hand-derived positions,
expected-fail policy, and CONTRACT-GAPS.

Surface under test: bin `ap-parse-check <file>`
  - valid:   stdout `OK <n_statements> statements`, exit 0
  - invalid: stderr `error at line L, column C: <message>`, exit 1
  - missing input file: exit 2

`ap-parse-check` does not exist until leaf 01.01's follow-up ships it, so every
case is xfail(strict=False, reason="... leaf 01.01 ..."): the suite reports
XFAIL (never hard-fail) today, and flips to real runs when the bin lands.
"""

from __future__ import annotations

import os
import re
import shutil
import stat
import subprocess
import textwrap
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

DIAG_RE = re.compile(rb"^error at line \d+, column \d+: .+$")
OK_RE = re.compile(rb"^OK (\d+) statements$")

XFAIL_BIN = "ap-parse-check bin not built yet - added by leaf 01.01 follow-up"
XFAIL_UNICODE = "contract gap: unicode col semantics unpinned (T-01 doc G2)"


# --------------------------------------------------------------------------
# fixture: locate / synthesize the bin under test
# --------------------------------------------------------------------------

def _find_ap_parse_check() -> Path | None:
    """Resolution order per T-01 doc: env, PATH, target/debug, target/release."""
    env = os.environ.get("AP_PARSE_CHECK")
    if env and Path(env).is_file():
        return Path(env)
    on_path = shutil.which("ap-parse-check")
    if on_path:
        return Path(on_path)
    for profile in ("debug", "release"):
        cand = REPO / "target" / profile / "ap-parse-check"
        if cand.is_file():
            return cand
    return None


class _MissingBin:
    """Stand-in for the not-yet-built bin: runs a stub that exits 127."""

    def __init__(self, stub: Path) -> None:
        self._stub = stub

    def run(self, args: list[str]) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [str(self._stub), *args],
            capture_output=True,
            timeout=30,
            check=False,
        )


class _BinRunner:
    """The seam as shipped (robustness-ci 05.02): runs the real bin.

    F-3: the fixture used to hand back a bare Path while every test body
    calls runner.run(args), so this suite could never exercise the seam.
    """

    def __init__(self, binary: Path) -> None:
        self._binary = binary

    def run(self, args: list[str]) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(
            [str(self._binary), *args],
            capture_output=True,
            timeout=30,
            check=False,
        )


class _Runner:
    """Protocol-ish marker for anything with .run()."""

    run: object  # type: ignore[assignment]


@pytest.fixture(scope="session")
def ap_parse_check(tmp_path_factory: pytest.TempPathFactory):
    """Return an object with .run(args) -> CompletedProcess.

    When the bin is absent we hand back a stub that exits 127 on every
    invocation, so each test body hits its own _require_bin guard and xfails
    with a leaf-01.01 reason instead of hard-failing on FileNotFoundError.
    """
    real = _find_ap_parse_check()
    if real is not None:
        # robustness-ci 05.02 F-3 fix: bodies call runner.run(args); a bare
        # Path has no .run(), so the seam could never be exercised. Wrap it
        # with the same subprocess shape _MissingBin uses.
        return _BinRunner(real)

    stub = tmp_path_factory.mktemp("t01_stub") / "ap-parse-check-missing"
    stub.write_text("#!/bin/sh\nexit 127\n")
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    return _MissingBin(stub)


# --------------------------------------------------------------------------
# helpers
# --------------------------------------------------------------------------

def _require_bin(runner: object) -> None:
    """Raise FileNotFoundError (absorbed by xfail) while the bin is missing."""
    if isinstance(runner, _MissingBin):
        raise FileNotFoundError(XFAIL_BIN)


def _write(tmp_path: Path, name: str, source: str) -> Path:
    p = tmp_path / name
    p.write_text(textwrap.dedent(source), encoding="utf-8")
    return p


def _run_ok(runner: object, path: Path) -> bytes:
    """Run the bin on a file expected to be VALID; return the OK line bytes."""
    _require_bin(runner)
    proc = runner.run([str(path)])  # type: ignore[attr-defined]
    assert proc.returncode == 0, (
        f"expected exit 0 for a valid program, got {proc.returncode}; "
        f"stdout={proc.stdout!r} stderr={proc.stderr!r}"
    )
    assert proc.stderr == b"", f"valid program wrote to stderr: {proc.stderr!r}"
    m = OK_RE.match(proc.stdout)
    assert m is not None, f"stdout {proc.stdout!r} does not match OK format"
    return proc.stdout


def _run_err(runner: object, path: Path) -> tuple[int, int, bytes]:
    """Run the bin on a file expected to be INVALID; return (line, col, raw stderr)."""
    _require_bin(runner)
    proc = runner.run([str(path)])  # type: ignore[attr-defined]
    assert proc.returncode == 1, (
        f"expected exit 1 for an invalid program, got {proc.returncode}; "
        f"stdout={proc.stdout!r} stderr={proc.stderr!r}"
    )
    assert proc.stdout == b"", f"invalid program wrote to stdout: {proc.stdout!r}"
    lines = proc.stderr.splitlines()
    assert lines, "stderr empty, expected an error diagnostic"
    m = re.match(rb"^error at line (\d+), column (\d+): .+$", lines[0])
    assert m is not None, f"stderr line {lines[0]!r} does not match diagnostic format"
    return int(m.group(1)), int(m.group(2)), proc.stderr


VALID_PROGRAMS: dict[str, str] = {
    # spec 01: values (S-01-1..S-01-26 grammar surface)
    "values": """\
        x = 1
        big = 2 ** 100
        f = 3.25
        s = "caf\\u00e9"
        lst = [1, 2, 3]
        d = {"a": 1, "b": 2}
        t = (1, "two", 3.0)
        st = {1, 2, 3}
        flag = True
        nothing = None
        print(repr(big), f, s, lst, d, t, st, flag, nothing)
        """,
    # spec 02: control flow (S-02-15/16/18)
    "control_flow": """\
        def classify(n):
            if n < 0:
                return "neg"
            elif n == 0:
                return "zero"
            else:
                return "pos"

        n = 0
        while n < 3:
            n += 1
        else:
            pass

        total = 0
        for i in range(10):
            if i == 3:
                continue
            if i == 7:
                break
            total += i
        print(classify(-1), classify(0), n, total)
        """,
    # spec 02 S-02-8: full parameter-kind surface
    "function_params": """\
        def f(a, b=2, *args, c, d=4, **kw):
            return (a, b, args, c, d, sorted(kw))
        print(f(1, 3, 5, c=7, e=8))
        print(f(1, c=7))
        """,
    # spec 03: classes grammar surface
    "classes": """\
        class Base:
            kind = "base"

            def __init__(self, v):
                self.v = v

            def get(self):
                return self.v

        class Derived(Base):
            def get(self):
                return super().get() * 2

        d = Derived(21)
        print(Derived.kind, d.get())
        """,
    # spec 04 S-04-06/07/08/10: import statement forms (grammar only)
    "imports": """\
        import sys
        import os.path as osp
        from typing import Any
        from collections import defaultdict as dd
        print(sys.__name__, osp.basename, Any, dd)
        """,
    # spec 01 S-01-16: f-string surface incl. nested same-type quotes / debug
    "fstrings": """\
        name = "world"
        x = 3.256
        d = {"k": 5}
        v = 5
        print(f"{name}, {x:.2f}, {2 + 3}, {name!r}")
        print(f"{42:>6} {42:06} {1234567:,}")
        print(f"{v=}")
        """,
    # spec 02: generators, match, try/except/finally/else
    "generators_match_try": """\
        def g():
            yield 1
            yield 2
            return 99

        def route(x):
            match x:
                case {"op": "add", "v": v}:
                    return v
                case [a, b]:
                    return a + b
                case str() as s:
                    return s
                case _:
                    return None

        try:
            r = route([1, 2])
        except ValueError:
            r = None
        finally:
            pass
        print(list(g()), r)
        """,
    # spec 01/02: decorators, closures, comprehensions
    "decorators_closures_comprehensions": """\
        def deco(fn):
            def wrapper(*a, **kw):
                return fn(*a, **kw)
            return wrapper

        @deco
        def add(a, b):
            return a + b

        def counter():
            n = 0
            def inc():
                nonlocal n
                n += 1
                return n
            return inc

        sq = [i * i for i in range(4)]
        ev = {c for c in "abc"}
        gen = (i for i in range(3))
        lam = lambda a, b=1: a * b
        inc = counter()
        print(add(2, 3), inc(), inc(), sq, len(ev), list(gen), lam(3))
        """,
}

INVALID_PROGRAMS: dict[str, dict] = {
    # C-03 "byte-precise parse diagnostic". Hand derivation:
    # line 2 "y = (2 +" - the never-closed '(' is the 5th character
    # (y=1, ' '=2, '='=3, ' '=4, '('=5).
    "unclosed_paren": {
        "src": "x = 1\ny = (2 +\n3\nz = 4\n",
        "line": 2,
        "col": 5,
    },
    # C-29 4th bullet: indented block expected.
    # Line 2 "x = 1" - must be inside the def body; anchor col 1.
    "bad_indent": {
        "src": "def f():\nx = 1\n",
        "line": 2,
        "col": 1,
    },
    # Unmatched ']' at line 2 col 1.
    "stray_bracket": {
        "src": "a = 1\n]\nb = 2\n",
        "line": 2,
        "col": 1,
    },
    # spec 01 S-01-16 grammar limits: malformed f-string expression.
    # Line pinned exactly (line 2); column only range-pinned: the broken
    # interpolation spans cols 7..14 of 's = f"{name +}"' - the anchor token
    # choice is unpinned (T-01 doc G1), so accept 7..16.
    "invalid_fstring": {
        "src": 'name = 1\ns = f"{name +}"\n',
        "line": 2,
        "col": (7, 16),
    },
    # compat-contract C-03 Verify(3): parser rejects CPython 2-style print.
    # Offending token '"x"' starts at col 7 of 'print "x"'.
    "py2_print": {
        "src": 'print "x"\n',
        "line": 1,
        "col": 7,
    },
    # spec 04 S-04-10 + corpus anchor A9: import * is SyntaxError anywhere
    # but module level. '*' is col 19 of '    from m import *'
    # (4 spaces + "from m import " = 18 chars, so '*' = 19).
    "star_import_in_function": {
        "src": "def f():\n    from m import *\n",
        "line": 2,
        "col": 19,
    },
    # spec 02 S-02-18: break outside a loop is SyntaxError, col 1.
    "break_outside_loop": {
        "src": "break\n",
        "line": 1,
        "col": 1,
    },
    # spec 02 S-02-4: nonlocal at module level is SyntaxError, col 1.
    "nonlocal_module_level": {
        "src": "nonlocal x\n",
        "line": 1,
        "col": 1,
    },
    # spec 02 S-02-10: duplicate keyword in a call literal is a compile-time
    # SyntaxError, anchored at the SECOND 'a' of 'f(a=1, a=2)' -> col 8
    # (f=1, (=2, a=3, ==4, 1=5, ,=6, ' '=7, a=8).
    "duplicate_keyword_call": {
        "src": "def f(a):\n    pass\nf(a=1, a=2)\n",
        "line": 3,
        "col": 8,
    },
}


# --------------------------------------------------------------------------
# valid programs (spec 01-04 constructs): exit 0 + OK format
# --------------------------------------------------------------------------

@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_trivial_single_statement_ok_and_count(tmp_path, ap_parse_check):
    """Bin contract + T-01 doc G3: one unambiguous statement count."""
    p = _write(tmp_path, "trivial.py", "x = 1\n")
    out = _run_ok(ap_parse_check, p)
    assert out == b"OK 1 statements"


@pytest.mark.parametrize("prog", sorted(VALID_PROGRAMS), ids=str)
@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_valid_programs_accept(tmp_path, ap_parse_check, prog):
    """Spec 01-04 grammar surfaces must parse (C-04: no false rejections)."""
    p = _write(tmp_path, f"valid_{prog}.py", VALID_PROGRAMS[prog])
    out = _run_ok(ap_parse_check, p)
    n = int(OK_RE.match(out).group(1))  # type: ignore[union-attr]
    assert n >= 1


@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_ok_format_regex(tmp_path, ap_parse_check):
    """OK line must be exactly ^OK \\d+ statements$ on stdout (one sample)."""
    p = _write(tmp_path, "okfmt.py", "y = 2\n")
    _require_bin(ap_parse_check)
    proc = ap_parse_check.run([str(p)])  # type: ignore[attr-defined]
    assert proc.returncode == 0
    assert OK_RE.match(proc.stdout)


# --------------------------------------------------------------------------
# invalid programs: exact line/col (hand-derived), exit 1, stderr only
# --------------------------------------------------------------------------

@pytest.mark.parametrize("case", sorted(INVALID_PROGRAMS), ids=str)
@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_invalid_exact_position(tmp_path, ap_parse_check, case):
    """First error at the hand-derived position; format per bin contract."""
    spec = INVALID_PROGRAMS[case]
    p = _write(tmp_path, f"invalid_{case}.py", spec["src"])
    line, col, _ = _run_err(ap_parse_check, p)
    assert line == spec["line"], (
        f"{case}: expected error at line {spec['line']}, got {line}"
    )
    expected_col = spec["col"]
    if isinstance(expected_col, tuple):
        assert expected_col[0] <= col <= expected_col[1], (
            f"{case}: expected col in {expected_col} (anchor unpinned, T-01 G1), got {col}"
        )
    else:
        assert col == expected_col, f"{case}: expected col {expected_col}, got {col}"


@pytest.mark.parametrize(
    "src",
    [INVALID_PROGRAMS[c]["src"] for c in sorted(INVALID_PROGRAMS)],
    ids=sorted(INVALID_PROGRAMS),
)
@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_error_format_regex(tmp_path, ap_parse_check, src):
    """Diagnostic format: ^error at line \\d+, column \\d+: .+$ on stderr."""
    p = _write(tmp_path, "fmt.py", src)
    _require_bin(ap_parse_check)
    proc = ap_parse_check.run([str(p)])  # type: ignore[attr-defined]
    assert proc.returncode == 1
    assert DIAG_RE.match(proc.stderr.splitlines()[0])


# --------------------------------------------------------------------------
# exit codes
# --------------------------------------------------------------------------

@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_exit_code_0_valid(tmp_path, ap_parse_check):
    """C-30: exit 0 reserved for clean runs; a valid parse is clean."""
    p = _write(tmp_path, "exit0.py", "print(1 + 1)\n")
    _require_bin(ap_parse_check)
    assert ap_parse_check.run([str(p)]).returncode == 0  # type: ignore[attr-defined]


@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_exit_code_1_invalid(tmp_path, ap_parse_check):
    """C-29 bullet 4: compile-time SyntaxError -> nonzero (pinned to 1 here)."""
    p = _write(tmp_path, "exit1.py", "def f(:\n")
    _require_bin(ap_parse_check)
    assert ap_parse_check.run([str(p)]).returncode == 1  # type: ignore[attr-defined]


@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_exit_code_2_missing_file(tmp_path, ap_parse_check):
    """Bin contract: missing input file -> exit 2 (T-01 doc G5)."""
    _require_bin(ap_parse_check)
    missing = tmp_path / "no_such_file.py"
    assert ap_parse_check.run([str(missing)]).returncode == 2  # type: ignore[attr-defined]


# --------------------------------------------------------------------------
# fail-fast: exactly ONE diagnostic (first error), per error-recovery-off
# --------------------------------------------------------------------------

@pytest.mark.xfail(strict=False, reason=XFAIL_BIN)
def test_first_error_only_fail_fast(tmp_path, ap_parse_check):
    """Errors at lines 2 AND 3; only line 2's may be reported (recovery off)."""
    src = "a = 1\n]\n)\nb = 2\n"  # stray ']' line 2 col 1, stray ')' line 3 col 1
    p = _write(tmp_path, "multi_error.py", src)
    line, _col, stderr = _run_err(ap_parse_check, p)
    n_lines = [ln for ln in stderr.splitlines() if DIAG_RE.match(ln)]
    assert len(n_lines) == 1, (
        f"fail-fast contract: expected exactly 1 diagnostic, got {len(n_lines)}: {stderr!r}"
    )
    assert line == 2, f"first error is at line 2 (stray ']'), got line {line}"


# --------------------------------------------------------------------------
# unicode: contract gap G2 - column units unpinned, xfail until a contract PR
# --------------------------------------------------------------------------

@pytest.mark.xfail(strict=False, reason=XFAIL_UNICODE)
def test_unicode_column_characters(tmp_path, ap_parse_check):
    """Columns in CHARACTERS (CPython convention), pending contract gap G2.

    Source (UTF-8):
        line 1: "é = 1"    (é is 2 bytes, irrelevant to line 2)
        line 2: "漢 = (2 +"  the unclosed '(' is the 5th CHARACTER
              (漢=1, ' '=2, '='=3, ' '=4, '('=5). In BYTES the '(' starts at
              byte offset 8 of the line (漢 is 3 UTF-8 bytes) -> col 8.
    """
    src = "é = 1\n漢 = (2 +\n3\n"
    p = _write(tmp_path, "unicode_pos.py", src)
    line, col, _ = _run_err(ap_parse_check, p)
    assert line == 2
    assert col == 5, (
        f"expected column 5 (1-based, CHARACTERS) for the unclosed '(' on line 2, got {col}; "
        f"if the bin reports bytes (col 8) the contract gap T-01 G2 must be resolved by PR"
    )

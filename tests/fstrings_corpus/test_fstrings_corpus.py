# T-11-style corpus slice: f-string forms (differential bug hunt leaf).

# Cites: S-01-16 (f-string lowering), C-18 (f-string rendering), C-33
# (determinism/masking), docs/pon-study-lowering.md "f-strings / t-strings"
# (parts-not-elements gotcha, debug-text `{x=}` forms).
# Slice: f-string language forms (T-xx, from the f-string differential hunt).
# Reference bytes pinned by /opt/homebrew/bin/python3.14 (3.14.7;
# provisional vs the C-01 pin 3.14.0, per the T-05/T-06/T-11 provenance-gap
# precedent). Expected triples use the tests/corpus/stdlib layout:
# expected/<case>.out / .err / .rc.
#
# Known intentional error cases (also part of the contract):
# - f33_bytes_adjacent_error: `f"..." b"..."` is a SyntaxError (C-18/S-01-16);
#   the parse-refusal diagnostic contract is what the .err/.rc pin.

import ast
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


def _mask_root(data: bytes) -> bytes:
    """Normalize absolute corpus paths in stderr self-check pins.

    CPython 3.14 resolves the script path to an ABSOLUTE path in tracebacks
    and warning locations, so pins recorded the original checkout's absolute
    path and failed in every other worktree/CI. The mask collapses any
    absolute prefix that precedes a tests/corpus/ path to <ROOT>/... .
    Line numbers, caret columns, and message text are untouched, so the pin
    still diffs everything semantic (C-33: mask, never weaken). Follows the
    in-tree precedent tests/t09_binary (_norm_stderr, S-06-7 caveat). The
    differential gate (ap-conform, reference vs implementation) stays
    byte-exact with NO masking; this applies only to reference self-checks.
    """
    return re.sub(rb"/[^\s\"',;()]*?(tests/corpus/[^\s\"',;()]*)",
                  rb"<ROOT>/\1", data)
CORPUS_DIR = REPO_ROOT / "tests" / "corpus" / "fstrings"
CASES_DIR = CORPUS_DIR / "cases"
EXPECTED_DIR = CORPUS_DIR / "expected"
# Reference discovery: unified env+PATH ladder (robustness-ci 01.01). The old
# local helper allowed a bare `python3` fallback — tests/README rule 5 says NO
# (the version-string check is mandatory). _ref_python.reference_or_skip()
# enforces the check + the AP_REQUIRE_REFERENCE=1 skip->fail policy.
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
from _ref_python import reference_or_skip  # noqa: E402

ALL_CASES = sorted(CASES_DIR.glob("*.py"))
CASE_IDS = [c.stem for c in ALL_CASES]


def _ref_python():
    return reference_or_skip()


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_expected_triple_exists(case):
    stem = case.stem
    for suffix in (".out", ".err", ".rc"):
        assert (EXPECTED_DIR / (stem + suffix)).is_file(), (
            f"missing expected/{stem}{suffix}")


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_expected_bytes_stable(case):
    # Re-run the reference and byte-compare stdout/rc against the pinned
    # triple (S-06-1 no-normalization). A failure here means nondeterminism —
    # fix by masking, never weaken (C-33).
    #
    # Pinned env (S-06-15 + C-27): PATH/HOME only + LANG=C, so the locale is
    # checkout-independent. LC_* is excluded (CPython rewrites LC_CTYPE in
    # its own env block, leaking it to children). Case path is RELATIVE to
    # REPO_ROOT so any traceback embeds a stable path, not the checkout dir.
    stem = case.stem
    exp_out = (EXPECTED_DIR / (stem + ".out")).read_bytes()
    exp_rc = int((EXPECTED_DIR / (stem + ".rc")).read_text())
    env = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    env["LANG"] = "C"
    proc = subprocess.run(
        [_ref_python(), "-I", os.path.relpath(case, REPO_ROOT)],
        capture_output=True, timeout=30, cwd=str(REPO_ROOT), env=env)
    assert proc.stdout == exp_out, stem
    assert _mask_root(proc.stderr) == _mask_root(
        (EXPECTED_DIR / (stem + ".err")).read_bytes()), f"{stem}: stderr drift"
    assert proc.returncode == exp_rc, stem


@pytest.mark.parametrize("case", ALL_CASES, ids=CASE_IDS)
def test_cases_are_deterministic(case):
    # No salted/random/time-dependent anchors in an f-string slice (C-33);
    # a cheap static lint now, extensible later.
    src = case.read_text(encoding="utf-8")
    for bad in ("time.", "random", "os.urandom", "hash(", "id("):
        assert bad not in src, f"{case.stem}: prohibited anchor {bad!r}"


def test_debug_forms_covered():
    # The pon-study-flagged area: `{x=}` debug suffix must be exercised in
    # both spellings plus spec/conv combinations (docs/pon-study-lowering.md
    # f-string section).
    blob = "\n".join(p.read_text(encoding="utf-8") for p in ALL_CASES)
    assert re.search(r"\{x=\}", blob), "plain debug form missing"
    assert re.search(r"\{x = \}", blob), "spaced debug form missing"
    assert re.search(r"\{x=:>", blob), "debug+spec form missing"
    assert re.search(r"\{x=!", blob), "debug+conv form missing"


def test_parts_not_elements_mix_covered():
    # The pon-flagged ruff gotcha: plain string literals implicitly
    # concatenated BETWEEN f-string parts must survive lowering.
    blob = "\n".join(p.read_text(encoding="utf-8") for p in ALL_CASES)
    assert '"#define X {"' in blob and '"%s"' in blob, (
        "mixed implicit-concat literal/fstring case missing")

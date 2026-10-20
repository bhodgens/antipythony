"""XPASS ceiling check as a pytest test (robustness-ci leaf 01.01).

This test does NOT re-run the suite (the full differential track is ~1h —
far too slow for a lint). Instead it gates a CACHED report from a real
``pytest tests/ -q`` run, exactly like tests/tools/_xpass_gate.py does for
CI. Set ``XPASS_REPORT`` to the report file:

    pytest tests/ -q --tb=no > /tmp/report.txt
    XPASS_REPORT=/tmp/report.txt pytest tests/corpus_traceability/test_xpass_ceiling.py -q

Without XPASS_REPORT (or when the file doesn't exist) the test skips with a
message — the committed ceiling itself is still linted below (it must be a
non-negative integer), so a corrupt/missing ceiling file fails fast even
with no report cached.
"""

import os
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))

from _xpass_gate import CEILING_FILE, count_xpass, read_ceiling  # noqa: E402


def test_ceiling_file_is_a_valid_integer():
    ceiling = read_ceiling()
    assert ceiling >= 0, f"ceiling must be non-negative, got {ceiling}"
    # The ceiling is the MEASURED xpass count of a real full-suite run
    # (provenance lives as # comment lines in tests/XPASS_CEILING). A
    # measured 0 is legitimate once the stale-xfail burn-down (01.02)
    # lands — then ANY new xpass breaches the gate.
    raw_lines = CEILING_FILE.read_text().splitlines()
    assert raw_lines and raw_lines[0].strip().isdigit(), (
        "tests/XPASS_CEILING line 1 must be the bare measured integer")
    assert any(ln.lstrip().startswith("#") and "measured" in ln
               for ln in raw_lines), (
        "tests/XPASS_CEILING must carry a '# measured ...' provenance "
        "comment (date + commit sha) per leaf 01.01")


def test_xpass_count_within_ceiling():
    report = os.environ.get("XPASS_REPORT", "").strip()
    if not report:
        pytest.skip(
            "no cached pytest report: set XPASS_REPORT=/path/to/'pytest "
            "tests/ -q' output (the ceiling gate does not re-run the "
            "suite; CI wires tests/tools/_xpass_gate.py post-suite)")
    path = Path(report)
    if not path.is_file():
        pytest.skip(f"XPASS_REPORT={report} does not exist")
    ceiling = read_ceiling()
    n = count_xpass(path.read_text(errors="replace"))
    assert n <= ceiling, (
        f"XPASS CEILING BREACH: report {path} shows {n} xpassed > ceiling "
        f"{ceiling} (tests/XPASS_CEILING). Burn stale xfail markers per "
        f"STALE-XFAIL-LEDGER.md; never raise the ceiling to hide them.")

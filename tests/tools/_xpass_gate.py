#!/usr/bin/env python3
"""XPASS ceiling gate (robustness-ci leaf 01.01, master.md "XPASS ceiling").

Compares the xpass count of a completed `pytest tests/ -q` run against the
committed ceiling in ``tests/XPASS_CEILING``. CI fails when the count
EXCEEDS the ceiling (monotone-decreasing policy: lowering stale-xfail
markers is 01.02's burn-down; this gate makes regressions mechanical).

Usage:
    python tests/tools/_xpass_gate.py <report-file>
    cat <report> | python tests/tools/_xpass_gate.py -

The report is pytest's -q short output (with the final summary line, e.g.
``1240 passed, 280 xfailed, 3 skipped, 5 xpassed in 312.4s``), optionally
with -r/-rf extras. The gate accepts:
  - the summary line "N xpassed" token, and
  - per-test "XPASS" report lines (from -rX / -rx), taking the MAX of the
    two signals (guards against partial reports).

Exit codes: 0 = at/below ceiling (or xpass signal absent => 0 xpasses),
1 = above ceiling, 2 = input/environment error (missing ceiling file,
unparseable ceiling, no summary line in a truncated report).

CI wiring (orchestrator step, NOT wired by this leaf): the conformance job
pipes its full-suite report to a file and appends
``python tests/tools/_xpass_gate.py <report>`` after pytest, so the gate
never re-runs the suite.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

CEILING_FILE = Path(__file__).resolve().parents[1] / "XPASS_CEILING"

# "5 xpassed" inside the -q summary line (pytest may pluralize/skip the
# token entirely when the count is zero).
_SUMMARY_XPASSED = re.compile(r"(\d+)\s+xpassed")
# Per-test XPASS report lines emitted by -rX/-rx/-ra extras.
_REPORT_XPASS_LINE = re.compile(r"^(?:XPASS|XPASS)[\s(]", re.M)


def read_ceiling(path: Path = CEILING_FILE) -> int:
    raw = path.read_text().strip()
    # allow trailing comment lines starting with '#'
    first = [ln for ln in raw.splitlines() if ln.strip() and not ln.lstrip().startswith("#")]
    if not first:
        print(f"_xpass_gate: {path} is empty", file=sys.stderr)
        raise SystemExit(2)
    try:
        return int(first[0].strip())
    except ValueError:
        print(f"_xpass_gate: {path} does not contain an integer "
              f"(found {first[0]!r})", file=sys.stderr)
        raise SystemExit(2)


def count_xpass(report_text: str) -> int:
    """Extract the xpass count from a pytest -q report.

    Returns the max of the summary-token count and the per-test XPASS line
    count. Raises SystemExit(2) when the report has no final summary line
    (i.e. the run was truncated/killed — refusing to infer 0 xpasses from a
    half report keeps the gate honest).
    """
    # pytest -q ends with a summary line: contains " in <duration>" plus
    # outcome words, e.g. "1240 passed, 5 xpassed in 3123.45s (0:52:03)".
    summary = None
    for line in report_text.splitlines():
        s = line.strip()
        if " in " in s and re.search(
                r"\b(passed|failed|error|xfailed|xpassed|skipped)\b", s) \
                and not s.startswith((".", "=", "F", "E", "x", "s")):
            summary = s
    signals = []
    if summary is not None:
        m = _SUMMARY_XPASSED.search(summary)
        if m:
            signals.append(int(m.group(1)))
        else:
            signals.append(0)  # summary present, xpass token absent => 0
    signals.append(len(_REPORT_XPASS_LINE.findall(report_text)))
    if summary is None and not _REPORT_XPASS_LINE.search(report_text):
        print("_xpass_gate: no pytest summary line found — report looks "
              "truncated or not -q output; refusing to gate", file=sys.stderr)
        raise SystemExit(2)
    return max(signals)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("report", help="pytest -q report file, or '-' for stdin")
    ap.add_argument("--ceiling", default=str(CEILING_FILE),
                    help="ceiling file (default: tests/XPASS_CEILING)")
    args = ap.parse_args(argv)

    ceiling = read_ceiling(Path(args.ceiling))
    if args.report == "-":
        text = sys.stdin.read()
    else:
        p = Path(args.report)
        if not p.is_file():
            print(f"_xpass_gate: report file {p} not found", file=sys.stderr)
            return 2
        text = p.read_text(errors="replace")

    n = count_xpass(text)
    if n > ceiling:
        print(f"XPASS CEILING BREACH: {n} xpassed > ceiling {ceiling} "
              f"(tests/XPASS_CEILING). Stale xfail markers went XPASS above "
              f"the recorded baseline — un-xfail them per "
              f"plans/antipythany/02-native-backend/STALE-XFAIL-LEDGER.md, "
              f"then lower the ceiling only as part of that flip.")
        return 1
    print(f"xpass gate OK: {n} xpassed <= ceiling {ceiling}"
          + (" (room to lower the ceiling: run the 01.02 ledger flip)"
             if n < ceiling else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())

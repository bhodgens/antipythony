#!/usr/bin/env python3
"""Generate CPython-pinned expectation fixtures for the T-04 verify suite.

Run EXACTLY this from the repo root (the pinned reference implementation):

    /opt/homebrew/bin/python3.14 tests/t04_ir_verify/gen_expectations.py

It executes every probe in tests/t04_ir_verify/probes/ as a FILE under the
reference interpreter (mirrors the ap-conform convention: `python3.14 <path>`,
spec 02 "Corpus anchors" preamble), captures rc/stdout/stderr verbatim,
normalizes the absolute probe path to "<PROBE>" (the traceback `File "..."`
text is the invocation path — spec 02 preamble: both implementations receive
the identical argv, so tests compare after this normalization), and derives
from the observed error report:

  - error_class   : the exception name on the report's last line
  - message       : everything after the first ": " on that last line
  - expected_line : innermost frame line (runtime) / File line (SyntaxError)
  - expected_col  : column of the first '^' on the FGE marker line (1-based),
                    null when CPython emitted no marker line (S-02-37: raise /
                    simple statements have none)
  - kind          : "runtime-traceback" | "compile-syntaxerror" | "clean"

Output: tests/t04_ir_verify/fixtures/expected_t04.json (machine) and
        expected_t04.txt (human-readable, same data).
Written by the T-04 test author; the values are the expect side of the
differential generator described in plans/antipythony/tests/T-04-ir-verify.md.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SUITE = Path(__file__).resolve().parent
PROBES = SUITE / "probes"
FIXTURES = SUITE / "fixtures"


def normalize(text: str, path: Path) -> str:
    return text.replace(str(path), "<PROBE>")


def derive(path: Path, rc: int, stdout: str, stderr: str) -> dict:
    """Classify the observed report and pull the pinned assertion fields out."""
    rec = {"rc": rc, "stdout": stdout, "stderr": stderr}
    if rc == 0 and not stderr:
        rec.update(kind="clean", error_class=None, message=None,
                   expected_line=None, expected_col=None)
        return rec
    if rc == 0:
        # SyntaxWarning-only run (e.g. S-01-31 `is`-with-literal warning):
        # completed normally, stderr has a warning report, NOT an error.
        rec.update(kind="clean-with-warning", error_class=None, message=None,
                   expected_line=None, expected_col=None)
        return rec
    lines = stderr.rstrip("\n").splitlines()
    last = lines[-1] if lines else ""
    if stderr.startswith("Traceback (most recent call last):"):
        kind = "runtime-traceback"
    elif "SyntaxError" in last:
        kind = "compile-syntaxerror"
    else:
        kind = "other"
    exc_name, _, exc_msg = last.partition(":")
    # innermost frame: LAST `  File "` block in a traceback; the only one in a
    # compile-time SyntaxError report.
    file_idxs = [i for i, ln in enumerate(lines) if ln.startswith('  File "')]
    fi = file_idxs[-1]
    import re

    m = re.search(r'line (\d+)', lines[fi])
    expected_line = int(m.group(1)) if m else None
    expected_col = None
    # marker line: the line after the indented source line, made of ~^ chars
    # (runtime FGE, S-02-37) or the caret line of the SyntaxError report.
    for cand in (lines[fi + 2] if fi + 2 < len(lines) else "",
                 lines[fi + 2] if fi + 2 < len(lines) else ""):
        if cand and set(cand.strip()) <= {"~", "^"} and "^" in cand:
            expected_col = cand.index("^") + 1
            break
    rec.update(
        kind=kind,
        error_class=exc_name,
        message=exc_msg.strip() if exc_msg else "",
        expected_line=expected_line,
        expected_col=expected_col,
    )
    return rec


def main() -> int:
    if sys.version_info[:2] != (3, 14):
        sys.exit("this script MUST run under the reference python3.14")
    FIXTURES.mkdir(parents=True, exist_ok=True)
    out = {
        "generator": "tests/t04_ir_verify/gen_expectations.py",
        "python_executable": sys.executable,
        "python_version": sys.version,
        "path_normalization": "absolute probe path replaced with <PROBE>",
        "probes": {},
    }
    for probe in sorted(PROBES.glob("*.py")):
        proc = subprocess.run(
            [sys.executable, str(probe)],
            capture_output=True,
            text=True,
            timeout=60,
        )
        rec = derive(
            probe,
            proc.returncode,
            normalize(proc.stdout, probe),
            normalize(proc.stderr, probe),
        )
        out["probes"][probe.name] = rec

    (FIXTURES / "expected_t04.json").write_text(
        json.dumps(out, indent=2, sort_keys=True) + "\n"
    )
    txt_lines = [
        "# T-04 CPython-pinned expectations (generated, do not hand-edit)",
        f"# reference: {out['python_version'].splitlines()[0]}  {out['python_executable']}",
        "# regenerate: /opt/homebrew/bin/python3.14 tests/t04_ir_verify/gen_expectations.py",
        "",
    ]
    for name, rec in out["probes"].items():
        txt_lines.append(
            f"{name}\tkind={rec['kind']}\trc={rec['rc']}\t"
            f"class={rec['error_class']}\tline={rec['expected_line']}\t"
            f"col={rec['expected_col']}\tmsg={rec['message']!r}"
        )
    (FIXTURES / "expected_t04.txt").write_text("\n".join(txt_lines) + "\n")
    print(f"wrote {FIXTURES / 'expected_t04.json'} ({len(out['probes'])} probes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

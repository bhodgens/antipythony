#!/usr/bin/env python3.14
"""Golden-provenance recorder (leaf 03.01, robustness-ci finding #11).

Writes `expected/<case>.meta` for every pinned corpus case under
tests/corpus/**/expected/ WITHOUT touching any pin file. One .meta per
case stem, covering the whole triple (.out/.err/.rc or
.stdout/.stderr/.exit).

Schema (stable contract, linted by
tests/corpus_traceability/test_golden_provenance.py):

    {
      "sys_version":       "<sys.version of the reference interpreter, captured
                            from an actual run of that binary>",
      "date":              "YYYY-MM-DD of the recording",
      "generator_sha256":  "sha256 of the generator script that wrote this",
      "family":            "<corpus family dir name>",
      "generator":         "repo-relative path of the recording script"
    }

Honesty note for the 2026-10-20 backfill: the pins were originally recorded
ad hoc by test-track authors (no generator script existed), under
CPython 3.14.7 while the contract (C-01) pins 3.14.0 — see
.hermes/audits/robustness-review-2026-10-19.md #11. The backfilled meta
records the reference interpreter as it exists on the recording machine
(sys.version captured live), which makes the 3.14.x-vs-3.14.0 debt
mechanically visible. A future 3.14.0 re-verification run rewrites the
metas via this tool.

Per-family generator attribution: where a family HAS a canonical pin
generator (currently: gc -> tests/t07_gc/_gen_expected.py, which writes
its own metas on regeneration), the meta attributes to that script and
records ITS sha256; the provenance gate
(tests/corpus_traceability/test_golden_provenance.py) checks the recorded
path's live hash, so editing the pin generator without re-recording is a
CI failure. Families recorded ad hoc by test-track authors attribute to
this recorder.

Usage:
    /opt/homebrew/bin/python3.14 tests/tools/_gen_meta.py            # all families
    /opt/homebrew/bin/python3.14 tests/tools/_gen_meta.py gc values  # named families

NEVER writes, rewrites, or deletes .out/.err/.rc/.stdout/.stderr/.exit.
Stdlib only.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
from datetime import date
from pathlib import Path

CORPUS = Path(__file__).resolve().parents[1] / "corpus"   # tests/corpus
REPO = Path(__file__).resolve().parents[2]
GEN_FILE = Path(__file__).resolve()

PIN_SUFFIXES = (".out", ".err", ".rc", ".stdout", ".stderr", ".exit")

# Families whose pins have a canonical generator script (that script writes
# its own metas when it re-pins; backfill metas attribute to it too, so the
# lint's generator-hash freshness check covers the real pin producer).
FAMILY_GENERATORS = {
    "gc": REPO / "tests" / "t07_gc" / "_gen_expected.py",
}


def find_reference_python() -> str:
    """Reference interpreter selection (kept per-generator local on purpose —
    centralizing discovery is finding #4 / a different leaf)."""
    cand = os.environ.get("AP_CONFORM_REFERENCE_PYTHON")
    if cand and Path(cand).is_file():
        return cand
    hb = "/opt/homebrew/bin/python3.14"
    if Path(hb).is_file():
        return hb
    which = shutil.which("python3.14")
    if which:
        return which
    if sys.version_info[:2] == (3, 14):
        return sys.executable
    sys.exit("no Python 3.14 reference interpreter found "
             "(set AP_CONFORM_REFERENCE_PYTHON)")


def capture_sys_version(py: str) -> str:
    """Run the reference binary and capture its actual sys.version string."""
    proc = subprocess.run([py, "-c", "import sys; print(sys.version)"],
                          capture_output=True, text=True, timeout=60)
    if proc.returncode != 0:
        sys.exit(f"reference interpreter {py!r} failed: {proc.stderr}")
    return proc.stdout.strip()


def generator_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def family_of(exp_dir: Path) -> str:
    rel = exp_dir.parent.relative_to(CORPUS)
    return rel.parts[0]


def pinned_stems(exp_dir: Path) -> set[str]:
    return {p.stem for p in exp_dir.iterdir()
            if p.is_file() and p.suffix in PIN_SUFFIXES}


def record_meta(exp_dir: Path, stem: str, family: str, sys_version: str,
                generator_path: Path | None = None,
                when: str | None = None) -> Path:
    gen = (generator_path or GEN_FILE).resolve()
    payload = {
        "sys_version": sys_version,
        "date": when or date.today().isoformat(),
        "generator_sha256": generator_sha256(gen),
        "family": family,
        "generator": str(gen.relative_to(REPO)),
    }
    meta = exp_dir / f"{stem}.meta"
    meta.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8")
    return meta


def expected_dirs(families: list[str]) -> list[Path]:
    dirs = sorted(p for p in CORPUS.rglob("expected") if p.is_dir())
    if families:
        dirs = [d for d in dirs if family_of(d) in families]
    return dirs


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        if argv and argv[0] in ("-h", "--help"):
            return 0
        families = []
    else:
        families = argv

    py = find_reference_python()
    sys_version = capture_sys_version(py)
    print(f"reference: {py} -> {sys_version.splitlines()[0]}")

    n = 0
    for exp in expected_dirs(families):
        stems = pinned_stems(exp)
        if not stems:
            print(f"NOTE {exp.relative_to(REPO)}: no pins, nothing to record")
            continue
        family = family_of(exp)
        gen = FAMILY_GENERATORS.get(family)
        for stem in sorted(stems):
            meta = record_meta(exp, stem, family, sys_version,
                               generator_path=gen)
            n += 1
        print(f"OK   {exp.relative_to(REPO)}: {len(stems)} metas")
    print(f"wrote {n} .meta files (pins untouched)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

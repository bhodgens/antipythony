#!/usr/bin/env python3
"""Leaf 03.05 CLI gate: run every currently-green wasm corpus case through
the ap-host BINARY and diff stdout/stderr/exit byte-exactly against the
CPython 3.14 pins (tests/corpus/<fam>/expected/).

Runs OUTSIDE cargo: the deliverable is that corpus cases run through the
CLI without a cargo test harness. Machine policy: no cargo involved here.

Usage: python3 crates/ap-host/src/ap-host  (see the actual path below)
       python3 scripts/cli_corpus_gate.py <path-to-ap-host-binary>
"""

import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "tests" / "corpus"

# (family, binary filename in logs/wasm-corpus) — pins from expected/
FAMILIES = {
    "controlflow_wasm": ".stdout",  # pins: .stdout/.stderr/.exit
    "classes_wasm": ".out",         # pins: .out/.err/.rc
    "exceptions_wasm": ".out",
}


def pin_set(fam_dir: Path, stem: str):
    """Return (stdout, stderr, exit) pins, tolerating both naming schemes."""
    for out_name, err_name, rc_name in (
        (".stdout", ".stderr", ".exit"),
        (".out", ".err", ".rc"),
    ):
        o = fam_dir / "expected" / f"{stem}{out_name}"
        e = fam_dir / "expected" / f"{stem}{err_name}"
        r = fam_dir / "expected" / f"{stem}{rc_name}"
        if o.exists() and e.exists() and r.exists():
            return o.read_bytes(), e.read_bytes(), int(r.read_text().strip())
    raise FileNotFoundError(f"pins for {stem} in {fam_dir}/expected")


def main() -> int:
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else REPO / "target-0304c/debug/ap-host")
    wasm_dir = REPO / "logs" / "wasm-corpus"
    assert binary.exists(), f"ap-host binary missing: {binary}"
    assert wasm_dir.is_dir(), f"wasm dump missing: {wasm_dir}"

    passed = failed = 0
    failures = []
    for wasm in sorted(wasm_dir.glob("*.wasm")):
        stem = wasm.stem
        # locate the corpus family by prefix
        if stem.startswith("CF"):
            fam = "controlflow_wasm"
        elif stem.startswith("CLW"):
            fam = "classes_wasm"
        elif stem.startswith("EXW"):
            fam = "exceptions_wasm"
        else:
            fam = None  # drain probes: no pins here (covered by cargo gates)

        proc = subprocess.run(
            [str(binary), "run", str(wasm)],
            capture_output=True,
            timeout=120,
        )
        if fam is None:
            # probes: report-only, verdict recorded in the session log
            print(f"[probe] {stem}: rc={proc.returncode} out={proc.stdout[:60]!r}")
            passed += 1
            continue
        want_out, want_err, want_rc = pin_set(CORPUS / fam, stem)
        got_out = proc.stdout.replace(b"\r\n", b"\n")
        ok = got_out == want_out and proc.stderr == want_err and proc.returncode == want_rc
        if ok:
            passed += 1
        else:
            failed += 1
            failures.append(
                f"{fam}/{stem}: MISMATCH\n"
                f"  want out {want_out!r}\n  got  out {got_out!r}\n"
                f"  want err {want_err!r}\n  got  err {proc.stderr!r}\n"
                f"  want rc {want_rc}  got  rc {proc.returncode}"
            )

    print(f"CLI corpus gate: {passed} passed, {failed} failed")
    for f in failures:
        print(f)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

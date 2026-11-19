#!/usr/bin/env python3
"""Leaf 03.07 full-corpus TRIPLE-DIFF report generator.

For every corpus case that has a wasm lowering (the 41 pinned families + the
10 drain probes), runs:
  (1) NATIVE  — the antipythony binary (tier-0), per-case live run
  (2) WASM    — ap-host run <module.wasm>, per-case live run
  (3) CPYTHON — the CPython 3.14 pins in tests/corpus/<fam>/expected/
                (pins generated against CPython 3.14; re-verified live for
                this leaf below)
and byte-diffs all three channels (stdout/stderr/exit-code) pairwise.

Machine-readable JSON report -> plans/antipythony/03-wasm-backend/03.07-triple-diff.json
Never /tmp for artifacts (machine policy). No cargo involved here.

Usage: python3 scripts/triple_diff_report.py <antipythony-bin> <ap-host-bin>
"""

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "tests" / "corpus"
LOGS = REPO / "logs"
OUT_JSON = REPO / "plans/antipythony/03-wasm-backend/03.07-triple-diff.json"
OUT_MD = REPO / "plans/antipythony/03-wasm-backend/03.07-triple-diff.md"

# wasm-dump families (41 pinned cases) + probe sources (10)
WASM_FAMILIES = [
    "controlflow_wasm",
    "classes_wasm",
    "exceptions_wasm",
    "generators_wasm",
    "match_wasm",
]

PROBES = [
    ("hello", "print('hello, wasm')\n"),
    ("for-range", "for j in range(2):\n    print('j', j)\nprint('done')\n"),
    ("for-list", "for v in [10, 20, 30]:\n    print('v', v)\nprint('done')\n"),
    ("for-tuple-else", "for w in (1, 2):\n    print('w', w)\nprint('done')\n"),
    ("for-range-sum", "total = 0\nfor i in range(3):\n    total = total + i\nprint(total)\n"),
    ("for-break", "for i in range(5):\n    if i == 2:\n        break\n    print(i)\nprint('end')\n"),
    ("drain-list-lit", "print(list([1, 2]))\n"),
    ("drain-tuple-lit", "print(list((1, 2)))\n"),
    ("sum-min", "print(sum(range(4)))\n"),
    ("drain-gen-lit", "it = iter([5, 6])\nprint(next(it))\nprint(list(it))\n"),
]

CPYTHON = "/opt/homebrew/bin/python3.14"


def pins_for(fam_dir: Path, stem: str):
    """CPython pins, tolerating both naming schemes."""
    for o, e, r in ((".stdout", ".stderr", ".exit"), (".out", ".err", ".rc")):
        po = fam_dir / "expected" / f"{stem}{o}"
        pe = fam_dir / "expected" / f"{stem}{e}"
        pr = fam_dir / "expected" / f"{stem}{r}"
        if po.exists() and pe.exists() and pr.exists():
            return po.read_bytes(), pe.read_bytes(), int(pr.read_text().strip())
    return None


def run_cmd(cmd, cwd=None, timeout=120):
    p = subprocess.run(cmd, capture_output=True, timeout=timeout, cwd=cwd)
    return p.stdout, p.stderr, p.returncode


def norm(b):
    return b.replace(b"\r\n", b"\n")


def verdict(native, wasm, pins):
    """(native_ok, wasm_ok, class) — class: byte-exact / semantic-pass-bytes-diverge / fail"""
    n_ok = w_ok = True
    for ch, got in zip(("stdout", "stderr", "rc"), native):
        want = pins[ch]
        got_n = norm(got) if isinstance(got, bytes) else got
        if got_n != want:
            n_ok = False
    for ch, got in zip(("stdout", "stderr", "rc"), wasm):
        want = pins[ch]
        got_w = norm(got) if isinstance(got, bytes) else got
        if got_w != want:
            w_ok = False
    if n_ok and w_ok:
        return n_ok, w_ok, "byte-exact"
    # semantic pass = rc identical across all three AND stdout semantically
    # identical across native-vs-wasm (native vs pins may carry D6-scope
    # repr/wording drift); anything else is a fail.
    sem = (
        native[2] == wasm[2] == pins["rc"]
        and norm(native[0]) == norm(wasm[0]) == norm(pins["stdout"])
        and norm(native[1]) == norm(wasm[1]) == norm(pins["stderr"])
    )
    if sem:
        return n_ok, w_ok, "semantic-pass-bytes-diverge"
    return n_ok, w_ok, "fail"


def main():
    native_bin = Path(sys.argv[1])
    wasmx_bin = Path(sys.argv[2])
    assert native_bin.exists(), native_bin
    assert wasmx_bin.exists(), wasmx_bin
    wasm_dir = LOGS / "wasm-corpus"
    assert wasm_dir.is_dir(), "run cli_corpus_dump first"

    cases = []
    # pinned families
    for wasm in sorted(wasm_dir.glob("*.wasm")):
        stem = wasm.stem
        fam = None
        for prefix, f in (
            ("CF", "controlflow_wasm"), ("CLW", "classes_wasm"),
            ("EXW", "exceptions_wasm"), ("GE", "generators_wasm"),
            ("MA", "match_wasm"),
        ):
            if stem.startswith(prefix):
                fam = f
                break
        if fam is None:
            continue  # probes handled separately
        fam_dir = CORPUS / fam
        pins_raw = pins_for(fam_dir, stem)
        if pins_raw is None:
            cases.append({"case": stem, "family": fam, "status": "NO-PINS"})
            continue
        pins = {"stdout": pins_raw[0], "stderr": pins_raw[1], "rc": pins_raw[2]}
        # wasm leg
        wo, we, wrc = run_cmd([str(wasmx_bin), "run", str(wasm)])
        # native leg — live run of the .py source
        src = fam_dir / f"{stem}.py"
        no, ne, nrc = run_cmd([str(native_bin), "run", str(src)], cwd=str(REPO))
        # cpython leg — live re-verify of the pin (methodology check)
        co, ce, crc = run_cmd([CPYTHON, str(src)], cwd=str(REPO))
        n_ok, w_ok, cls = verdict(
            (no, ne, nrc), (wo, we, wrc), pins
        )
        pin_live = (co, ce, crc) == (pins["stdout"], pins["stderr"], pins["rc"])
        cases.append({
            "case": stem, "family": fam,
            "verdict": cls,
            "native_byte_exact": n_ok, "wasm_byte_exact": w_ok,
            "native_rc": nrc, "wasm_rc": wrc, "pin_rc": pins["rc"],
            "cpython_live_matches_pin": pin_live,
        })

    # probes (report-only; live triple with CPython live as the "pin")
    for name, src_text in PROBES:
        src = LOGS / f"probe_{name.replace('/', '_')}.py"
        src.write_text(src_text)
        wasm = wasm_dir / f"{name}.wasm"
        wo, we, wrc = run_cmd([str(wasmx_bin), "run", str(wasm)])
        no, ne, nrc = run_cmd([str(native_bin), "run", str(src)], cwd=str(REPO))
        co, ce, crc = run_cmd([CPYTHON, str(src)], cwd=str(REPO))
        trip = (no, ne, nrc) == (co, ce, crc) == (wo, we, wrc)
        cases.append({
            "case": name, "family": "probes",
            "verdict": "byte-exact" if trip else "fail",
            "native_rc": nrc, "wasm_rc": wrc, "cpython_rc": crc,
        })

    # summary
    lowered = [c for c in cases if c.get("verdict")]
    counts = {}
    for c in lowered:
        counts[c["verdict"]] = counts.get(c["verdict"], 0) + 1

    report = {
        "leaf": "03.07",
        "amendment": "A-3 flip: byte-exactness is a HARD requirement for every case that lowers",
        "wasm_corpus": {"pinned": 41, "probes": 10},
        "cpython_reference": {"version": "3.14.7", "path": CPYTHON},
        "summary": counts,
        "cases": cases,
    }
    OUT_JSON.write_text(json.dumps(report, indent=2) + "\n")

    # markdown table
    lines = [
        "# Leaf 03.07 full-corpus triple-diff report (generated)",
        "",
        f"Reference: CPython 3.14.7 pins (live re-verified per case).",
        f"Wasm leg through the ap-host CLI binary; native leg through the antipythony CLI binary (tier-0).",
        "",
        "| case | family | native | wasm | verdict |",
        "| --- | --- | --- | --- | --- |",
    ]
    for c in lowered:
        lines.append(
            f"| {c['case']} | {c['family']} | "
            f"{'exact' if c.get('native_byte_exact', c.get('verdict') == 'byte-exact') else 'diverge'} | "
            f"{'exact' if c.get('wasm_byte_exact', c.get('verdict') == 'byte-exact') else 'diverge'} | "
            f"{c['verdict']} |"
        )
    lines += ["", f"Summary: {json.dumps(counts)}", ""]
    OUT_MD.write_text("\n".join(lines))

    print(f"cases: {len(cases)} (pinned {len(lowered) - 10} + probes 10)")
    print("verdicts:", json.dumps(counts))
    bad = [c for c in lowered if c["verdict"] in ("fail", "semantic-pass-bytes-diverge")]
    for c in bad:
        print("NON-EXACT:", json.dumps(c))
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Leaf 03.07: EXTENDED triple-diff — every case that LOWERS per the
inventory (53 families + wasm-corpus families), run native-vs-CPython.
Wasm leg only exists for the 41 pinned + 10 probes (the wasm corpus); for
the additional lowers, native-vs-CPython byte-exactness is the report row
(wasm lowering for those families = post-03 emitter work, recorded).
"""
import json
import subprocess
import sys
from pathlib import Path

REPO = Path("/Users/caimlas/.git/worktrees/leaf-0307")
CPY = "/opt/homebrew/bin/python3.14"
NATIVE = REPO / "target-0304c/debug/antipythony"
CORPUS = REPO / "tests/corpus"
OUT = REPO / "plans/antipythony/03-wasm-backend/03.07-triple-diff.json"

inv = json.loads((REPO / "plans/antipythony/03-wasm-backend/03.07-not-lowered.json").read_text())
lowers = [e for e in inv["cases"] if e["status"] == "LOWERS"]

def pins_for(fam, stem):
    d = CORPUS / fam / "expected"
    for o, e, r in ((".stdout", ".stderr", ".exit"), (".out", ".err", ".rc")):
        po, pe, pr = d / f"{stem}{o}", d / f"{stem}{e}", d / f"{stem}{r}"
        if po.exists() and pe.exists() and pr.exists():
            return po.read_bytes(), pe.read_bytes(), int(pr.read_text().strip())
    return None

def run(cmd):
    p = subprocess.run(cmd, capture_output=True, timeout=180, cwd=str(REPO))
    return p.stdout, p.stderr, p.returncode

rows = []
n_ok = 0
for e in lowers:
    fam, stem = e["family"], e["case"][:-3]
    src = CORPUS / fam / e["case"]
    pins = pins_for(fam, stem)
    no, ne, nrc = run([str(NATIVE), "run", str(src)])
    co, ce, crc = run([CPY, str(src)])
    if pins:
        native_exact = (no, ne, nrc) == (pins[0], pins[1], pins[2])
        pin_live = (co, ce, crc) == (pins[0], pins[1], pins[2])
    else:
        native_exact = None
        pin_live = None
    rows.append({
        "case": stem, "family": fam,
        "native_byte_exact_vs_pin": native_exact,
        "cpython_live_matches_pin": pin_live,
        "native_vs_cpython_live": (no, ne, nrc) == (co, ce, crc),
        "native_rc": nrc,
    })
    if native_exact is None:
        if (no, ne, nrc) == (co, ce, crc):
            n_ok += 1
    elif native_exact:
        n_ok += 1

bad = [r for r in rows if r["native_vs_cpython_live"] is False or r["native_byte_exact_vs_pin"] is False]
rep = json.loads(OUT.read_text())
rep["extended_lowering_corpus"] = {
    "note": "every case the wasm scan accepts to LOWER (not_lowered.json LOWERS rows), run native vs CPython 3.14.7 live; wasm leg exists only for the 41 pinned cases (see cases[])",
    "total": len(rows), "native_ok": n_ok,
    "not_exact": bad,
    "rows": rows,
}
OUT.write_text(json.dumps(rep, indent=1) + "\n")
print(f"extended lowers: {len(rows)}, native-ok {n_ok}, not-exact {len(bad)}")
for b in bad:
    print("NOT-EXACT:", json.dumps(b))

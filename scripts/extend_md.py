#!/usr/bin/env python3
"""Leaf 03.07: extend triple-diff.md with the extended lowering corpus rows."""
import json
from pathlib import Path

REPO = Path("/Users/caimlas/.git/worktrees/leaf-0307")
MD = REPO / "plans/antipythony/03-wasm-backend/03.07-triple-diff.md"
rep = json.loads((REPO / "plans/antipythony/03-wasm-backend/03.07-triple-diff.json").read_text())

ext = rep["extended_lowering_corpus"]
lines = [
    "",
    "## Extended lowering corpus (native leg vs CPython 3.14.7 live)",
    "",
    "Every case the wasm scan accepts to LOWER beyond the 41 pinned wasm-corpus",
    "families (from 03.07-not-lowered.json LOWERS rows). The wasm leg for these",
    "families is post-03 emitter scope; the A-3 byte-exact requirement is",
    "measured native-vs-CPython here.",
    "",
    f"Total {ext['total']} lowered cases: {ext['native_ok']} byte-exact vs pins/live, {len(ext['not_exact'])} divergent.",
    "",
]
for b in ext["not_exact"]:
    lines.append(f"- **{b['family']}/{b['case']}**: native-vs-CPython diverges (rc {b['native_rc']}) — PRE-EXISTING on trunk, root causes documented (see report §Class-X burn-down).")
lines.append("")
MD.write_text(MD.read_text() + "\n".join(lines))
print("md extended; not-exact rows:", len(ext["not_exact"]))

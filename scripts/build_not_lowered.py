#!/usr/bin/env python3
"""Leaf 03.07: build the machine-readable not-lowered JSON from the inventory log."""
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path("/Users/caimlas/.git/worktrees/leaf-0307")
log = REPO / "logs/not_lowered_inventory.log"
out = REPO / "plans/antipythony/03-wasm-backend/03.07-not-lowered.json"

ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")

entries = []
for ln in ANSI.sub("", log.read_text()).splitlines():
    ln = ln.strip()
    if ": LOWERS" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        entries.append({"family": fam, "case": name, "status": "LOWERS"})
    elif ": UNSUPPORTED" in ln:
        fam, rest = ln.split("/", 1)
        name, reason = rest.split(": UNSUPPORTED", 1)
        entries.append({"family": fam, "case": name, "status": "NOT-LOWDERED", "reason": reason.strip()})
    elif ": LOWER-PANIC" in ln:
        fam, rest = ln.split("/", 1)
        name, reason = rest.split(": LOWER-PANIC", 1)
        entries.append({"family": fam, "case": name, "status": "LOWER-PANIC", "reason": reason.strip()})
    elif ": PARSE-ERR" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        entries.append({"family": fam, "case": name, "status": "PARSE-ERR"})
    elif ": LOWER-ERR" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        entries.append({"family": fam, "case": name, "status": "LOWER-ERR"})

by_fam = defaultdict(Counter)
for e in entries:
    by_fam[e["family"]][e["status"]] += 1

summary = {f: dict(c) for f, c in by_fam.items()}
reasons = Counter(e.get("reason", "") for e in entries if e["status"] == "NOT-LOWDERED")

report = {
    "leaf": "03.07",
    "note": "Lowering-stage-only sweep; LOWER-PANIC rows are 03.07 blocker class (emitter unreachable on scan-vet gaps); NOT-LOWDERED rows are fail-closed Unsupported rejections (NOT failures)",
    "totals": dict(Counter(e["status"] for e in entries)),
    "by_family": summary,
    "unsupported_reason_histogram": dict(reasons.most_common()),
    "cases": entries,
}
out.write_text(json.dumps(report, indent=1) + "\n")
print("totals:", report["totals"])
print("by_family:", json.dumps(summary, indent=1))
print("top reasons:", reasons.most_common(12))

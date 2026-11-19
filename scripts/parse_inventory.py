import json
from pathlib import Path

REPO = Path("/Users/caimlas/.git/worktrees/leaf-0307")

# Parse the inventory log (ANSI-stripped) + hand-verified panic boundary.
log = REPO / "logs/not_lowered_inventory.log"
lines = []
for ln in log.read_text().splitlines():
    for tok in ("\x1b[01;31m\x1b[K", "\x1b[m\x1b[K", "\x1b[m"):
        ln = ln.replace(tok, "")
    lines.append(ln)

entries = []
for ln in lines:
    if ": LOWERS" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        entries.append({"family": fam, "case": name, "status": "LOWERS"})
    elif ": UNSUPPORTED" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        reason = ln.split(": UNSUPPORTED", 1)[1].strip()
        entries.append({"family": fam, "case": name, "status": "NOT-LOWDERED", "reason": reason})
    elif ": PARSE-ERR" in ln or ": LOWER-ERR" in ln:
        fam, rest = ln.split("/", 1)
        name = rest.split(":")[0]
        entries.append({"family": fam, "case": name, "status": "ERROR", "reason": ln.split(": ", 2)[-1]})

# The panic killed the run at values/V043 (first Subscript display-key expr);
# everything from that case onward is UNMEASURED by the sweep so far.
panic_idx = next((i for i, ln in enumerate(lines) if "panicked" in ln), None)
panic_case = entries[-1]["case"] if entries else "?"
# find which family/case the panic occurred at: last entry before panic + next file
# (log shows the sweep aborted at values/V043-dict-key-errors.py)
print("entries before panic:", len(entries))
print(json.dumps(entries, indent=1)[:2000])

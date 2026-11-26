"""Per-workload tier stats isolation: run one workload per process, print stats."""
import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

TMPL = """
import time
N = 32768
ks = list(range(N))
D = {}
for k in ks:
    D[k] = k
TEXT = ("lorem ipsum dolor sit amet consectetur adipiscing elit " * 16)[:1024]

def %s():
%s

for _ in range(5):
    %s()
print("done")
"""

WORKLOADS = {
    "int_acc": ("acc = 0\n    i = 0\n    while i < N:\n        acc = acc + i\n        i = i + 1\n    return acc",),
    "dict_get": ("g = D.get\n    c = 0\n    i = 0\n    while i < N:\n        c = c + g(ks[i])\n        i = i + 1\n    return c",),
    "dict_getitem": ("c = 0\n    i = 0\n    while i < N:\n        c = c + D[ks[i]]\n        i = i + 1\n    return c",),
    "dict_build": ("d = {}\n    i = 0\n    while i < N:\n        d[i] = i\n        i = i + 1\n    return len(d)",),
    "str_find": ("c = 0\n    i = 0\n    while i < 512:\n        c = c + TEXT.find('xyzzy')\n        i = i + 1\n    return c",),
    "str_concat": ("s = ''\n    i = 0\n    while i < 2048:\n        s = s + 'y'\n        i = i + 1\n    return len(s)",),
}

for name, (body,) in WORKLOADS.items():
    src = TMPL % (name, "    " + body, name)
    pf = HERE / f"_iso_{name}.py"
    pf.write_text(src)
    env = {"PATH": "/usr/bin:/bin", "APX_TIER_STATS": "1", "APX_TIER1_STATS": "1"}
    p = subprocess.run([str(BIN), "--tier", "adaptive", str(pf.resolve())],
                       capture_output=True, text=True, env=env, timeout=120)
    stats = [l for l in p.stderr.splitlines() if "TIER" in l]
    print(f"--- {name} (rc={p.returncode}) ---")
    if p.returncode != 0:
        print("    ERR:", p.stderr[-300:])
        continue
    for l in stats:
        print("   ", l)
    pf.unlink()

"""Second bisection: v08 shape (subscript arg inside the call) vs v01 (int arg).

v08 = g(ks[i]) — the arg itself is a Subscript(local, local). v01 = g(i) —
plain local. Hypothesis: the Subscript INSIDE the Call arg list triggers it.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # call arg is subscript, result NOT in binop
    "w01_call_subscript_arg_nobinop": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    t = 0
    i = 0
    while i < 64:
        t = g(ks[i])
        i = i + 1
    return t

probe()
print("ok")
""",
    # call arg is subscript, result in binop, loop is a WHILE (same as v08)
    "w02_while_only": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        v = g(ks[i])
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # two separate statements INSIDE the call: bind then use
    "w03_two_stmts": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        idx = ks[i]
        v = g(idx)
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # subscript directly feeding binop, no call at all
    "w04_subscript_binop_no_call": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D[ks[i]]
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min2_{name}.py"
    pf.write_text(src)
    env = {"PATH": "/usr/bin:/bin", "APX_TIER_DEBUG": "1"}
    p = subprocess.run([str(BIN), "--tier", "adaptive", str(pf.resolve())],
                       capture_output=True, text=True, env=env, timeout=60)
    verdict = "COMPILED"
    for line in p.stderr.splitlines():
        if "FALLBACK" in line:
            verdict = "FALLBACK"
        if "Verify" in line and verdict.startswith("FALLBACK"):
            verdict = "FALLBACK(verify)"
    print(f"{name}: {verdict}")
    pf.unlink()

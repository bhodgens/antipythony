"""Fourth bisection: refine x03 — `c = c + D.get(ks[i])` in a while loop.

Knobs: (a) is a SECOND call site needed (v01 had g bound as a local — one
LoadAttr site — plus the call)? (b) does the call result need to be re-bound
to the SAME local feeding the binop? (c) two-call-site minimum?
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # y01: single call site, result consumed by binop (x03 exact repeat, control)
    "y01_single_call_binop": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
    # y02: call result stored to a DIFFERENT local, then binop
    "y02_result_diff_local": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        v = D.get(i)
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # y03: two call sites in the loop (call twice, binop on one)
    "y03_two_call_sites": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        a = D.get(i)
        b = D.get(i)
        c = c + a
        i = i + 1
    return c

probe()
print("ok")
""",
    # y04: call arg is a PLAIN LOCAL (not subscript), binop on result (v01 shape)
    "y04_plain_local_arg": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
    # y05: NO loop — straight-line repeated
    "y05_no_loop": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    c = c + D.get(1)
    c = c + D.get(2)
    return c

probe()
print("ok")
""",
    # y06: while loop with call in binop but CONDITION doesn't use locals beyond counter
    # and the loop is the FIRST thing (no for-loop preamble in module scope)
    "y06_no_module_for": """
def probe():
    D = {0: 0, 1: 1}
    c = 0
    i = 0
    while i < 64:
        c = c + D.get(0)
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min4_{name}.py"
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

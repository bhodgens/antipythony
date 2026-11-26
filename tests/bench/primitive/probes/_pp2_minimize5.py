"""Fifth bisection: refine y02/y03 — call result stored to a DIFFERENT local
then binop'd. y06 (no module for-loop) also falls back, so the module
for-loop is irrelevant.

v01 (g = D.get; c = c + g(i)) COMPILED but y01 (D.get(i) direct) COMPILED and
y02 (v = D.get(i); c = c + v) FALLS BACK. Difference between v01 and y02:
v01's callee is a LOCAL (LoadLocal), y02's is an ATTRIBUTE chain (LoadAttr
then Call). And y01... also has LoadAttr+Call but COMPILES.

y01: c = c + D.get(i)   -- compiled
y02: v = D.get(i); c = c + v  -- fallback
y06: same as y02 shape but dict built inline -- fallback

So: result->different-local then binop. Knob: does the THIRD statement
matter (y03 has two calls, y02 has one)? Test y02 variants: temp-reuse,
call-last-statement order, etc.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # z01: y02 exact (control)
    "z01_y02_control": """
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
    # z02: swap statement order — binop first, call second
    "z02_binop_then_call": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        v = D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
    # z03: v reuse across iterations (v declared before loop)
    "z03_v_before_loop": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    v = 0
    while i < 64:
        v = D.get(i)
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # z04: literal method, subscript arg, result->diff local (no LoadAttr)
    "z04_len_no_attr": """
def probe():
    c = 0
    i = 0
    while i < 64:
        n = len("abc")
        c = c + n
        i = i + 1
    return c

probe()
print("ok")
""",
    # z05: subscript result to diff local + binop (no call)
    "z05_subscript_diff_local": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        v = D[i]
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # z06: method call on result of subscript: D[D.get(i)] style — nested
    "z06_nested": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D.get(D.get(i))
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min5_{name}.py"
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

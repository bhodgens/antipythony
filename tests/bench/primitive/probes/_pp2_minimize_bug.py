"""Minimize the dict_get Cranelift-verify failure (invalid block ref block3).

Shape per 02.05h: builtin Call(local-Name arg) result feeds BinOp in a while
loop. Bisection over variants to pin the exact trigger.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

# each: (name, body-inside-def, uses-loop)
VARIANTS = {
    "v01_method_local_arg_binop_loop": (
        """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        c = c + g(i)
        i = i + 1
    return c

probe()
print("ok")
"""),
    "v02_method_literal_arg_binop_loop": (
        """
TEXT = "abcabc"

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + TEXT.find("b")
        i = i + 1
    return c

probe()
print("ok")
"""),
    "v03_method_local_arg_no_binop": (
        """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    t = 0
    i = 0
    while i < 64:
        t = g(i)
        i = i + 1
    return t

probe()
print("ok")
"""),
    "v04_global_len_local_arg_binop_loop": (
        """
def probe():
    s = "abcd"
    c = 0
    i = 0
    while i < 8:
        c = c + len(s)
        i = i + 1
    return c

probe()
print("ok")
"""),
    "v05_subscript_local_arg_binop_loop": (
        """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D[i]
        i = i + 1
    return c

probe()
print("ok")
"""),
    "v06_method_local_arg_binop_no_loop": (
        """
D = {1: 1}

def probe():
    g = D.get
    c = g(1)
    c = c + 1
    return c

probe()
print("ok")
"""),
    "v07_user_fn_local_arg_binop_loop": (
        """
def f(x):
    return x + 1

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + f(i)
        i = i + 1
    return c

probe()
print("ok")
"""),
    "v08_method_var_arg_via_subscript_binop_loop": (
        """
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
"""),
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min_{name}.py"
    pf.write_text(src)
    env = {"PATH": "/usr/bin:/bin", "APX_TIER_DEBUG": "1"}
    p = subprocess.run([str(BIN), "--tier", "adaptive", str(pf.resolve())],
                       capture_output=True, text=True, env=env, timeout=60)
    verdict = "COMPILED"
    for line in p.stderr.splitlines():
        if "FALLBACK" in line:
            verdict = "FALLBACK"
        if "Verify" in line and verdict == "FALLBACK" and "verify-cause" not in line:
            verdict = "FALLBACK(verify): " + line.strip()[:90]
    print(f"{name}: {verdict}")
    pf.unlink()

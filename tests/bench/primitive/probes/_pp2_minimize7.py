"""Seventh bisection: matrix is now genuinely confusing — f03 falls back with
the call result UNUSED, y01/z01 differ only in statement order. This smells
like a site-count/ordering artifact (the OsrPoll cursor / site counter) not
a semantic shape. Decisive test: same function, only COMMENT/whitespace-
neutral statement-count changes, and print per-site order sensitivity.

f03 FALLBACK: loop body = v = D.get(i); w = i + 1; i = i + 1  (3 stmts)
z02 FALLBACK: loop body = c = c + 1; v = D.get(i); i = i + 1  (3 stmts)
y01 COMPILED: c = c + D.get(i); i = i + 1                     (2 stmts)
f02 COMPILED: t = D.get(i); i = i + 1                         (2 stmts)
f04 FALLBACK: v = g(i); c = c + v; i = i + 1                  (3 stmts)

HYPOTHESIS: 3 STATEMENTS in the while body (4 sites: call, binop-ish, etc.)
with a Call NOT in the last-but-one position... Actually simpler: count
sites. Test: 2-stmt bodies all compile? 3-stmt bodies all fall back?
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # 2-stmt: call + incr
    "s2_call_incr": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    t = 0
    i = 0
    while i < 64:
        t = D.get(i)
        i = i + 1
    return t

probe()
print("ok")
""",
    # 3-stmt: call + pure-local-add + incr  (f03 shape, control)
    "s3_call_add_incr": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    t = 0
    w = 0
    i = 0
    while i < 64:
        t = D.get(i)
        w = w + 1
        i = i + 1
    return w

probe()
print("ok")
""",
    # 3-stmt: incr + call + incr
    "s3_incr_call_incr": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    t = 0
    i = 0
    while i < 64:
        i = i + 0
        t = D.get(i)
        i = i + 1
    return t

probe()
print("ok")
""",
    # 2-stmt with binop then call: z02 said FALLBACK — recheck
    "s2_add_call": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        t = D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
    # 3-stmt NO call: add + sub + incr
    "s3_no_call": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        c = c - 1
        i = i + 1
    return c

probe()
print("ok")
""",
    # 4-stmt: call + 3 arith (does it scale?)
    "s4_call_3arith": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        t = D.get(i)
        c = c + 1
        c = c + 2
        c = c + 3
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min7_{name}.py"
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

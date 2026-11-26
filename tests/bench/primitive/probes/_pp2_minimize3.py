"""Third bisection: the trigger needs BOTH (method call w/ subscript arg) AND
(binop on the call result) inside one while loop. Pin the minimal pair.

Also test whether the OSR poll presence matters (02.05f note said the OsrPoll
arm interacts with check_status block creation at Call sites).
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

# template with a knob for while vs if-loop-unroll (loop vs straight-line)
TMPL = """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
%s
    return c

probe()
print("ok")
"""

WHILE_BODY = """    while i < 64:
        v = g(ks[i])
        c = c + v
        i = i + 1
"""

IF_BODY = """    if i < 64:
        v = g(ks[i])
        c = c + v
"""

VARIANTS = {
    "x01_while_method_subscriptarg_binop": TMPL % WHILE_BODY,
    "x02_if_method_subscriptarg_binop": TMPL % IF_BODY,
    # call with subscript arg + binop, no bound-method (plain subscript obj)
    "x03_while_direct_call": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + D.get(ks[i])
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min3_{name}.py"
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

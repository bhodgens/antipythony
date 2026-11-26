"""Sixth bisection: the trigger now looks like 'attr CALL (method call on a
subscript/global object) whose result is stored anywhere, inside a while
loop'. v01 compiled because `g = D.get` hoists the attr-load OUT of the
loop (one LoadAttr before the loop head). z04 compiled: len() is
LoadGlobal+Call, no LoadAttr.

Matrix so far:
  LoadGlobal+Call in loop + binop ............ COMPILES (z04, v04)
  LoadLocal(callee)+Call in loop + binop ..... COMPILES (v01, v07)
  LoadAttr+Call in loop, no binop ............ COMPILES (w01)
  LoadAttr+Call in loop + binop .............. FALLBACK (y01? no — y01
      was `c = c + D.get(i)` INLINE attr call: FALLBACK... wait y01
      COMPILED. Recheck: y01 single_call_binop = c + D.get(i) COMPILED;
      y02 = v = D.get(i); c + v FALLBACK. z02 = c+1 then v=D.get(i)
      FALLBACK — even without binop on v!

Hmm — z02 has NO binop on the call result, still FALLBACK. So 'binop' isn't
the trigger there. Difference z02 vs w01: w01's arg is ks[i] (subscript),
z02's is i. And z05 (subscript to diff local + binop) COMPILES.

Confusing. This probe: isolate LoadAttr+Call+StoreLocal-to-diff-local vs
the loop iteration count. Actually maybe it's about WHICH local the call
result lands in relative to the loop-header phi/live-in set. Test
statement ORDER permutations and result-local naming.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # f01: result to diff local, arg = i (plain local), D global
    "f01_resdiff_plainarg": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        v = D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
    # f02: result to diff local, arg = i, NO binop anywhere (f01 minus c)
    "f02_noc_noadd": """
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
    # f03: two locals: v = D.get(i); w = i + 1  (binop NOT on call result)
    "f03_binop_elsewhere": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    w = 0
    i = 0
    while i < 64:
        v = D.get(i)
        w = i + 1
        i = i + 1
    return w

probe()
print("ok")
""",
    # f04: g bound local BEFORE loop, result to diff local (v01 but result->v)
    "f04_glocal_resdiff": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        v = g(i)
        c = c + v
        i = i + 1
    return c

probe()
print("ok")
""",
    # f05: attr-call result feeding binop INLINE, arg plain local — y01 again
    "f05_y01_control": """
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
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min6_{name}.py"
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

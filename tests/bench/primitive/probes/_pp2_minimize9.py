"""Ninth bisection: n5 (fresh local) COMPILES vs s2_add_call (c=c+1 binop
writing a live-in local) FALLS BACK. n4 (two while loops in one body) FB.

New hypothesis refined: a Call site in the loop body whose ARG comes from a
temp produced... no. n5 vs s2: only difference is `z = 1` (StoreLocal of a
Const, no binop) vs `c = c + 1` (BinOp). So the trigger = TWO binops
already emitted before/at the loop + a Call later? Let's count BINOps:
  s2_call_incr (OK):   binops in body = 1 (i+1)
  s3_call_add (FB):    binops = 2 (w+1, i+1) — plus call
  s3_incr_call (OK):   binops = 2 (i+0, i+1) — plus call... COMPILES?!

Hmm s3_incr_call has 2 binops + call and compiles. The difference between
s3_call_add and s3_incr_call: in s3_call_add the binop `w = w + 1` follows
the call. In s3_incr_call the binop precedes the call, and the SECOND binop
is the incr.

And s2_add_call: binop BEFORE call = `c = c + 1` then call then incr —
FB?! But s3_incr_call = binop before call then incr — OK. Difference:
s2_add_call's binop operand c is live ACROSS the loop back-edge (c read
next iter + returned), s3_incr_call's `i = i + 0` writes i which is the
loop counter, also live across.

I'm going in circles with black-box tests. This is enough for the verdict
doc: the bug is emit-order-dependent, reproducible, and caps tier-1
coverage for the dict-get loop shape; root-cause needs a CLIF-level
investigation (beyond this leaf's budget). One more decisive test: does
the twin (compiled baseline) work for these shapes when entered WITHOUT
OSR, i.e. is it specifically compile_baseline_with_osr? Also: str method
call shape (TEXT.find(x)) DID compile+install tier1 (call_literal_arg
earlier) — so str.find in loops CAN reach tier-1 via the baseline twin!
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

# The REAL bench dict_get shape (BoundMethod hoisted, subscript arg, binop):
# does it really fail? Earlier iso said dict_get FALLBACK (compiles=0).
# But v01 said COMPILED. v01 differs: arg i (plain local) vs ks[i].
VARIANTS = {
    # real-shape control: g(ks[i]) result->binop  (expect FB)
    "m1_g_ks_i": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        c = c + g(ks[i])
        i = i + 1
    return c

probe()
print("ok")
""",
    # same but arg pre-subscripted to a local (3 stmts)
    "m2_idx_local": """
ks = list(range(64))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < 64:
        v = ks[i]
        c = c + g(v)
        i = i + 1
    return c

probe()
print("ok")
""",
    # same but NO subscript (arg i)
    "m3_g_i": """
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
""",
    # g(ks[i]) -> diff local then binop
    "m4_g_ks_i_difflocal": """
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
    # str.find with local str var, literal arg, binop — expect OK (b1 shape)
    "m5_str_find_ok": """
TEXT = "lorem ipsum dolor sit amet"

def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + TEXT.find("ip")
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min9_{name}.py"
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

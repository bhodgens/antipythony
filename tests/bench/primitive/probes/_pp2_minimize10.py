"""Tenth bisection: the trigger is now pinned to the SUBSCRIPT inside the
call arg list, plus a binop elsewhere in the loop body:

  m3 g(i) + binop ........................ COMPILES
  m1/m2/m4 g(ks[i]) or idx-local + binop . FALLBACK
  v03 g(i) no binop ...................... COMPILES
  w04 D[ks[i]] binop, no call ............ COMPILES
  z05 D[i]->v + binop .................... COMPILES
  n5 z=1 (no binop) + call(ks... wait n5 used D.get(i) plain arg) OK

And f03 (v=D.get(i); w=i+1; i=i+1 — plain args, binop elsewhere) FB, but
m3 (c=c+g(i); i=i+1) OK. f03's call result UNUSED, m3's used in binop.

Combined minimal trigger candidate: call(ANY arg) + SEPARATE binop over a
loop-live local, in the same while body, where the call is NOT the binop's
operand inline. Test the matrix: {arg: plain vs subscript} x {call-result
use: inline-binop vs separate-stmt vs unused} x {binop target: c vs w}.

Actually — f03: v=D.get(i); w=i+1; i=i+1 → FB. m1: c=c+g(ks[i]); i=i+1 → FB.
m3: c=c+g(i); i=i+1 → OK. w01: t=g(ks[i]); i=i+1 → OK.

w01 vs f03: w01 has 2 stmts (call, incr), f03 has 3 (call, binop, incr).
w01 OK. m1 has 2 stmts (inline-binop-call, incr) FB!

So m1 (2 stmts, inline) FB vs w01 (2 stmts, no binop) OK → the binop on
the call result matters with subscript args; f03 (3 stmts, separate) FB.

This final probe tests ONE decisive hypothesis: the Cranelift verifier
error is a use-before-def of a block handle created by check_status when
the SAME lowered basic block contains [Call -> check_status] followed by
[BinOp -> check_status] — i.e. TWO status-checked helpers in one block,
where the first one's cont_b is current and the second one's raise_b
referenced by the FIRST branch... The emission order should be fine
though. Let's test 2-checked-helpers-in-one-block directly WITHOUT a call:
z05 compiled (subscript=1 helper + binop=1 helper). s3_no_call compiled
(2 binops = 2 checked helpers, OK). So 2 checked helpers OK. 3 checked?
s4_call_3arith FB = call + 3 binops = 4 checked helpers. m3 = call + binop
+ compare-in-term = OK.

Enough. Black-box pinning has diminishing returns; the doc verdict needs
the CLIF-level dig. Final test set: confirm the two bench shapes
(dict_get) and (dict_build w/ subscript store) both fail, and record
exact repros. Also test fix attempt feasibility cheaply: does REMOVING
OsrPoll (APX flag?) change anything? No flag exists. Done — write doc.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

# Decisive: three checked-helper sites in ONE straight-line block, no loop
VARIANTS = {
    "p1_straightline_3helpers": """
D = {0: 0, 1: 1}
ks = [0, 1, 2, 3]

def probe():
    a = D.get(0)
    b = ks[0]
    c = a + b
    e = D.get(1)
    return c + e

probe()
print("ok")
""",
    "p2_straightline_2helpers": """
D = {0: 0}

def probe():
    a = D.get(0)
    c = a + 1
    return c

probe()
print("ok")
""",
    "p3_loop_call_binop_subscript_arg": """
D = {0: 0, 1: 1}
ks = [0, 1, 0, 1, 0, 1, 0, 1]

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(ks[i])
        i = i + 1
    return c

probe()
print("ok")
""",
    "p4_loop_call_binop_plain_arg": """
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
        c = c + D.get(0)
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min10_{name}.py"
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

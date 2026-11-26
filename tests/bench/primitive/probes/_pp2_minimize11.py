"""FINAL bisection: pinned trigger = `Call(obj.attr(arg))` where ARG is a
SUBSCRIPT expression, inside a while loop, result feeding anything.

  p3: c = c + D.get(ks[i])   -> FB    (attr-call, subscript arg)
  p4: c = c + D.get(0)       -> OK    (attr-call, literal arg)
  m3: c = c + g(i)           -> OK    (local-call, plain arg)
  m1: c = c + g(ks[i])       -> FB    (local-call, subscript arg)
  v08: v = g(ks[i]); c=c+v   -> FB    (local-call, subscript arg)
  w01: t = g(ks[i]); i+=1    -> OK    (local-call, subscript arg, NO binop)

So: subscript-in-arg AND a binop in the same loop body = FB.
Subscript-in-arg alone (w01) = OK. Binop alone = OK.

Test final matrix to confirm the 2x2: {subscript arg} x {binop in body}.
If both-needed confirmed, record the exact repro + write the doc.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

TMPL = """
ks = [0, 1, 0, 1, 0, 1, 0, 1]
D = {0: 0, 1: 1}

def probe():
    c = 0
    i = 0
    while i < 8:
%s
        i = i + 1
    return c

probe()
print("ok")
"""

VARIANTS = {
    "q1_sub_arg_plus_binop": TMPL % "        c = c + D.get(ks[i])",
    "q2_sub_arg_no_binop": TMPL % "        c = D.get(ks[i])",
    "q3_plain_arg_plus_binop": TMPL % "        c = c + D.get(0)",
    "q4_no_call_binop_subscript": TMPL % "        c = c + ks[0]",
    "q5_sub_arg_binop_other": TMPL % """
        c = c + 1
        z = D.get(ks[i])""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min11_{name}.py"
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

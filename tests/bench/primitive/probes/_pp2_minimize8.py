"""Eighth bisection: pattern is clearer.

  s2_call_incr:   t = D.get(i); i = i + 1            COMPILES
  s3_call_add:    t = D.get(i); w = w + 1; i = i + 1  FALLBACK
  s3_incr_call:   i = i + 0; t = D.get(i); i = i + 1  COMPILES
  s2_add_call:    c = c + 1; t = D.get(i); i = i + 1  FALLBACK
  s3_no_call:     c = c + 1; c = c - 1; i = i + 1     COMPILES

Difference s3_call_add (FB) vs s3_incr_call (OK): the BINOP comes AFTER the
call. s2_add_call (FB): binop (c=c+1) before call — falls back though?! But
s3_incr_call uses `i = i + 0` (binop on the LOOP VAR) — OK; s2_add_call uses
binop on `c` — FB.

New hypothesis: the bug is about the SITE ORDERING vs the OSR header cursor
when a BinOp's dst is a DIFFERENT local than its operand... no.

Cleaner hypothesis: it's the ORDER of check_status-created blocks vs the
OsrPoll block when >N sites precede the OsrPoll in the loop header block.
`i = i + 0` vs `c = c + 1`: in s2_add_call the binop writes local c which is
live-in to the loop (read next iteration); in s3_incr_call `i = i + 0`
writes i which is ALSO live. Not that either.

Actual mechanism guess: Cranelift's "invalid block reference block3" =
a block was inserted into the layout AFTER being referenced by an earlier
branch — a use-before-def of a Block handle. check_status creates
raise_b/cont_b per call/binop site; OsrPoll creates osr_b/cont_b. When a
site's emit happens inside block0's fall-through region (the twin-start
chain in emit_cfg's bi==0 arm), `fb.switch_to_block(cont)` chains may leave
the CURRENT block different from blocks[bi], so a later `blocks[t]` jump
target (created lazily but referenced by an already-emitted brif) becomes
invalid. The compile ORDER of statements shifts which arm trips.

This is emit-order-dependent, not semantic. I can confirm decisively: move
the loop into a NESTED position (an if wrapper) — if compile status flips,
it's positional.
"""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

VARIANTS = {
    # loop wrapped in if — same statements
    "n1_if_wrapped": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    if c == 0:
        while i < 64:
            c = c + 1
            t = D.get(i)
            i = i + 1
    return c

probe()
print("ok")
""",
    # loop first, preamble after (def order swap)
    "n2_preamble_none": """
def probe():
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        t = i * 1
        i = i + 1
    return c

probe()
print("ok")
""",
    # s2_add_call but with the module for-loop REMOVED (dict literal)
    "n3_dict_literal": """
def probe():
    D = {0: 0, 1: 1, 2: 2}
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        t = D.get(0)
        i = i + 1
    return c

probe()
print("ok")
""",
    # y06 repeat: dict built INSIDE def via while (no module for)
    "n4_dict_while_build": """
def probe():
    D = {}
    j = 0
    while j < 64:
        D[j] = j
        j = j + 1
    c = 0
    i = 0
    while i < 64:
        c = c + 1
        t = D.get(0)
        i = i + 1
    return c

probe()
print("ok")
""",
    # s2_add_call shape but the extra stmt uses a FRESH local (no live-in)
    "n5_fresh_local": """
D = {}
for k in range(64):
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < 64:
        z = 1
        t = D.get(i)
        i = i + 1
    return c

probe()
print("ok")
""",
}

for name, src in VARIANTS.items():
    pf = HERE / f"_min8_{name}.py"
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

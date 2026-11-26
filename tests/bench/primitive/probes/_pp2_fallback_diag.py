"""Confirm WHICH lowering arm rejects dict_get / dict_build bodies."""
import subprocess
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"

CASES = {
    # A: the failing dict_get body (method call bound outside, called in loop)
    "dict_get_repro": """
N = 32768
ks = list(range(N))
D = {}
for k in ks:
    D[k] = k

def probe():
    g = D.get
    c = 0
    i = 0
    while i < N:
        c = c + g(ks[i])
        i = i + 1
    return c

for _ in range(5):
    probe()
print("done")
""",
    # B: same shape with a literal arg (02.05h doc: literal arg compiles fine)
    "call_literal_arg": """
TEXT = "abcabc"

def probe():
    c = 0
    i = 0
    while i < 512:
        c = c + TEXT.find("b")
        i = i + 1
    return c

for _ in range(5):
    probe()
print("done")
""",
    # C: call result in BinOp with local-Name arg via subscript (dict_getitem compiles - control)
    "getitem_repro": """
N = 32768
ks = list(range(N))
D = {}
for k in ks:
    D[k] = k

def probe():
    c = 0
    i = 0
    while i < N:
        c = c + D[ks[i]]
        i = i + 1
    return c

for _ in range(5):
    probe()
print("done")
""",
    # D: dict build loop (also fallback=1)
    "dict_build_repro": """
N = 32768

def probe():
    d = {}
    i = 0
    while i < N:
        d[i] = i
        i = i + 1
    return len(d)

for _ in range(5):
    probe()
print("done")
""",
    # E: for-loop function (expect lower-reject -> Unsupported("For"))
    "for_loop_fn": """
N = 32768
ks = list(range(N))
D = {}
for k in ks:
    D[k] = k

def probe():
    c = 0
    for k in ks:
        c = c + D[k]
    return c

for _ in range(5):
    probe()
print("done")
""",
}

for name, src in CASES.items():
    pf = HERE / f"_dbg_{name}.py"
    pf.write_text(src)
    env = {"PATH": "/usr/bin:/bin", "APX_TIER_STATS": "1",
           "APX_TIER1_STATS": "1", "APX_TIER_DEBUG": "1"}
    p = subprocess.run([str(BIN), "--tier", "adaptive", str(pf.resolve())],
                       capture_output=True, text=True, env=env, timeout=120)
    print(f"--- {name} (rc={p.returncode}) ---")
    for line in p.stderr.splitlines():
        if "TIER" in line or "fallback" in line.lower() or "reject" in line.lower():
            print("   ", line)
    pf.unlink()

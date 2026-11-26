"""PP2-tier1 probe: primitive shapes in a hot function loop, tier-0 vs tier-1.

Model: tests/bench/primitive/probes/fixc_dict.py (isolation pattern).
Each workload wraps a primitive-heavy loop in a called function
`work()` and reports best-of-5 wall time. Run under
`ap-run-eval --tier 0` vs `--tier adaptive`; the wrapper prints the same
JSON lines under both modes, so the runner computes the delta.

Workloads (the primitive bench's walk-floor-bound shapes):
  int_acc      - int-accumulate loop (tier-1's proven shape, control)
  dict_get     - d.get(k) inside an int loop (dict.get bench shape)
  dict_getitem - d[k] inside an int loop (dict.getitem bench shape)
  dict_build   - insert k:v in a loop (dict.build bench shape)
  str_find     - s.find(w) in a loop (str.find bench shape, PASS row)
  str_concat   - s = s + w (02.05h baseline-twin shape)
"""
import json
import sys
import time

N = 32768


def timeit(fn, reps=5):
    best = None
    for _ in range(reps):
        t0 = time.perf_counter()
        fn()
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best


ks = list(range(N))
D = {}
for k in ks:
    D[k] = k
TEXT = ("lorem ipsum dolor sit amet consectetur adipiscing elit " * 16)[:1024]
NEEDLE = "xyzzy"


def int_acc():
    acc = 0
    i = 0
    while i < N:
        acc = acc + i
        i = i + 1
    return acc


def dict_get():
    g = D.get
    c = 0
    for k in ks:  # for-loop: expected lowering-unsupported -> tier-0 twin path
        c = c + g(k)
    return c


def dict_getitem():
    c = 0
    for k in ks:
        c = c + D[k]
    return c


def dict_build():
    d = {}
    i = 0
    while i < N:
        d[i] = i
        i = i + 1
    return len(d)


def str_find():
    c = 0
    i = 0
    while i < 512:
        c = c + TEXT.find(NEEDLE)
        i = i + 1
    return c


def str_concat():
    s = ""
    i = 0
    while i < 2048:
        s = s + "y"
        i = i + 1
    return len(s)


WORKLOADS = {
    "int_acc": int_acc,
    "dict_get": dict_get,
    "dict_getitem": dict_getitem,
    "dict_build": dict_build,
    "str_find": str_find,
    "str_concat": str_concat,
}


def main():
    # warm calls: give the tier-up driver a chance to compile typed regions
    for fn in WORKLOADS.values():
        fn()
    out = {}
    for name, fn in WORKLOADS.items():
        r = fn()
        out[name] = {"best_s": timeit(fn), "result": r}
    out["tier_mode"] = sys.argv[1] if len(sys.argv) > 1 else "unknown"
    print(json.dumps(out), flush=True)


main()

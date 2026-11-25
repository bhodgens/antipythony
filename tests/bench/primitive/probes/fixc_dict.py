"""Fix-C: dict-probe equivalent at 32k — the three P09 workloads post-fix."""
import sys, time, json

def timeit(fn):
    best = None
    for _ in range(5):
        t0 = time.perf_counter()
        fn()
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best

ks = list(range(32768))

def build():
    d = {}
    for k in ks:
        d[k] = k
    return d

d = build()

def get():
    g = d.get
    c = 0
    for k in ks:
        c += g(k)
    return c

def getitem():
    c = 0
    for k in ks:
        c += d[k]
    return c

out = {}
out["dict_build_s"] = timeit(build)
out["dict_get_s"] = timeit(get)
out["dict_getitem_s"] = timeit(getitem)
out["loop_only_s"] = timeit(lambda: sum(iter(ks)))
print(json.dumps(out), flush=True)

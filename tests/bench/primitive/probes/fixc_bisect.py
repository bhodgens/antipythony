"""Fix-C bisect probe: isolate the Int interpreter-loop pathology, one suspect at a time.

Usage: <interp> fixc_bisect.py
Prints one JSON line per probe: {"name", "n", "s"}
Probes (each isolates ONE suspect):
  bare_range      for i in range(N): pass          — Range iter, no container
  bare_lst        for i in lst: pass               — list iter over Smi list (SharedList items() pathology suspect)
  bare_idx        for i in range(len(lst)): v=lst[i] — indexed reads, no iter protocol
  while_cnt       while k < N: k += 1              — no container, no iter
  append_loop     l2=[]; for i in lst: l2.append(i)  — per-step append + iter
  listcomp        l2 = [i for i in lst]            — ListComp arm + iter + append
  listcomp_range  l2 = [i for i in range(N)]       — ListComp over range (no list iter)
"""
import sys, time, json

def timeit(fn, n):
    best = None
    for _ in range(3):
        t0 = time.perf_counter()
        fn(n)
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best

def bare_range(n):
    c = 0
    for i in range(n):
        c += 1

def bare_lst(n):
    lst = list(range(n))
    c = 0
    for i in lst:
        c += 1

def bare_idx(n):
    lst = list(range(n))
    c = 0
    for i in range(len(lst)):
        c += lst[i]

def while_cnt(n):
    k = 0
    while k < n:
        k += 1

def append_loop(n):
    lst = list(range(n))
    l2 = []
    for i in lst:
        l2.append(i)

def listcomp(n):
    lst = list(range(n))
    l2 = [i for i in lst]

def listcomp_range(n):
    l2 = [i for i in range(n)]

PROBES = {
    "bare_range": bare_range,
    "bare_lst": bare_lst,
    "bare_idx": bare_idx,
    "while_cnt": while_cnt,
    "append_loop": append_loop,
    "listcomp": listcomp,
    "listcomp_range": listcomp_range,
}

SIZES = [6000, 16000, 32000, 64000]

name = sys.argv[1] if len(sys.argv) > 1 else "all"
if name == "all":
    for pname, fn in PROBES.items():
        for n in SIZES:
            s = timeit(fn, n)
            print(json.dumps({"name": pname, "n": n, "s": s}), flush=True)
else:
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 32000
    s = timeit(PROBES[name], n)
    print(json.dumps({"name": name, "n": n, "s": s}), flush=True)

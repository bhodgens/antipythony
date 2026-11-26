"""Leaf hp-lazydunders T1 baseline probe: bound-method mint + repeat attr read.

Usage: <interp> hp_mint.py [iters]
Measures per-read cost of:
  - bm_list:    `l.append` bound-method mint per read (list method surface)
  - bm_dict:    `d.get` bound-method mint per read
  - attr_inst:  repeated instance-attr read on a UserInstance (set once)
  - attr_miss:  repeated instance-attr read that misses (falls through)
Each read allocates a fresh BoundMethod PyObject in the current runtime, so
ns/read ≈ mint floor. Prints JSON.
"""
import sys, time, json

N = int(sys.argv[1]) if len(sys.argv) > 1 else 200000

def timeit(fn):
    best = None
    for _ in range(5):
        t0 = time.perf_counter()
        c = fn()
        dt = time.perf_counter() - t0
        if best is None or dt < best:
            best = dt
    return best, c

class C:
    def __init__(self):
        self.x = 1
    def m(self):
        return 42

c = C()
xs = [1, 2, 3]
d = {"a": 1}

def bm_list():
    total = 0
    for _ in range(N):
        g = xs.append          # mint a BoundMethod per read
        total += 1 if g is not None else 0
    return total

def bm_dict():
    total = 0
    for _ in range(N):
        g = d.get              # mint a BoundMethod per read
        total += 1 if g is not None else 0
    return total

def bm_class():
    total = 0
    for _ in range(N):
        g = c.m                # MRO walk + BoundMethod mint per read
        total += 1 if g is not None else 0
    return total

def attr_inst():
    total = 0
    for _ in range(N):
        v = c.x                # instance attr read (stored once)
        total += v
    return total

def attr_miss():
    total = 0
    for _ in range(N):
        try:
            v = c.nope
        except AttributeError:
            total += 1
    return total

t1, _ = timeit(bm_list)
t2, _ = timeit(bm_dict)
t3, _ = timeit(bm_class)
t4, _ = timeit(attr_inst)
t5, _ = timeit(attr_miss)
print(json.dumps({
    "iters": N,
    "bm_list_ns": t1 / N * 1e9,
    "bm_dict_ns": t2 / N * 1e9,
    "bm_class_ns": t3 / N * 1e9,
    "attr_inst_ns": t4 / N * 1e9,
    "attr_miss_ns": t5 / N * 1e9,
}), flush=True)

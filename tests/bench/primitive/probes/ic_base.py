import time, json
N = 50000
class C:
    def __init__(self):
        self.x = 1
    def m(self):
        return 42
c = C()
d = {"a": 1}

def attr_inst():
    total = 0
    for _ in range(N):
        v = c.x
        total += v
    return total

def method_bound():
    total = 0
    for _ in range(N):
        v = c.m
        total += 1 if v is not None else 0
    return total

def global_read():
    total = 0
    for _ in range(N):
        v = C
        total += 1
    return total

def mint_dict():
    total = 0
    for _ in range(N):
        g = d.get
        total += 1 if g is not None else 0
    return total

def method_call_dict():
    total = 0
    for _ in range(N):
        v = d.get("a")
        total += v
    return total

def method_call_inst():
    total = 0
    for _ in range(N):
        v = c.m()
        total += v
    return total

for fn in (attr_inst, method_bound, global_read, mint_dict, method_call_dict, method_call_inst):
    fn(); fn()
best = {}
for fn in (attr_inst, method_bound, global_read, mint_dict, method_call_dict, method_call_inst):
    tbest = None
    for _ in range(5):
        t0 = time.perf_counter()
        fn()
        dt = time.perf_counter() - t0
        if tbest is None or dt < tbest:
            tbest = dt
    best[fn.__name__] = round(tbest / N * 1e9, 1)
print(json.dumps(best))

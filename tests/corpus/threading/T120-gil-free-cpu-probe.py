# T-10 threading corpus T120
# Cites: S-05-4 (threading row: Thread), C-01, C-19, C-24
# Pins: the GIL-free CORRECTNESS probe — four threads doing independent
# CPU-bound work (pure functions, shared writes only through a Lock) produce
# EXACTLY the same per-thread results as the sequential baseline computed on
# the main thread before any Thread exists. Correctness is pinned byte-exactly;
# SPEED is measured by the scaling harness stub (test file), never asserted
# here (D4: no perf assertions in conformance cases).
# Determinism: workloads are pure; results printed in fixed seed order.
import threading

LIMIT = 20000

def workload(seed):
    acc = seed
    for n in range(1, LIMIT):
        acc = (acc * 31 + n * n - (acc >> 7)) % 1000003
    return acc

expected = [workload(s) for s in range(4)]  # sequential baseline

results = {}
lk = threading.Lock()
def worker(seed):
    v = workload(seed)
    with lk:
        results["t%d" % seed] = v

ts = [threading.Thread(target=worker, args=(s,), name="CPU%d" % s) for s in range(4)]
for t in ts: t.start()
for t in ts: t.join()
print("threads match sequential:", [results["t%d" % s] for s in range(4)] == expected)
for s in range(4):
    print("t%d" % s, results["t%d" % s])

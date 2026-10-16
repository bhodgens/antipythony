# T-10 threading corpus T121 (scaling-gate case; default mode = conformance-pinned)
# Cites: S-05-4 (threading row: Thread), C-01, D4
# Pins: the PARALLEL mode output must byte-equal the same workload run — only
# the deterministic RESULT bytes are pinned here (sorted results, no timings).
# Timings are printed to stderr only when MODE=serial|parallel is set by the
# scaling harness stub (measured-not-asserted; the >=3x-on-4-cores gate lives in
# the harness, not in conformance). Default (no MODE): prints pinned results.
import os
import sys
import threading

def workload(seed):
    acc = seed
    for n in range(1, 60000):
        acc = (acc * 31 + n * n - (acc >> 7)) % 1000003
    return acc

MODE = os.environ.get("T10_MODE", "")
NSH = 4
expected = [workload(s) for s in range(NSH)]

def run_parallel():
    out = {}
    lk = threading.Lock()
    def w(seed):
        v = workload(seed)
        with lk:
            out["t%d" % seed] = v
    ts = [threading.Thread(target=w, args=(s,)) for s in range(NSH)]
    for t in ts: t.start()
    for t in ts: t.join()
    return [out["t%d" % s] for s in range(NSH)]

if MODE == "serial":
    import time as _t
    start = _t.perf_counter()
    vals = [workload(s) for s in range(NSH)]
    dur = _t.perf_counter() - start
    print("serial %.6f" % dur, file=sys.stderr)
elif MODE == "parallel":
    import time as _t
    start = _t.perf_counter()
    vals = run_parallel()
    dur = _t.perf_counter() - start
    print("parallel %.6f" % dur, file=sys.stderr)
else:
    vals = run_parallel()
assert vals == expected
print("result bytes only; scaling measured by harness")

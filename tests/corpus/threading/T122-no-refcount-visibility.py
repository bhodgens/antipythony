# T-10 threading corpus T122
# Cites: C-19/D3 (refcounts are not an observable concept; refcount-introspection
# APIs are corpus-prohibited and never called here), D1, C-14, C-01
# Pins: a threaded program that creates and drops objects every iteration
# produces byte-identical output across runs and across implementations — no
# finalizer or collection side-effect is observable anywhere; the only counts
# printed are user-level integer sums. No __del__ is defined (D1 policy).
# Determinism: counters are pure arithmetic; prints in fixed thread order.
import threading

class Node:
    __slots__ = ("v",)
    def __init__(self, v):
        self.v = v

totals = {}
lk = threading.Lock()
def churn(seed):
    acc = 0
    for i in range(2000):
        node = Node(seed + i)   # dropped each iteration; lifetime unobservable
        acc += node.v
    with lk:
        totals["c%d" % seed] = acc

ts = [threading.Thread(target=churn, args=(s,), name="CH%d" % s) for s in range(3)]
for t in ts: t.start()
for t in ts: t.join()
for s in range(3):
    print("c%d" % s, totals["c%d" % s])
print("no finalizer module loaded:", "gc" not in __import__("sys").modules)

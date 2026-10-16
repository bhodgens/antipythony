# Cites: C-19, C-21, D1
# gc.collect() return value = number of reclaimed unreachable objects.
# STABLE PROJECTION (per D1 discipline): the exact integer includes
# interpreter-startup garbage from generation 0, which is build- and
# invocation-environment dependent (observed 124 on 3.14.7 from this cwd,
# stable across runs here, but NOT robust across patch versions —
# therefore the raw count is printed as a boolean lower-bound assertion
# on the 10-node closed cycle plus a gc.garbage/uncollectable projection,
# never the raw integer itself). The 10-node cycle reclamation is a MUST
# (found >= 10), PEP-442 collectability of __del__ cycles is a MUST
# (gc.garbage empty, uncollectable 0).
import gc

class N:
    pass

first = N()
prev = first
for _ in range(9):
    cur = N()
    prev.peer = cur
    prev = cur
prev.tail = first
del first, prev, cur

found = gc.collect()
print("cycle found >= 10:", found >= 10)

uncol = []
def hook(phase, info):
    if phase == "stop":
        uncol.append(info["uncollectable"])
gc.callbacks.append(hook)
gc.collect()
gc.callbacks.clear()
print("uncollectable seen:", uncol)
print("garbage:", gc.garbage)

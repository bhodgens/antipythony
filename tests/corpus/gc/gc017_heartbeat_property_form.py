# Cites: C-20, C-23, D1
# The C-20 CANONICAL divergent program, written as the property-style
# test the clause's Verify line demands: the three `make` lines and the
# final `loop done` line are byte-pinned (they do not depend on
# finalization timing); the three `del` lines are pinned only as a
# MULTISET sampled after an explicit gc.collect() placed before `loop
# done`, which per C-20 makes `make x3, del x3, loop done` deterministic.
# The interleaving of `del` vs `make`/`loop done` WITHOUT the collect is
# exactly the D1-unsanctioned byte-interleave diff and is deliberately
# NOT emitted (finalizer output goes to a list, not to stdout, until the
# collect-anchored projection print).
import gc

out = []

class Noisy:
    def __init__(self):
        print("make")
    def __del__(self):
        out.append("del")

for _ in range(3):
    Noisy()

gc.collect()
print("dels:", out.count("del"), "total:", len(out))
print("loop done")

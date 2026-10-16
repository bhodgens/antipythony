# Cites: C-19, C-21, D1
# gc.get_threshold()/set_threshold()/get_count()/freeze()/unfreeze()
# observation, all integer/API-shape behavior with no addresses.
# set_threshold returns None (verified 3.14.7); values round-trip.
# get_count() SHAPE is pinned; raw counter VALUES are not (allocation
# noise). After a full gc.collect() gen-0 is 0 — deterministic, pinned.
# Freeze (verified live on 3.14.7): gc.freeze() and gc.unfreeze() both
# return None. The between-states observation is deliberately NOT pinned:
# whether the frozen, name-dropped object is finalized at the first
# collect or only after unfreeze depends on the destruction model
# (CPython frees it via the refcount cascade when its gen-0 member list
# dies; a tracing GC only reclaims it once unfreeze returns it to a
# scanned generation). Both models MUST agree on the end state sampled
# after explicit collects: exactly one finalization (C-21) and gone from
# gc.get_objects() (C-19: refcount-introspection-free projection).
import gc

old = gc.get_threshold()
print("threshold len:", len(old))
got = gc.set_threshold(1000, 13, 17)
print("set_threshold returns:", got)
print("round trip:", gc.get_threshold() == (1000, 13, 17))
gc.set_threshold(*old)
print("restored:", gc.get_threshold() == old)

counts = gc.get_count()
print("get_count shape:", len(counts), all(isinstance(c, int) for c in counts))
gc.collect()
print("after collect gen0 is 0:", gc.get_count()[0] == 0)

fired = []

class Frz:
    def __del__(self):
        fired.append("freed")

f = Frz()
f.keep = []
rc1 = gc.freeze()
del f
gc.collect()          # intermediate state: NOT sampled (D1)
rc2 = gc.unfreeze()
gc.collect()          # end state: finalized by now under BOTH models (C-21)
print("freeze returns:", rc1)
print("unfreeze returns:", rc2)
print("finalized exactly once by end:", fired == ["freed"])
print("gone after unfreeze+collect:",
      sum(1 for o in gc.get_objects() if type(o) is Frz) == 0)

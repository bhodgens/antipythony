# Cites: C-20, C-21, C-23, D1
# gc.disable() stops automatic collection (gc.isenabled False) but an
# explicit gc.collect() STILL finalizes unreachable objects — the C-20
# forced-finalization guarantee is about collect(), not about scheduling.
# Timing-safe: the counter is sampled only after an explicit collect.
import gc

fired = []

class D:
    def __del__(self):
        fired.append("d")

gc.disable()
print("isenabled:", gc.isenabled())
d = D()
del d
gc.collect()
print("finalized despite disable:", fired)
print("still disabled:", gc.isenabled())
gc.enable()
print("re-enabled:", gc.isenabled())
gc.collect()
print("once only:", len(fired))

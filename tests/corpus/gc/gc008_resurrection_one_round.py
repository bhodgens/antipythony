# Cites: C-21, C-22, C-23, D1
# Finalizer resurrection: __del__ re-adopts its object exactly once. The
# resurrection keeps the weakref alive across the collect (C-22: callback
# MUST NOT fire for a reachable object — and the revived object is
# reachable again); after the revived reference is dropped and collected
# once more, the second destruction is SILENT: no double fire (C-21
# "zero or one times"), and the weakref callback lands only then.
import gc
import weakref

events = []
rescued = []

class R:
    def __del__(self):
        events.append("finalize")
        if len(events) == 1:
            rescued.append(self)

r = R()
wref = weakref.ref(r, lambda w: events.append("weakref-cb"))
del r

gc.collect()
print("round1 events:", events)
print("resurrected alive:", wref() is not None, rescued[0] is wref())

del rescued[0]
gc.collect()
print("round2 events:", events)
print("dead now:", wref() is None)
gc.collect()
print("still once only:", events.count("finalize"))

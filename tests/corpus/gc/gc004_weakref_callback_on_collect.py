# Cites: C-22, C-23, D1
# weakref callback timing: while the referent is reachable, 5 forced
# gc.collect() cycles MUST NOT fire the callback (C-22 "never a false
# collection"); after the last strong reference is dropped and a collect
# runs, ref() is None and the callback fired exactly once. GC-anchored
# observation only (C-23).
import gc
import weakref

fired = []

class W:
    pass

w = W()
ref = weakref.ref(w, lambda r: fired.append("cb"))
holder = [w]

for _ in range(5):
    gc.collect()
print("while reachable:", ref() is w, fired == [])

del w
del holder[0]
gc.collect()
print("after drop+collect:", ref() is None, fired)
gc.collect()
print("exactly once:", len(fired))

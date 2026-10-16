# Cites: C-21, C-22, C-23, D1
# weakref.finalize registration/invocation per C-22: fires at collection
# time (GC-anchored: observed only after explicit gc.collect()), alive
# bookkeeping flips, re-invoking the finalized object returns None, detach
# returns the (obj, func, args, kwargs) tuple and cancels the callback.
import gc
import weakref

calls = []

class F:
    pass

obj = F()
fin = weakref.finalize(obj, calls.append, "bye")
print("alive before:", fin.alive)

del obj
gc.collect()
print("after collect:", calls, "alive:", fin.alive)
print("reinvoke returns:", fin())

keep = F()
fin2 = weakref.finalize(keep, calls.append, "never")
det = fin2.detach()
print("detach fields:", len(det), det[0] is keep, fin2.alive)
print("detached invoke:", det[1](*det[2], **det[3]), calls)
del keep
gc.collect()
print("detached no callback:", calls)

# atexit path still runs (C-22): a finalize whose object survives to exit
survivor = F()
weakref.finalize(survivor, calls.append, "atexit")
print("pending at exit:", calls)

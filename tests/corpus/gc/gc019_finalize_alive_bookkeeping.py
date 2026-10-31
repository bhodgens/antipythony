# Cites: C-22, C-23, D1
# ISOLATED fence for the gc015 failure shape (issue #10): weakref.finalize
# alive bookkeeping. alive must be True at registration, flip False after
# the target dies + explicit gc.collect() (C-22 GC-anchored timing), stay
# readable after detach() cancels the finalizer, and reinvoke()/detach()
# observe exactly-once (None after the first fire). No addresses printed.
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

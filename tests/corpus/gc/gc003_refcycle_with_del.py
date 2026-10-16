# Cites: C-19, C-20, C-21, D1
# Reference cycle + __del__ (PEP 442 era): an object in a self-referential
# cycle whose last external name is unbound still gets exactly one __del__
# at the next explicit gc.collect(). No gc.garbage accumulation on 3.14.
import gc

log = []

class Node:
    def __init__(self, tag):
        self.tag = tag
    def __del__(self):
        log.append(self.tag)

a = Node("a")
b = Node("b")
a.peer = b
b.peer = a
a.self_ref = a
del a, b

gc.collect()
print("finalized:", sorted(log))
print("garbage list:", gc.garbage)
gc.collect()
print("no double fire:", len(log))

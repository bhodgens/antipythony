# Cites: C-21, C-23, S-06-17#1, D1
# Finalizer ordering WITHIN one explicit gc.collect() pass. C-23 allows
# "ordering after explicit gc.collect()" as an anchor; CPython 3.14.7 fires
# the five members of a single unreachable strongly-connected generation-0
# set in object creation order (verified deterministic across runs; the
# pytest fixture re-runs this case 3x and byte-compares). The ORDER ITSELF
# is a tie-break pin (C-02: reference behavior normative where unspecified),
# while the mere fact that all five fire before collect() returns is the
# C-21 guarantee.
import gc

events = []

class P:
    def __del__(self):
        events.append(self.name)

def make_chain(n):
    nodes = []
    for i in range(n):
        p = P()
        p.name = "p%d" % i
        nodes.append(p)
    for a, b in zip(nodes, nodes[1:]):
        a.nxt = b
        b.prv = a
    nodes[-1].nxt = nodes[0]
    nodes[0].prv = nodes[-1]

make_chain(5)
gc.collect()
print("all fired:", len(events))
print("order:", events)

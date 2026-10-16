# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-7
# Default expressions evaluated ONCE at def time, not per call: shared mutable default -> [1, 1] [1, 1].
def mk(seq=[]):
    seq.append(1)
    return seq
print(mk(), mk())

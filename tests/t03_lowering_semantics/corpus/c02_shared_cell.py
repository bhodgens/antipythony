# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-13
# Cites: S-02-3
# Cites: S-02-12
# Two closures over one variable share ONE cell: a nonlocal write in one is seen by the other.
def outer():
    n = 0
    def inc():
        nonlocal n
        n += 1
    def read():
        return n
    inc()
    inc()
    return read()
print(outer())

# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-3
# Cites: S-02-12
# Cites: S-02-14
# nonlocal counter across calls: 1 2 3 (S-02-3 Observed). Closure outlives defining frame (S-02-14).
def make_counter():
    count = 0
    def counter():
        nonlocal count
        count += 1
        return count
    return counter
c = make_counter()
print(c(), c(), c())

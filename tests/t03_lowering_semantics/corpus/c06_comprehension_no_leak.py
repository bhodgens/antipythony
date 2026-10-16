# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-5
# Comprehension iteration variable has its OWN implicit scope and does not leak to the enclosing scope.
xs = [1, 2, 3]
ys = [v * 2 for v in xs]
zs = [q + 1 for q in ys]
print(ys, zs)

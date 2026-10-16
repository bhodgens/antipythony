# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-12
# Cites: S-02-5
# Cites: S-02-1
# Simple closure late binding: [lambda: i for i in range(3)] called -> [2,2,2].
fns = [lambda: i for i in range(3)]
vals = [fn() for fn in fns]
print(vals)

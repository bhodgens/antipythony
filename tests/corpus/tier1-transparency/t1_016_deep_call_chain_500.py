# Cites: S-02-7 (def-time binding), S-02-10 (call binding errors), C-31 (tier modes invisible)
# Stresses: 500-deep chained Python calls — debug-build stack-overflow gate (leaf 02.05 T2).
# Each level adds one interpreter frame; passes only if the runtime survives depth 500.
def f(n):
    if n <= 0:
        return 0
    return 1 + f(n - 1)

print(f(500))

# T-05 values corpus V032
# Cites: S-01-33
# Pins: nan != nan (== is False) but containment finds nan via IDENTITY first
# in lists, sets and dicts (same object). hash(nan) is never printed:
# UNSPECIFIED per S-01-37 (deterministic-masking decision).
n = float("nan")
print(n == n, n != n, bool(n))
print(n in [n], n in [1, n], n in {n}, n in {n: 1})
d = {n: "nan-value"}
print(d[n])
print(n != 0.0, 0.0 == -0.0)

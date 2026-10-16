# Cites: S-01-1, S-01-2 (power rules, overflow-free), C-31
# Stresses: benchmark slice A (arithmetic) — growing-bignum multiply loop (arbitrary-precision hot path); speed-gate stub timing target.
# Benchmark slice A (arithmetic): repeated bignum growth then reduction; last 12 digits pinned.
n = 1
for i in range(1, 2000):
    n = n * (i % 97 + 2)
digits = str(n)
print(len(digits))
print(digits[-12:])

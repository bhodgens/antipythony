# Cites: S-01-1 (bigint arith), S-01-3/S-01-4 (floor-div/mod), S-02-17 (for), C-31 (byte-identity across modes)
# Stresses: benchmark slice A (arithmetic) — hot int add/sub/floordiv/mod sites; timed tier0-vs-adaptive by the T-08 speed-gate stub (C-31 also requires byte-identity across modes).
# Benchmark slice A (arithmetic): tight int +, -, //, % loop. Deterministic checksum output;
# the SAME file is also a cross-mode transparency case.
acc = 0
for i in range(1, 50000):
    acc = acc + i * 3
    acc = acc - (i // 7)
    acc = acc + (i % 13)
print(acc)

# Cites: S-01-6 (true division), S-01-7 (promotion), S-01-8 (IEEE-754 binary64), C-31
# Stresses: benchmark slice A (arithmetic) — hot float mul/div chain exercising IEEE-754 tier-1 float specialization; speed-gate stub timing target.
# Benchmark slice A (arithmetic): float mul/div chain; round(...,6) pins exact IEEE accumulation.
acc = 1.0
for i in range(1, 50000):
    acc = acc * 1.000001 + (i % 17) / 3.0
    acc = acc / 1.0000005
print(repr(round(acc, 6)))

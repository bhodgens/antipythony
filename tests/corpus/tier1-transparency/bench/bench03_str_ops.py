# Cites: S-01-14 (str), S-01-16 (f-strings), S-01-15 (slicing), C-31
# Stresses: benchmark slice B (string) — f-string build + join + upper over 20k parts; string-IC hot path; speed-gate stub timing target.
# Benchmark slice B (string): join/upper/f-string/slice mix; length + sampled slices pin content.
parts = []
for i in range(20000):
    parts.append(f"item-{i % 313}")
joined = "-".join(parts)
print(len(joined))
print(joined[:20])
print(joined.upper()[-20:])

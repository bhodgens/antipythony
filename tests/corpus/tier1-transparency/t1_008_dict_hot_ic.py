# Cites: S-01-21 (dict insertion order), S-01-23 (hash+eq collision buckets), C-31
# Stresses: dict get/setitem sites specialize; periodic re-read of the first 10 keys pins that
# insertion order is invariant across every tier transition.
d = {}
for i in range(30000):
    key = f"k{i % 500}"
    d[key] = d.get(key, 0) + i
    if i % 1500 == 0:
        for old in list(d)[:10]:
            d[old] = -d[old] if d[old] > 0 else d[old]
print(len(d))
print(list(d)[:10])
print(sum(d.values()))

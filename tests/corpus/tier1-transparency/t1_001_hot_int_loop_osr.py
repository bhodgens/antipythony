# Cites: S-01-1 (bigint exactness), S-02-16 (while), S-02-17 (for protocol), C-31 (tier modes invisible)
# Stresses: hot int-arithmetic loop -> OSR into tier-1 mid-loop; int-IC specialization of + and *.
# 50k iterations (tier-0 slow by design; budget cap per T-08 doc).
total = 0
prod = 1
for i in range(1, 50000):
    total = total + i * i
    if i % 4096 == 0:
        prod = prod * 3
print(total)
print(prod)

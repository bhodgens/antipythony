# T-01 valid fixture: assignment expressions.
# Cites: C-03 (3.14 grammar surface; walrus is 3.8+ and inside the pin).
values = [1, 5, 42, 7]
if (n := len(values)) > 3:
    big = n
while (m := max(values, default=0)) > 10:
    values.pop()
    if m < 0:
        break
sq = [y := i * i]
print(big, values, sq)

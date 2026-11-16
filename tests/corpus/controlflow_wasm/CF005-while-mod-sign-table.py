# Cites: S-01-4 S-01-3
# Pins: int % and // sign rules (sign of divisor / floor toward -inf)
# exercised from inside a loop body.
i = 0
acc = []
while i < 5:
    acc_i = (7 * i - 3) % 5
    i = i + 1
    print(acc_i, (7 * i) // 4)

# Cites: S-02-16 S-02-18
# Pins: pre-test while loop, accumulator rebinding across iterations
# (module-global writes visible to every read).
total = 0
i = 0
while i < 100:
    i = i + 1
    total = total + i
print(total, i)

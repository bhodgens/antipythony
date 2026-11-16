# Cites: S-02-18 S-02-16
# Pins: break exits the innermost loop and SKIPS its else clause;
# while True with a break-only exit.
t = 0
i = 0
while True:
    i = i + 1
    if i > 4:
        break
    t = t + i
else:
    t = -999
print(t, i)

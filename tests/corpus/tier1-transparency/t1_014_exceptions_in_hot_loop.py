# Cites: S-02-29 (message rules), S-02-30 (clause source order), S-02-31 (bare except), C-31
# Stresses: two exception kinds raised and caught 2k+ times inside a 20k tier-1 loop;
# the unwind-to-handler path must not corrupt loop-carried state (caught/kind are exact sums).
caught = 0
kind = 0
for i in range(20000):
    try:
        if i % 10 == 0:
            1 / 0
        if i % 13 == 0:
            raise LookupError("skip")
        v = int("42")
    except ZeroDivisionError:
        caught += 1
    except LookupError as e:
        caught += 2
        kind += len(str(e))
    else:
        kind += v % 7
print(caught, kind)

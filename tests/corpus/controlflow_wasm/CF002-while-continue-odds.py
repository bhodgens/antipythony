# Cites: S-02-18 S-02-16
# Pins: continue skips the remaining body (no accumulator update) but
# still re-tests the condition; def-local scope.
def f(n):
    total = 0
    i = 0
    while i < n:
        i = i + 1
        if i % 2 == 0:
            continue
        total = total + i
    return total
print(f(10), f(9), f(0), f(1))

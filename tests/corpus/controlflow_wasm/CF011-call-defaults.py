# Cites: S-02-7
# Pins: defaults evaluated at def time, used in order.
def g(a, b=10, c=100):
    return a + b + c
print(g(1), g(1, 2), g(1, 2, 3))

# Cites: S-07-24 S-02-8
# Pins: direct calls to module defs, positional binding, multiple args,
# return values flowing into expressions.
def add(a, b):
    return a + b
def wrap(v):
    return add(v, add(1, 2))
print(add(2, 3), wrap(10), add(add(1, 1), add(2, 2)))

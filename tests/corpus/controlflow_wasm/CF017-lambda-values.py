# Cites: S-02-12 S-02-38
# Pins: lambda bodies close over the enclosing scope; lambda in expression
# position calls like any function.
def compose(f, g, v):
    return f(g(v))
inc = lambda x: x + 1
dbl = lambda x: x * 2
print(inc(5), dbl(6), compose(inc, dbl, 10))
n = 100
print((lambda: n)())

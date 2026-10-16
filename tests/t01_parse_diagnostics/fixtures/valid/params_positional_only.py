# T-01 valid fixture: positional-only parameters (3.8+ grammar, inside C-03 pin).
# Cites: S-02-8 (parameter kinds); plan-01 01.03 names args incl. kwonly — positional-only is the mirrored kind.
def f(a, b, /, c, *, d):
    return a, b, c, d
g = lambda p, /, q: (p, q)
print(f(1, 2, 3, d=4), g(1, q=2))

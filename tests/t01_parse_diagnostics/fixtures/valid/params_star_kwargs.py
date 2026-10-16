# T-01 valid fixture: *args, **kwargs, keyword-only params, defaults.
# Cites: S-02-7/8 (defaults evaluated at def time; parameter kinds incl. kwonly).
def simple(*args, **kwargs):
    return args, kwargs
def mixed(a, b=2, *rest, c, d=4, **kw):
    return a, b, rest, c, d, kw
def kwonly(*, only):
    return only
vals = simple(1, 2, k=3)
print(vals, mixed(1, 2, 3, c=4), kwonly(only=5))

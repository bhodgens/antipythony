f = lambda a, b=2, *args, k=3, **kw: (a, b, args, k, kw)
g = lambda *, key: key
h = lambda a, /, b: (a, b)

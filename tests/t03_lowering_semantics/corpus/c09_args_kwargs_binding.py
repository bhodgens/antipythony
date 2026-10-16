# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-8
# Parameter kinds in order: positional-or-keyword, *args tuple, kwonly, **kwargs dict. f(1, 3, 5, c=7, e=8) -> (1, 3, (5,), 7, 4, ['e']).
def f(a, b=2, *args, c, d=4, **kw):
    return (a, b, args, c, d, sorted(kw))
print(f(1, 3, 5, c=7, e=8))

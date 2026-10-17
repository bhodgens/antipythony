# Cites: S-05-4 (row functools), anchor #8
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import functools
names = ["cache", "cached_property", "cmp_to_key", "get_cache_token",
         "lru_cache", "partial", "partialmethod", "recursive_repr", "reduce",
         "singledispatch", "singledispatchmethod", "total_ordering",
         "update_wrapper", "wraps", "WRAPPER_ASSIGNMENTS", "WRAPPER_UPDATES",
         "Placeholder", "RLock"]
print(len(names), all(hasattr(functools, n) for n in names))
print(functools.reduce(lambda a, b: a + b, [1, 2, 3]),
      functools.reduce(lambda a, b: a + b, [], 10),
      functools.reduce(max, [3, 9, 1]))
def double(x):
    return 2 * x
p = functools.partial(double, 3)
print(p(), p.func is double, p.args, p.keywords)
addk = functools.partial(lambda a, b=0: a + b, 1)
print(addk(), addk(b=5))
print(functools.Placeholder is not None)
h = functools.partial(lambda x, y: (x, y), functools.Placeholder, 2)
try:
    h("only-one")
except TypeError as e:
    print("placeholder TypeError:", str(e)[:58])
def base():
    "docstring"
    return 1
@functools.wraps(base)
def wrapped():
    return 2
print(wrapped.__name__, wrapped.__doc__, wrapped() == 2)
print(functools.WRAPPER_ASSIGNMENTS == ("__module__", "__name__",
      "__qualname__", "__doc__", "__dict__"))
class Widget:
    def __init__(self):
        self.n = 0
    def bump(self, amt=1):
        self.n += amt
    bump5 = functools.partialmethod(bump, 5)
w = Widget()
w.bump5()
w.bump(2)
print(w.n)
print(sorted([3, 1, 2], key=functools.cmp_to_key(lambda a, b: a - b)))
@functools.total_ordering
class T:
    def __init__(self, v):
        self.v = v
    def __eq__(self, o):
        return self.v == o.v
    def __lt__(self, o):
        return self.v < o.v
print(T(1) < T(2), T(2) >= T(1), T(2) > T(1), T(1) <= T(1))
@functools.singledispatch
def hand(x):
    return "base"
@hand.register(int)
def _(x):
    return "int"
@hand.register(str)
def _(x):
    return "str"
print(hand(1), hand("a"), hand(3.5))
print(functools.singledispatchmethod is not None)
class CP:
    calls = 0
    @functools.cached_property
    def v(self):
        CP.calls += 1
        return CP.calls
cp = CP()
print(cp.v, cp.v, CP.calls)
class Rec:
    def __init__(self):
        self.kids = []
    @functools.recursive_repr()
    def __repr__(self):
        return f"Rec({self.kids})"
r = Rec()
r.kids.append(r)
print(repr(r))
print(functools.RLock() is not None, callable(functools.get_cache_token))
print("CacheInfo" in dir(functools))

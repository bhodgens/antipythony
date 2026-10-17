# Cites: S-05-4 (row math), C-14, C-16, S-06-7 (single-line pins)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import math
def exc(fn, *a, **k):
    try:
        v = fn(*a, **k)
        return f"{fn.__name__}-value {v!r}"
    except Exception as e:
        return f"{fn.__name__} {type(e).__name__}: {e}"
print(exc(math.sqrt, -1.0))
print(exc(math.exp, 1000.0))
print(exc(math.log, 0))
print(exc(math.isqrt, -4))
print(exc(math.factorial, -1))
print(exc(math.comb, -1, 2))
print(exc(math.perm, 5, -1))
print(exc(math.log, -1))
print(exc(math.lgamma, 0))
print(exc(math.acos, 2))
print(exc(math.fma, float("inf"), 0.0, 0.0))
print(math.prod(iter([])), math.prod([0, 1, 2]), math.prod([], start=5),
      math.sumprod([1], [2]), math.dist([0, 0], [0, 0]))
try:
    math.sin("x")
except TypeError as e:
    print("sin-str TypeError:", e)
print(exc(math.nextafter, 1.0, float("nan")))
try:
    math.exp(10000)
except OverflowError as e:
    print(type(e).__name__ + ":", e)
print(float("1e999") == math.inf, repr(math.pow(2.0, 1023.0)))
print(exc(math.factorial, 3.5), exc(math.isqrt, 3.5), exc(math.gcd, 1.0, 2))

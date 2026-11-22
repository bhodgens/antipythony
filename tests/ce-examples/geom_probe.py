# geom_probe.py — the leaf 10.03 differential driver for the geom example
# extension. Run under BOTH antipythony native and CPython 3.14 with the
# extension importable from sys.path; stdout + rc must be byte-identical.
import sys

sys.path.insert(0, ".")
import geom

print(geom.add3(2, 40, 100))
print(geom.dist(0.0, 0.0, 3.0, 4.0))
print(geom.describe("beam", 42, 1.5))
print(geom.sum_tuple((1, 2, 3, 4)))
print(geom.make_list(4))
print(geom.make_dict(7, 8))
print(geom.is_even(10), geom.is_even(3))
print(geom.none_fn())
print(geom.div_or(50))
print(geom.div_or(7))

# FromSpec type: ctor -> getset -> methods -> C-raised ValueError.
s = geom.Segment(10, 5)
print(type(s).__name__)
print(s.len)
print(s.area())
print(s.twice(21))
try:
    s.check(5)
except ValueError as e:
    print("caught:", e)
s.check(50)
print("check-ok")

# Error taxonomy: ZeroDivisionError + OverflowError round-trips.
try:
    geom.div_or(0)
except ZeroDivisionError as e:
    print("zde:", e)
try:
    geom.div_or(-1)
except OverflowError as e:
    print("oe:", e)

# ParseTuple arity + type error texts (CPython-exact on the S0 surface).
for bad in [(1, 2), (1, 2, "x"), ()]:
    try:
        geom.add3(*bad)
    except TypeError as e:
        print("TypeError:", str(e)[:60])
try:
    geom.describe(1, 2, 3)
except TypeError as e:
    print("TypeError:", str(e)[:60])

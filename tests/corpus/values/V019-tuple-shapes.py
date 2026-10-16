# T-05 values corpus V019
# Cites: S-01-20
# Pins: tuple arity shapes — (1,) is a 1-tuple, (1) is the int 1, () is the
# empty tuple — display and unpacking order are left-to-right, immutability.
print(repr((1,)), repr((1)), repr(()), len((1,)), type((1)).__name__)
a, b, c = (1, 2, 3)
print(a, b, c)
first, *rest = (7, 8, 9)
print(first, rest)
try:
    t = (1, 2)
    t[0] = 9
except Exception as e:
    print(type(e).__name__ + ":", e)

# T-05 values corpus V017
# Cites: S-01-19
# Pins: list aliasing vs copying (ys = xs shares; + builds new; += mutates),
# element-wise recursive equality incl. numeric-typed elements, del, and the
# exact tuple item-deletion TypeError.
xs = [1, 2, 3]
ys = xs
ys.append(4)
print(xs, xs is ys)
zs = xs + [5]
print(xs, zs, xs == [1, 2, 3, 4, 5])
xs += [6]
print(xs, len(xs))
print([1, 2] == [True, 2.0], [1, 2] == (1, 2))
del xs[0]
print(xs)
try:
    t = (1, 2)
    del t[0]
except Exception as e:
    print(type(e).__name__ + ":", e)

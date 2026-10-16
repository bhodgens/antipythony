# T-01 valid fixture: starred unpacking targets, nested tuple/list targets.
# Cites: S-02-17 family (iterable unpacking protocol); plan-01 Stmt list names `assign` — target shapes are grammar surface.
first, *middle, last = [1, 2, 3, 4]
(a, (b, c)), d = (1, (2, 3)), 4
[e, *rest] = range(5)
x = y = z = 0
x, y = y, x
a += 1
b = [1, 2, 3]
b[0], *b[1:] = 9, 8, 7
print(middle, a, b, c, d, e, rest, x, y, z)

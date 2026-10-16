# T-01 valid fixture: list/dict/set comprehension kinds, nesting, conditions.
# Cites: S-02-5 (comprehensions have their OWN implicit scope); plan-01 Expr list `comprehension`.
xs = [1, 2, 3]
ys = [i for i in xs]
zs = [i * 2 for i in xs if i > 1]
m = {k: v for k, v in zip(xs, ys)}
setc = {i + j for i in xs for j in ys}
nested = [[i, j] for i in xs if i for j in xs]
print(ys, zs, m, setc, nested)

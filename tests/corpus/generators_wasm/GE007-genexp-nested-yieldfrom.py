# GE007: genexpr in nested clauses + yield-from over a genexp + sum() drain
# (Cites: S-02-20/21, S-02-26; exercises the resumable clause walker).
pair = [y for x in [(1, 2), (3, 4)] for y in x]
print(pair)

def inner():
    yield 10
    yield 20
    return 7

total = 0
for v in (x + 1 for x in inner()):
    total += v
    print('got', v)

n = sum(x for x in range(5))
print(n)

w = list(x * x for x in range(4))
print(w)
print(next(iter(w)))

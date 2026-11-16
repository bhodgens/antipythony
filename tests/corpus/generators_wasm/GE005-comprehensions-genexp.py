# GE005: generator expressions (lazy), comprehensions, next() defaults
squares = [x * x for x in range(5)]
print(squares)
d = {k: k * 2 for k in range(3)}
print(d)
evens = [x for x in range(10) if x % 2 == 0]
print(evens)
pair = [y for x in [(1, 2), (3, 4)] for y in x]
print(pair)
gen = (x for x in range(4))
print(next(gen))
print(list(gen))
w = [y := x * 10 for x in range(3)]
print(w, y)
n = sum(x for x in range(5))
print(n)

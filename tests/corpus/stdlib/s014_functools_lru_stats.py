# Cites: S-05-4 (row functools): lru_cache observable stats
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import functools
@functools.lru_cache(maxsize=2)
def fib(n):
    return n if n < 2 else fib(n - 1) + fib(n - 2)
print(fib(6))
print(fib.cache_info())
print(fib.cache_parameters())
for _ in range(4):
    fib(1)
print(fib.cache_info())
fib.cache_clear()
print(fib.cache_info())
@functools.lru_cache(maxsize=None, typed=True)
def g(x):
    return x
g(1)
g(1.0)
print(g.cache_info(), g.__wrapped__(3), g.cache_info().currsize == 2)
@functools.cache
def c(x):
    return x
c(1)
print(c.cache_info().currsize == 1,
      isinstance(g.cache_info(), tuple), g.cache_info()._fields)

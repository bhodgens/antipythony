# GE001: basic yield/next/exhaustion + for-over-generator (PEP-342 lazy)
def g(n):
    i = 0
    while i < n:
        yield i
        i = i + 1
    return 42

it = g(3)
print(next(it))
print(next(it))
print(next(it))
print(next(it, 'exhausted'))
for v in g(2):
    print('v', v)

def once():
    yield 1
    yield 2

e = once()
print(next(e), next(e))
print(next(e, 'x'))
e2 = once()
print(next(e2))
e2.close()
print(next(e2, 'after-close'))

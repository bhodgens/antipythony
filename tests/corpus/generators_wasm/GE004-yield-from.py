# GE004: yield from delegation + return-value plumbing
def sub():
    yield 1
    yield 2
    return 42

def delegate():
    r = yield from sub()
    print('sub returned', r)
    yield 99

for v in delegate():
    print('D', v)

def empty():
    return 7
    yield 0

def outer():
    v = yield from empty()
    print('empty gave', v)

o = outer()
try:
    next(o)
except StopIteration:
    print('outer exhausted right away')

def deep():
    yield from [10, 20]

print(list(deep()))

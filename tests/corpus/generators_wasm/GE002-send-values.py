# GE002: send() — first-send-None, value plumbing, yield-as-expression
def counter():
    x = 0
    while True:
        got = yield x
        if got is None:
            x = x + 1
        else:
            x = got
    return 0

c = counter()
print(c.send(None))
print(c.send(None))
print(c.send(10))
print(c.send(None))

def echo():
    total = 0
    while total < 100:
        total = total + (yield total)
    return total

d = echo()
print(d.send(None))
print(d.send(5))
print(d.send(7))
try:
    while True:
        d.send(1)
except StopIteration as e:
    print('SI value', e.value)

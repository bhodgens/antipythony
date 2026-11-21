# GE006: generator expression LAZY semantics (02.10 2a resumable frames).
# The genexp is a real suspended frame: produce-side effects run at pull
# time, send/throw/close act at the suspension point, close stops the
# stream (Cites: S-02-20/21, S-02-24/25; 02-10 design doc s3.0).
def f(x):
    print('produce', x)
    return x * 2

g = (f(i) for i in range(3))
print('created')
print(next(g))
print('after-first')
print(next(g))
print(next(g, 'exhausted'))

h = (x * 2 for x in range(3))
print(next(h))
print(h.send(99))

k = (x * 2 for x in range(3))
print(next(k))
try:
    k.throw(ValueError('boom'))
except ValueError as e:
    print('caught', e)
print('after', next(k, 'exh'))

m = (x for x in range(3))
print(next(m))
m.close()
print('closed', next(m, 'exh'))

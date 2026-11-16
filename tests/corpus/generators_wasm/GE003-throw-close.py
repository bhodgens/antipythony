# GE003: throw()/close() + try/except inside a generator body
def raiser():
    yield 1
    raise ValueError('mid')
    yield 2

it = raiser()
print(next(it))
try:
    next(it)
except ValueError as e:
    print('caught', e)

def closer():
    try:
        yield 1
        yield 2
    except GeneratorExit:
        print('closing')
        raise

c = closer()
print(next(c))
c.close()
print('closed ok')

def inbody():
    try:
        yield 0
        raise KeyError('boom')
    except KeyError:
        yield 'recovered'
    yield 'end'

for v in inbody():
    print('B', v)

def pre():
    yield 1

p = pre()
try:
    p.throw(ValueError('pre-start'))
except ValueError as e:
    print('pre-start throw', e)

q = pre()
q.close()
try:
    q.throw(ValueError('post-close'))
except ValueError as e:
    print('post-close throw re-raises', e)

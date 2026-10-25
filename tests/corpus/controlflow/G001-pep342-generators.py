# Cites: S-02-20 (lazy start), S-02-21 (send value at yield), S-02-24 (fresh
# send TypeError), S-02-22 (StopIteration.value), S-02-23 (PEP-479),
# S-02-25 (close + GeneratorExit + finally), S-02-26 (yield from delegation)
# PEP-342 differential: resumable generator frames. Every print is ordered
# against live CPython 3.14 (expected/ pinned by gen_expectations discipline).
def gen():
    x = 1
    print("start")
    for i in range(3):
        x += i
        r = yield x
        print("got", r, "x", x)
    return "done"

g = gen()
print("created")
print("next1", next(g))
print("send", g.send("a"))
print("next2", next(g))
try:
    next(g)
except StopIteration as e:
    print("stopit", e.value)
# fresh-generator send constraint (S-02-24)
def fresh():
    yield 1
f = fresh()
try:
    f.send("nope")
except TypeError as e:
    print("fresh-send:", e)
print(f.send(None))
# close(): finally runs, then next() raises StopIteration (S-02-25)
def closer():
    try:
        yield 1
        yield 2
    finally:
        print("cleanup")
c = closer()
print(next(c))
c.close()
try:
    next(c)
except StopIteration:
    print("stop-after-close")
# throw() into a started generator: raises at the suspension point
def throwee():
    try:
        yield 1
    except ValueError as e:
        print("caught-in-gen", e)
        yield 99
t = throwee()
print(next(t))
print("thrown->", t.throw(ValueError("boom")))
# throw() into an exhausted generator propagates
x = closer()
list(x)
try:
    x.throw(ValueError("late"))
except ValueError as e:
    print("exhausted-throw", e)
# unstarted throw propagates without running the body
u = closer()
try:
    u.throw(ValueError("early"))
except ValueError as e:
    print("unstarted-throw", e)
# PEP-479: StopIteration inside a body becomes RuntimeError (S-02-23).
# (CPython also chains the original as __cause__; v0 exception payloads do
# not surface __cause__ on instances — documented divergence, so the pin
# covers the RuntimeError message only.)
def pep479():
    yield 1
    raise StopIteration
p = pep479()
next(p)
try:
    next(p)
except RuntimeError as e:
    print("pep479", e)
# yield from: values, send relay, and the sub's return value (S-02-26)
def inner():
    got = yield 1
    print("inner-got", got)
    yield 2
    return "ret"
def outer():
    v = yield from inner()
    print("inner-ret", v)
    yield 99
o = outer()
print(next(o))
print(o.send("s"))
print(next(o))
try:
    next(o)
except StopIteration:
    print("outer-done")
# nested delegation through a plain iterator
def deleg():
    yield from range(3)
print(list(deleg()))
# yield inside for with break (frame suspend mid-loop)
def loopgen():
    for i in range(5):
        if i == 3:
            break
        yield i
    print("loop-end")
print(list(loopgen()))

# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-26
# yield from: sends relay into sub, sub's return value becomes the StopIteration.value of the delegating generator.
def inner():
    v = yield 1
    return v * 3
def deleg():
    r = yield from inner()
    return r
o = deleg()
print(next(o))
try:
    o.send(5)
except StopIteration as e:
    print("value", e.value)

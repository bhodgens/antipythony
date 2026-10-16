# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-20
# Cites: S-02-21
# Cites: S-02-22
# Generator: lazy start; yield-expression value comes from send; return sets StopIteration.value (anchor A02-05).
def g():
    x = yield 1
    return x + 1
gg = g()
print(next(gg))
try:
    gg.send(10)
except StopIteration as e:
    print("stop", e.value)

# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-23
# StopIteration raised INSIDE a generator body becomes RuntimeError('generator raised StopIteration') chained to __cause__ (anchor A02-06).
def g():
    yield next(iter([]))
try:
    list(g())
except RuntimeError as e:
    print(type(e).__name__, ":", e, "| cause:", type(e.__cause__).__name__)

# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-7
# def-statement execution order: default-value expressions first, decorators innermost-first, then the name binding — observed ['d1', 'deco:g']
calls = []
def deco(fn):
    calls.append("deco:" + fn.__name__)
    def wrapper(*a, **kw):
        calls.append("call:" + fn.__name__)
        return fn(*a, **kw)
    return wrapper
def mark(v):
    calls.append("d" + str(v))
    return v
@deco
@deco
def g(seq=mark(1)):
    return seq
print(g())
print(calls)

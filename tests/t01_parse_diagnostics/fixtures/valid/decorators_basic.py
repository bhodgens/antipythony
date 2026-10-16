# T-01 valid fixture: function/class decorators, stacked, parametrized, async def decorated.
# Cites: S-02-7 (def-time evaluation family covers decorator expression eval); plan-01 01.03 names `decorators`.
decl = []
def deco(fn):
    decl.append("simple")
    return fn
def deco_arg(n):
    def wrap(fn):
        decl.append(n)
        return fn
    return wrap
@deco
@deco_arg(2)
def f():
    return 1
@deco
class C:
    pass
@deco
async def g():
    return 2
print(decl)

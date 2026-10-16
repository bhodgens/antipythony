# T-01 valid fixture: async def / await / async with / async for.
# Cites: plan-01 Expr list names `await`; S-02-40/41 (with protocol; async variant same enter/exit shape); S-02-17 (for protocol). Tier T1 (scrapy) requires async shapes.
import contextlib
@contextlib.asynccontextmanager
async def cm():
    yield 1
async def inner():
    return 7
async def outer():
    a = await inner()
    async with cm() as v:
        b = v + a
    async for x in some_iter():
        b += x
    return b
print(outer)

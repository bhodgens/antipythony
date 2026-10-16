# T-01 valid fixture: async comprehension inside a coroutine.
# Cites: S-02-5 (comprehension scope); plan-01 Expr list names `await`/`async` shapes (tier T1 scrapy).
async def agen():
    for i in range(3):
        yield i
async def main():
    out = [i async for i in agen()]
    mapped = {i: i async for i in agen()}
    return out, mapped
print(main)

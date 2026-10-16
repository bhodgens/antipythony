async def collect(agen):
    results = [item async for item in agen if item is not None]
    paired = {i: v async for i, v in azip()}
    return results, paired

async def azip():
    yield 0, "zero"

async def stream_lines(reader):
    async for line in reader:
        yield line.strip()

class AsyncCtx:
    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False

async def guarded():
    async with AsyncCtx() as c:
        async for item in stream_lines(c):
            if item:
                return item
    return None

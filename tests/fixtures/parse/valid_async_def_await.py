async def fetch():
    return 42

async def main():
    value = await fetch()
    inner = await fetch_inner()
    return value + inner

async def fetch_inner():
    return 8

def inner():
    yield 1
    yield 2
    return "done"

def outer():
    result = yield from inner()
    yield result
    yield from (3, 4)

def repeat(n):
    def deco(fn):
        def wrapper(*a, **kw):
            return [fn(*a, **kw) for _ in range(n)]
        return wrapper
    return deco

@repeat(3)
def ping():
    return "pong"

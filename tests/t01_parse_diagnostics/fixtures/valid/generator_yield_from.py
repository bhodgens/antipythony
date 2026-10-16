# T-01 valid fixture: generator functions, yield, yield from, send-receiver forms.
# Cites: S-02-20/21 (generator functions, yield suspends); plan-01 Expr list names `yield`.
def gen():
    x = yield
    while x is not None:
        x = yield x * 2
def chain():
    yield from gen()
    yield -1
def squares(n):
    for i in range(n):
        yield i * i
g = chain()
print(list(squares(4)))

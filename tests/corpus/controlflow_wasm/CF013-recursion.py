# Cites: S-02-11 S-07-23
# Pins: recursion through the uniform convention (fresh cells/locals per
# call) and implicit None returns.
def fact(n):
    if n <= 1:
        return 1
    return n * fact(n - 1)
def fib(n):
    if n < 2:
        return n
    return fib(n - 1) + fib(n - 2)
print(fact(10), fib(15))

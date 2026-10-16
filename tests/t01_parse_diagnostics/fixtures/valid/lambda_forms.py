# T-01 valid fixture: lambda expressions with default args and closures.
# Cites: S-02-11 (lambda body is one expression); S-02-38 (`<lambda>` frame name — lambda is first-class); S-02-7 (defaults at def time apply to lambdas too).
add = lambda a, b=2: a + b
pick = lambda *, x, y=False: x if y else -x
closure = lambda n: (lambda m: n + m)
apply_all = [f(1) for f in (add, closure(10))]
print(add(1), pick(x=3), apply_all)

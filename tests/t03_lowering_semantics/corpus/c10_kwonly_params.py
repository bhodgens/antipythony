# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-8
# Cites: S-02-10
# Cites: S-02-3
# Keyword-only params after bare *, plus a closure cell inside the kwonly function.
def f(a, *, c, d=4):
    cell = a + c + d
    def inner():
        return cell
    return inner()
print(f(1, c=2))
print(f(1, c=2, d=10))

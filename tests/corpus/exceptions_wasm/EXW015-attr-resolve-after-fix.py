# Cites: S-03-9
# Pins: real attributes/methods still resolve after the W3 rewire (the
# method-call path re-stashes the receiver before FN_BOUND_OBJ — the
# interim build dispatched a garbage bound arg).
class A:
    def m(self):
        return 42

    v = 7

a = A()
print(a.m())
print(a.v)

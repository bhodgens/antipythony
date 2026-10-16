# Cites: S-03-18, S-03-30
# Canonical diamond chase: D().m() prints D B C A (anchor A8) — never B->A
# early; inherited code resolves super() against type(self).__mro__.
class A:
    def m(self):
        print("A")
class B(A):
    def m(self):
        print("B")
        super().m()
class C(A):
    def m(self):
        print("C")
        super().m()
class D(B, C):
    def m(self):
        print("D")
        super().m()
print([t.__name__ for t in D.__mro__])
D().m()
class E(D): pass
E().m()

# Cites: S-03-19
# super() in a classmethod body resolves with cls propagating: canonical
# spec example (parent returns cls.__name__) -> B->B. Third-level C keeps the
# prefix from B and the runtime cls: B->C.
class A:
    @classmethod
    def m(cls):
        return cls.__name__
class B(A):
    @classmethod
    def m(cls):
        return "B->" + super().m()
print(B.m())
class C(B): pass
print(C.m())

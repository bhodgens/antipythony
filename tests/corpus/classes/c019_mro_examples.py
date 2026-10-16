# Cites: S-03-15
# C3 worked examples E1/E2/E3 plus the accepted redundant ordering (S-03-16).
class O: pass
class B1(O): pass
class C1(O): pass
class D1(B1, C1): pass
print([t.__name__ for t in D1.__mro__])
class A2: pass
class B2(A2): pass
class C2(A2): pass
class D2(B2, C2): pass
class E2(D2, C2): pass
print([t.__name__ for t in E2.__mro__])
class A3: pass
class B3(A3): pass
class C3(A3): pass
class D3(A3): pass
class X3(B3, C3, D3): pass
print([t.__name__ for t in X3.__mro__])
class Redundant(B2, C2, A2): pass
print([t.__name__ for t in Redundant.__mro__])
print(D1.__mro__[-1] is object)

# Cites: S-03-13
# Slots instances have no __dict__; undeclared set/read messages (anchor A6).
class A:
    __slots__ = ("x",)
a = A()
a.x = 1
try:
    a.z = 2
except AttributeError as e:
    print("AttributeError:", e)
print(hasattr(a, "__dict__"), a.x)
try:
    a.z
except AttributeError as e:
    print("AttributeError:", e)
print(type(A.__dict__["x"]).__name__, type(A.__dict__["x"]).__class__.__name__)
print([t.__name__ for t in a.__class__.__mro__])

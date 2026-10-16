# Cites: S-03-14
# __dict__ requested as a slot member re-enables it; a subclass with no
# __slots__ gets a dict while parent slot storage stays reachable.
class A:
    __slots__ = ("x", "__dict__")
a = A()
a.x = 1
a.y = 2
print(a.x, sorted(vars(a).items()), hasattr(a, "__dict__"))
class B(A): pass
b = B()
b.x = 5
b.z = 6
print(b.x, sorted(vars(b).items()))
print("x" in vars(b), B.__dict__.get("__slots__"))

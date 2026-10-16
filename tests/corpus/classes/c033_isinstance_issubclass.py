# Cites: S-03-31
# isinstance/issubclass hierarchy rules, tuple and union second args, bad
# second args, custom metaclass __instancecheck__/__subclasscheck__ hooks.
class A: pass
class B(A): pass
print(isinstance(B(), A), isinstance(A(), B), issubclass(B, A), issubclass(A, B))
print(issubclass(A, A), issubclass(A, object), object.__bases__)
print(isinstance(True, int), (1).__class__ is int, B().__class__ is type(B()))
print(isinstance(B(), (str, B)), issubclass(B, (int, A)))
class U1: pass
class U2: pass
print(isinstance(U1(), U1 | U2), isinstance(5, U1 | U2))
try:
    isinstance(1, 2)
except TypeError as e:
    print("TypeError:", e)
try:
    issubclass(1, int)
except TypeError as e:
    print("TypeError:", e)
class WeirdM(type):
    def __instancecheck__(cls, obj):
        return obj == 42
class Weird(metaclass=WeirdM): pass
print(isinstance(42, Weird), isinstance(43, Weird))
class WeirdS(type):
    def __subclasscheck__(cls, sub):
        return sub is int
class WS(metaclass=WeirdS): pass
print(issubclass(int, WS), issubclass(str, WS))

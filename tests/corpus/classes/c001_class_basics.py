# Cites: S-03-01, S-03-04
# Class statement mechanics + class object attributes (anchor A1).
class A:
    x = 1
    def m(self): return 2
print(A.__bases__, issubclass(A, object))
print(A.__name__, A.__qualname__, A.__module__)
print(type(A) is type, type(A) is A.__class__)
print(A.__mro__[-1] is object, len(A.__mro__))
print(sorted(k for k in A.__dict__ if not k.startswith("_")))
class B(A): pass
print(B.__bases__ == (A,), B.x, B.m is not A.m or True)

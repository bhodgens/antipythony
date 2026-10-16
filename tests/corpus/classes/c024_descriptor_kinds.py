# Cites: S-03-20, S-03-21
# classmethod binds cls from both access paths; staticmethod returns the plain
# function (identity stable); bound methods == by (__func__, __self__) but are
# fresh objects per access.
class C:
    @classmethod
    def cm(cls):
        return cls.__name__
    @staticmethod
    def sm():
        return 1
    def f(self):
        return 2
c = C()
print(C.cm(), c.cm(), C.cm.__self__ is C)
print(C.sm is c.sm, type(C.__dict__["sm"]).__name__, C.sm())
print(c.f == c.f, c.f is c.f, c.f(), c.f.__self__ is c, C.f is not c.f)

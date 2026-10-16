# Cites: S-03-26, S-03-27, S-03-28
# Unary slots and __contains__-then-iteration; metaclass creation pipeline
# trace and __call__ wrapping every instantiation. NOTE (CONTRACT-GAP: T-06-
# GAP-1): reference 3.14.7 runs __set_name__ INSIDE type.__new__ (before
# M.__init__), not "then ... and B.__init_subclass__ run" afterwards as
# S-03-27 phrases it; order pinned here is the observed one.
class Un:
    def __neg__(self): return "neg"
    def __pos__(self): return "pos"
    def __invert__(self): return "inv"
    def __abs__(self): return "abs"
u = Un()
print(-u, +u, ~u, abs(u))
class Cont:
    def __contains__(self, item):
        return item == "yes"
print("yes" in Cont(), "no" in Cont())
class IterOnly:
    def __iter__(self):
        return iter([1, 2, 3])
print(2 in IterOnly(), 5 in IterOnly())
class Base: pass
class Meta(type):
    @classmethod
    def __prepare__(mcs, name, bases, tag=None, **kw):
        print("prepare", name, "tag=", tag)
        return {}
    def __new__(mcs, name, bases, ns, tag=None, **kw):
        print("new", name, "tag=", tag)
        return super().__new__(mcs, name, bases, ns)
    def __init__(cls, name, bases, ns, tag=None, **kw):
        print("init", name, "tag=", tag)
        super().__init__(name, bases, ns)
    def __call__(cls, *a, **k):
        print("call", cls.__name__, a, k)
        return super().__call__(*a, **k)
class Desc:
    def __set_name__(self, owner, name):
        print("set_name", owner.__name__, name)
    def __get__(self, obj, objtype=None):
        return "d"
class Traced(Base, metaclass=Meta, tag=3):
    d = Desc()
print(Traced().d)
t = Traced()
print(type(t) is Traced)

# Cites: S-03-28, S-03-29
# __init_subclass__ is an implicit classmethod receiving the class-statement
# kwargs (minus metaclass) even under a custom metaclass; the default
# object.__init_subclass__ rejects kwargs; an override consumes its own
# keyword-only args, forwards the rest, and Base.__subclasses__() then
# contains Sub. Metaclass __call__ wraps every instantiation.
class Base:
    def __init_subclass__(cls, tag, **kw):
        super().__init_subclass__(**kw)
        cls.tag = tag
class Sub(Base, tag=7): pass
print(Sub.tag, sorted(c.__name__ for c in Base.__subclasses__()))
class Plain: pass
try:
    class Bad(Plain, oops=1): pass
except TypeError as e:
    print("TypeError:", e)
class Meta(type):
    def __new__(mcs, name, bases, ns):
        return super().__new__(mcs, name, bases, ns)
    def __call__(cls, *a, **k):
        print("call", cls.__name__, a, k)
        return super().__call__(*a, **k)
class Widget(metaclass=Meta):
    def __init__(self, n=0):
        print("init", n)
    def __init_subclass__(cls, label, **kw):
        super().__init_subclass__(**kw)
        cls.label = label
w = Widget(3)
print(type(w) is Widget)
class Gadget(Widget, label="g"): pass
print(Gadget.label)

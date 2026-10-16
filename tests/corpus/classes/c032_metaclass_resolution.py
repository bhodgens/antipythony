# Cites: S-03-02
# Metaclass resolution: most-derived wins; explicit conflict raises TypeError;
# with zero bases an explicit non-type metaclass is used verbatim (anchor
# shape from the spec's observed line).
class MA(type): pass
class MB(MA): pass
class PB(metaclass=MB): pass
class C1(PB, metaclass=MA): pass
print(type(C1).__name__)
class MC(type): pass
class MD(type): pass
class PD(metaclass=MD): pass
try:
    class C2(PD, metaclass=MC): pass
except TypeError as e:
    print("TypeError:", e)
class NotAMeta: pass
try:
    class C3(metaclass=NotAMeta): pass
except TypeError as e:
    print("TypeError:", e)
class NoBases(metaclass=MA): pass
print(type(NoBases).__name__, NoBases.__bases__)

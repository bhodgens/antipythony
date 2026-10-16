# Cites: S-03-12
# C.name falls back to the metaclass MRO and metaclass instance dict; instances
# get NO metaclass fallback (anchor A5).
class M(type):
    mattr = 7
    def __getattr__(cls, name):
        return "M-got:" + name
class C(metaclass=M):
    pass
print(C.mattr)
print(C.whatever)
try:
    C().mattr
except AttributeError as e:
    print("AttributeError:", e)
class M2(M): pass
class C2(metaclass=M2): pass
print(C2.mattr, isinstance(C2, M), C2.__class__.__name__)

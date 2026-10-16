# Cites: S-03-15, S-03-16
# Duplicate bases fail during base processing; inconsistent linearizations
# fail with the MRO TypeError (anchor A9).
class DA: pass
try:
    class X1(DA, DA): pass
except TypeError as e:
    print("TypeError:", e)
class IA: pass
class IB(IA): pass
try:
    class X2(IA, IB): pass
except TypeError as e:
    print("TypeError:", e)
print("mro(object):", [t.__name__ for t in object.__mro__])

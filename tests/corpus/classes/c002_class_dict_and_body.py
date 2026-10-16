# Cites: S-03-03, S-03-04
# Body execution: class scope is not an enclosing scope; decorators bottom-up;
# __dict__ is a read-only mappingproxy; __qualname__ is a slot not a dict key.
order = []
def d1(f):
    order.append("d1")
    return f
def d2(f):
    order.append("d2")
    return f
class C:
    x = 10
    @d1
    @d2
    def m(self):
        return x
print(order)
try:
    C().m()
except NameError as e:
    print("NameError:", e)
try:
    C.__dict__["z"] = 1
except TypeError as e:
    print("TypeError:", e)
class Outer:
    class Inner: pass
print("__qualname__" in Outer.__dict__, Outer.Inner.__qualname__)

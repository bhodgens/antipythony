# Cites: S-03-09, S-03-20, S-03-21
# A data descriptor (property) shadows the instance dict (anchor A3).
class C:
    @property
    def x(self):
        return "prop"
c = C()
c.__dict__["x"] = 99
print(c.x)
print(type(C.__dict__["x"]).__name__, type(C.x).__name__, c.x)
class ND:
    def __get__(self, obj, objtype=None):
        return "nd"
class C2:
    x = ND()
c2 = C2()
c2.__dict__["x"] = 99
print(c2.x)
del c2.__dict__["x"]
print(c2.x)

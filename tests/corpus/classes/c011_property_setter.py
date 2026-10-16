# Cites: S-03-10, S-03-21
# property setter routes __setattr__; missing setter raises; the property
# object itself is writable on the class.
class C:
    _v = 0
    @property
    def x(self):
        return "get"
    @x.setter
    def x(self, v):
        self._stored = v * 2
c = C()
c.x = 5
print(c._stored, c.x)
class RO:
    @property
    def x(self):
        return 1
r = RO()
try:
    r.x = 2
except AttributeError as e:
    print("AttributeError:", e)
RO.x = 5
print(RO.x, r.__class__.__name__)

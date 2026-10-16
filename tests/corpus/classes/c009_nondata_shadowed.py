# Cites: S-03-09, S-03-10
# Non-data descriptor shadowed by instance dict; assignment does not consult
# the descriptor (anchor A4).
class ND:
    def __get__(self, obj, objtype=None):
        return "nd"
class C:
    x = ND()
c = C()
print(c.x)
c.x = 99
print(c.x, c.__dict__["x"] == 99)
c2 = C()
print(c2.x, "x" in c2.__dict__)

# Cites: S-03-4, S-03-10
# __init__ with 2 params storing two attrs; method call reads them.
class Pt:
    def __init__(self, a, b):
        self.a = a
        self.b = b

    def sum(self):
        return self.a + self.b

pt = Pt(3, 4)
print(pt.a, pt.b)
print(pt.sum())

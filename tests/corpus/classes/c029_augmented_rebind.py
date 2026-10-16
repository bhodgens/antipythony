# Cites: S-03-26
# __iX__ first; NotImplemented/absent falls back to the full binary machinery
# and REBINDS the target name; list += stays in-place.
class Aug:
    def __init__(self, v):
        self.v = v
    def __iadd__(self, o):
        print("iadd", o)
        return Aug(self.v + o)
    def __add__(self, o):
        print("add fallback", o)
        return Aug(self.v + o)
a = Aug(1)
a += 2
print(a.v)
b = Aug(1)
class Other: pass
b += Other()
print(b.v)
class AugN:
    def __iadd__(self, o):
        return NotImplemented
    def __add__(self, o):
        return "via-add"
an = AugN()
an += 1
print(an)
l1 = [1]
orig = l1
l1 += [2]
print(l1, l1 is orig)

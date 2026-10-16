# Cites: S-03-26
# __iX__ first; NotImplemented/absent falls back to the full binary machinery
# and REBINDS the target name; list += stays in-place.
class Aug:
    def __init__(self, v):
        self.v = v
    def __iadd__(self, o):
        print("iadd")
        return Aug(self.v + 1)
    def __add__(self, o):
        print("add fallback")
        return Aug(self.v + 1)
a = Aug(1)
a += 2
print(a.v)
b = Aug(1)
class AugN:
    def __iadd__(self, o):
        return NotImplemented
    def __add__(self, o):
        return "via-add"
an = AugN()
an += 1
print(an)
class AugNoI:
    def __add__(self, o):
        return "plain-add"
ao = AugNoI()
ao += 1
print(ao)
l1 = [1]
orig = l1
l1 += [2]
print(l1, l1 is orig)

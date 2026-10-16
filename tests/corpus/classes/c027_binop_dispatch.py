# Cites: S-03-25
# Primary/reflected ordering, proper-subclass reflection priority, both-
# NotImplemented TypeError, comparison failure message (anchor A11).
class L:
    def __add__(self, o):
        return "L.add"
class R(L):
    def __radd__(self, o):
        return "R.radd"
print(L() + R(), R() + L())
class N25:
    def __add__(self, o):
        return NotImplemented
class M25:
    def __radd__(self, o):
        return NotImplemented
try:
    N25() + M25()
except TypeError as e:
    print("TypeError:", e)
class A25: pass
try:
    1 < A25()
except TypeError as e:
    print("TypeError:", e)
try:
    1 + "x"
except TypeError as e:
    print("TypeError:", e)

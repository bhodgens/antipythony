# Cites: S-03-25
# NotImplemented defers (int.__eq__ defers to the reflected slot); != is the
# negation of whichever __eq__ ran; identity default when only __eq__ is
# defined elsewhere.
class REq:
    def __eq__(self, o):
        return "reflected-eq"
    __hash__ = None
print(1 == REq())
x = REq()
y = REq()
print(x != y, x == y)
class LEq:
    def __eq__(self, o):
        return "left-eq"
    __hash__ = None
class SEq(LEq):
    def __eq__(self, o):
        return "sub-eq"
    __hash__ = None
print(LEq() == SEq())
p = LEq()
q = p
print(p is q, p == q, p != q)

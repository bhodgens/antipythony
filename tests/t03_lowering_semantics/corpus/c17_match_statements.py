# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-15
# Cites: S-02-19
# Cites: S-01-34
# match statements: literal / capture / class / as-pattern cases with bindings; capture names are LOCAL to the enclosing function scope.
def classify(v):
    match v:
        case 0:
            out = "zero"
        case [first, *rest]:
            out = ("seq", first, rest)
        case {"key": val}:
            out = ("map", val)
        case Point(x=px, y=py) if px > 0:
            out = ("point-pos", px, py)
        case other:
            out = ("other", other)
    return out
class Point:
    def __init__(self, x, y):
        self.x = x
        self.y = y
print(classify(0), classify([1, 2, 3]), classify({"key": 9}), classify(Point(1, 2)), classify("s"))

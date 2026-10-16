# T-01 valid fixture: match class patterns, capture, guards.
# Cites: plan-01 Stmt list `match`; spec 04 S-04-05-style dotted names (class pattern `Point(x=0)`); guard grammar (3.10+).
class Point:
    def __init__(self, x=0, y=0):
        self.x = x
        self.y = y
p = Point(1, 4)
match p:
    case Point(x=0, y=0):
        kind = "origin"
    case Point(x=x, y=y) if x == y:
        kind = f"diagonal {x}"
    case Point(x=x, y=_) if x < y:
        kind = "below"
    case Point():
        kind = "other"
print(kind)

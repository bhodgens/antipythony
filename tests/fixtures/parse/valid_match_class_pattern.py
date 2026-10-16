class Point:
    __slots__ = ("x", "y")
    def __init__(self, x, y):
        self.x = x
        self.y = y

def quadrant(p):
    match p:
        case Point(x=0, y=0):
            return "origin"
        case Point(x=x, y=y) if x > 0 and y > 0:
            return "first"
        case Point():
            return "elsewhere"

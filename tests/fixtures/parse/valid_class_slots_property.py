class Vec:
    __slots__ = ("x", "y")

    def __init__(self, x, y):
        self.x = x
        self.y = y

    @property
    def magnitude(self):
        return (self.x ** 2 + self.y ** 2) ** 0.5

    @magnitude.setter
    def magnitude(self, m):
        self.x, self.y = m, 0

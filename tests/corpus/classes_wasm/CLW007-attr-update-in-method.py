# Cites: S-03-10
# Attribute REassignment inside a method (attr-assign target on self).
class Acc:
    def __init__(self):
        self.total = 0

    def add(self, n):
        self.total = self.total + n

a = Acc()
a.add(3)
a.add(4)
print(a.total)

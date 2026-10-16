# Cites: S-03-09 (getattr protocol), S-03-10 (setattr), C-31
# Stresses: one attribute read/write site alternates between a __dict__ class and a __slots__
# class (two storage shapes): polymorphic attribute-IC, guard failure + deopt path.
class A:
    def __init__(self):
        self.x = 10

class B:
    __slots__ = ("x",)
    def __init__(self):
        self.x = 20

a, b = A(), B()
total = 0
for i in range(30000):
    obj = a if i % 2 == 0 else b
    total += obj.x
    obj.x = obj.x + 1
print(total)
print(a.x + b.x)

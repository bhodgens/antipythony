# Cites: S-03-10
# attr-assign target in __init__ + instance attr read back (anchor slice).
class P:
    def __init__(self, v):
        self.x = v

p = P(5)
print(p.x)
p.x = 7
print(p.x)

# Cites: S-03-17
# Zero-argument super() in a plain method: the start class is the class where
# the method is defined and the object is the first parameter.
class A:
    def __init__(self):
        self.x = 1

class B(A):
    def __init__(self):
        super().__init__()

print(B().x)

# Cites: S-03-12
# type's own slots (__bases__) resolve before a user body entry of the same
# name: assigning __bases__ in the class body does not change the real bases.
class BP:
    __bases__ = (int,)
print(BP.__bases__)
print([t.__name__ for t in BP.__mro__])

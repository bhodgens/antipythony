# Cites: S-03-18, S-03-30
# A method that overrides its parent, next to one it inherits. The override
# reaches the parent method through zero-argument super() and returns both
# values; the inherited method reads the override through self. Methods only
# return values (no nested print, no str + str: separate base defects).
class A:
    def name(self):
        return 'A'

    def parent_of(self):
        return 'A.parent_of'

class B(A):
    def name(self):
        return (super().name(), 'B')

a = A()
b = B()
print(a.name())
print(b.name())
print(a.parent_of())
print(b.parent_of())
print(type(b.name()).__name__)

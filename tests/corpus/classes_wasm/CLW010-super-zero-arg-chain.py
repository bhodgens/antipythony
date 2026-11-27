# Cites: S-03-18
# Three-level chain through zero-argument super(): each method calls the next
# class after its own defining class in the MRO of the instance. The result of
# the parent call is returned on its own (one value per line).
class A:
    def m(self):
        return 'A'

class B(A):
    def m(self):
        r = super().m()
        print('B calls', r)
        return 'B'

class C(B):
    def m(self):
        r = super().m()
        print('C calls', r)
        return 'C'

print(C().m())
print(B().m())

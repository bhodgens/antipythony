# Cites: S-03-17
# super(C, obj).m() and C.m(obj): a method call through a super object and a
# call through the class object bind exactly one first argument.
class A:
    def __init__(self):
        self.x = 1

    def m(self):
        return 'A.m'

class B(A):
    def __init__(self):
        super(B, self).__init__()

    def m(self):
        return super(B, self).m()

b = B()
print(b.x, b.m())
print(A.m(b))

# Cites: S-06-3, S-03-09
# same shape through a method call in the argument list (defect 2).
class C:
    def meth(self):
        print('meth-inner')
        return 'M'
c = C()
print('A', c.meth())
print('done')

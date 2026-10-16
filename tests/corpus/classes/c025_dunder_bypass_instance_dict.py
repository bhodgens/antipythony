# Cites: S-03-22, S-03-23
# Implicit special-method lookup bypasses the instance dict (anchor A10).
# Default repr embeds a nondeterministic address, so it is masked to a shape
# check: class-name prefix + '>' suffix + str()==repr() (S-06-17 #4).
class C:
    def __len__(self):
        return 3
c = C()
c.__len__ = lambda: 99
print(len(c))
class A: pass
a = A()
a.__str__ = lambda: "inst-str"
a.__repr__ = lambda: "inst-repr"
print(str(a) == repr(a), repr(a).startswith("<__main__.A object at 0x"), repr(a).endswith(">"))
print(a.__str__(), a.__repr__())

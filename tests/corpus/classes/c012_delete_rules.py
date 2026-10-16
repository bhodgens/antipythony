# Cites: S-03-11, S-03-20
# del priority: data descriptor with __delete__ wins; else instance dict must
# have the name; __delattr__ override honored; read-only property has no deleter.
class DD:
    def __get__(self, obj, objtype=None):
        return "dd-get"
    def __set__(self, obj, v):
        obj.__dict__["_b"] = v
    def __delete__(self, obj):
        print("dd-delete")
class C:
    b = DD()
c = C()
del c.b
class DA:
    def __delattr__(self, name):
        print("delattr", name)
del DA().zzz
class RO:
    @property
    def x(self):
        return 1
try:
    del RO().x
except AttributeError as e:
    print("AttributeError:", e)
c2 = C()
c2.b = 3
print(c2.__dict__)

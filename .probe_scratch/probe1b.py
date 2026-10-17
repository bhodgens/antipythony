class Desc:
    def __set_name__(self, owner, name):
        print("set_name", name)

class M(type):
    def __new__(mcs, name, bases, ns):
        print("M.__new__: start")
        r = super().__new__(mcs, name, bases, ns)
        print("M.__new__: after super().__new__")
        return r
    def __init__(cls, name, bases, ns):
        print("M.__init__")
        super().__init__(name, bases, ns)

class B(metaclass=M):
    def __init_subclass__(cls, **k):
        print("B.__init_subclass__")
        super().__init_subclass__(**k)

class D(B, metaclass=M):
    d = Desc()
print("order check done")

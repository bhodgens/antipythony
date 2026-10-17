class M(type):
    def __call__(cls, *a, **k):
        print("M.__call__")
        return super().__call__(*a, **k)
    def __init__(cls, name, bases, ns):
        print("M.__init__")
        super().__init__(name, bases, ns)
    def __new__(mcs, name, bases, ns):
        print("M.__new__")
        return super().__new__(mcs, name, bases, ns)
class B(metaclass=M):
    def __init_subclass__(cls, **k):
        print("B.__init_subclass__")
        super().__init_subclass__(**k)
print("--- class D body executes, then hooks ---")
class D(B, metaclass=M):
    def __set_name__(self, owner, name):
        print("set_name", name)
    attr = 1
print("--- instantiation ---")
D()

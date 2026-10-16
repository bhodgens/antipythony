# Cites: S-03-06
# __new__ returning an instance of a SUBCLASS of cls still triggers __init__.
class A:
    def __new__(cls):
        print("new")
        return B() if cls is A else super().__new__(cls)
    def __init__(self):
        print("init", type(self).__name__)
class B(A): pass
A()

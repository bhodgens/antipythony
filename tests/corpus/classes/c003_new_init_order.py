# Cites: S-03-05, S-03-06
# type(C) -> type.__call__ -> __new__ -> (isinstance check) -> __init__.
# __new__ returning a NON-instance of cls skips __init__ (anchor A2).
class A:
    def __new__(cls):
        print("new")
        return "not an A"
    def __init__(self):
        print("init")
a = A()
print(a, type(a).__name__)
class Base:
    def __new__(cls):
        print("base new")
        return super().__new__(cls)
    def __init__(self):
        print("base init")
Base()

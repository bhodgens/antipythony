# Cites: S-03-07
# Arity cross-check on the no-custom-__new__ fast path.
class B: pass
try:
    B(1)
except TypeError as e:
    print("TypeError:", e)
class A7:
    def __new__(cls):
        return super().__new__(cls)
    def __init__(self, x):
        self.x = x
try:
    A7(1)
except TypeError as e:
    print("TypeError:", e)
try:
    A7()
except TypeError as e:
    print("TypeError:", e)

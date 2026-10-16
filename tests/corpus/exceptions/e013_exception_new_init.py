# Cites: S-03-33, S-03-06
# Exceptions are normal classes: __new__/__init__ protocol applies, custom
# __new__ can substitute, non-instance return skips __init__.
class E(Exception):
    def __new__(cls, *a, **k):
        print("new", cls.__name__)
        return super().__new__(cls, *a, **k)
    def __init__(self, msg):
        print("init", msg)
        super().__init__(msg)
try:
    raise E("boom")
except E as e:
    print("caught:", repr(e))
class SubE(E): pass
try:
    raise SubE("sub")
except E as e:
    print(type(e).__name__, repr(e))

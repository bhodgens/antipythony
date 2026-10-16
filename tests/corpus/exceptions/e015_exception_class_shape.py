# Cites: S-03-04, S-03-32, S-03-33
# Built-in exception classes are class objects with the S-03-04 shape
# (__name__/__qualname__/__module__/__bases__), and a raised user exception
# instance isinstance-matches its class (the S-03-33 catch rule uses the same
# hierarchy test as S-03-31).
class UserErr(Exception): pass
print(UserErr.__name__, UserErr.__qualname__, UserErr.__module__, UserErr.__bases__)
print(ValueError.__module__, OSError.__name__, StopIteration.__qualname__)
try:
    raise UserErr("u")
except Exception as e:
    print(isinstance(e, UserErr), e.__class__ is UserErr, type(e) is UserErr)
    print(type(e).__name__, str(e))
try:
    raise KeyError("k")
except LookupError as e:
    print(type(e).__name__, str(e), repr(e))

# Cites: S-03-32, S-03-33
# Built-in exception classes are ordinary class objects (S-03-01/04 shape):
# __name__/__qualname__/__module__, mro roots, and args on a fresh instance.
print(ValueError.__name__, ValueError.__qualname__, ValueError.__module__)
print([t.__name__ for t in BaseException.__mro__])
print(issubclass(ValueError, BaseException), type(ValueError) is type)
e = TypeError("x", "y")
print(e.args, str(e), repr(e))
print(Exception("only one").args)
print(repr(Exception()), str(Exception()) == "")
print(KeyError("kk").args)

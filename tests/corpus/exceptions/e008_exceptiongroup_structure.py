# Cites: S-03-32, S-03-33
# ExceptionGroup class structure: direct subclasses of Exception/BaseException
# per the tree, .message/.exceptions attributes, catch membership.
print(issubclass(ExceptionGroup, Exception), issubclass(BaseExceptionGroup, BaseException),
      issubclass(BaseExceptionGroup, Exception))
print([t.__name__ for t in ExceptionGroup.__mro__])
g = ExceptionGroup("boom", [ValueError("v"), KeyError("k")])
print(g.message, len(g.exceptions), [type(e).__name__ for e in g.exceptions], g.args)
nested = ExceptionGroup("outer", [g])
try:
    raise nested
except ExceptionGroup as e:
    print("caught outer:", e.message, type(e.exceptions[0]).__name__)
try:
    raise BaseExceptionGroup("b", [ValueError("x")])
except ValueError:
    print("unreached")
except BaseException as e:
    print("caught by BaseException:", type(e).__name__, e.message)

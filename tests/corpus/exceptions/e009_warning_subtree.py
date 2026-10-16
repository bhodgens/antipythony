# Cites: S-03-32
# Warning is a direct Exception subclass (per the observed sorted list);
# the built-in warning subtree membership.
print(issubclass(Warning, Exception))
print(sorted(t.__name__ for t in Warning.__subclasses__()))
print(issubclass(DeprecationWarning, Warning), issubclass(SyntaxWarning, Warning),
      issubclass(UserWarning, Warning))
print([t.__name__ for t in SyntaxWarning.__mro__])
w = SyntaxWarning("bad syntax-ish")
print(isinstance(w, Warning), isinstance(w, Exception), str(w), repr(w))
try:
    raise SyntaxWarning("as exception")
except Warning as e:
    print("caught by Warning:", type(e).__name__)

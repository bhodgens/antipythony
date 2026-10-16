# Cites: S-03-31, S-03-32
# The except-match rule is the isinstance rule on the class hierarchy:
# except LookupError catches KeyError/IndexError; except BaseException catches
# everything incl. SystemExit; ordering is first-match (S-02-30 cites the
# dispatch, the hierarchy side is spec 03).
print(isinstance(KeyError(), LookupError), issubclass(IndexError, LookupError))
class MyKLookup(LookupError): pass
try:
    raise MyKLookup("mk")
except LookupError as e:
    print("caught", type(e).__name__, str(e))
try:
    raise SystemExit(9)
except BaseException as e:
    print("BaseException catches SystemExit:", type(e).__name__)
caught = []
class E(Exception): pass
class F(E): pass
try:
    raise F()
except E:
    caught.append("E-first")
except F:
    caught.append("F")
print(caught)
# UnboundLocalError -> NameError chain (S-03-32 membership of the tree)
print(issubclass(UnboundLocalError, NameError), issubclass(ArithmeticError, Exception))
print(issubclass(StopIteration, Exception), issubclass(GeneratorExit, BaseException),
      issubclass(GeneratorExit, Exception))

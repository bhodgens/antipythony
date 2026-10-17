# Cites: S-05-4 (rows numbers, abc)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import abc
import numbers
names_n = ["Number", "Complex", "Real", "Rational", "Integral"]
names_a = ["ABC", "ABCMeta", "abstractmethod", "abstractclassmethod",
           "abstractstaticmethod", "abstractproperty", "get_cache_token",
           "update_abstractmethods"]
print(len(names_n), all(hasattr(numbers, n) for n in names_n))
print(all(hasattr(abc, n) for n in names_a))
print(isinstance(3, numbers.Integral), isinstance(3.0, numbers.Real),
      isinstance(3.0, numbers.Complex), isinstance(1 + 2j, numbers.Complex),
      isinstance(3, numbers.Real), isinstance(3, numbers.Number),
      isinstance(True, numbers.Integral))
print(issubclass(numbers.Integral, numbers.Real),
      issubclass(numbers.Real, numbers.Complex))
class A(abc.ABC):
    @abc.abstractmethod
    def foo(self):
        ...
try:
    A()
except TypeError as e:
    print("abc:", e)
class B(A):
    def foo(self):
        return "made"
print(B().foo(), isinstance(B, abc.ABCMeta))
t0 = abc.get_cache_token()
@abc.update_abstractmethods
class B2(B):
    pass
print(abc.get_cache_token() != t0, type(A.__abstractmethods__) is frozenset)
print(abc.ABC is not None,
      type(A.__dict__["foo"]).__name__ == "abstractmethod")

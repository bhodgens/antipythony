# Cites: S-05-4 (rows pprint, reprlib), C-26
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import pprint
import reprlib
names_p = ["pprint", "pformat", "PrettyPrinter", "saferepr", "isreadable",
           "isrecursive"]
names_r = ["Repr", "aRepr", "recursive_repr"]
print(all(hasattr(pprint, n) for n in names_p),
      all(hasattr(reprlib, n) for n in names_r))
obj = {"b": [1, {"deep": (1, 2)}], "a": ["x", None, True]}
print(pprint.pformat(obj))
pprint.pprint(obj, sort_dicts=False)
print(pprint.pformat([[i, i * i] for i in range(6)], width=24))
print(pprint.saferepr({1: 2}))
xs = []
xs.append(xs)
print(pprint.isrecursive(xs), pprint.isreadable([1, 2]))
print(repr(reprlib.aRepr.repr(list(range(100)))))
print(reprlib.aRepr.repr({("k%d" % i): i for i in range(10)}))
print(reprlib.aRepr.repr("x" * 50))
print(reprlib.aRepr.maxlevel, reprlib.aRepr.maxdict, reprlib.aRepr.maxlist,
      reprlib.aRepr.maxtuple, reprlib.aRepr.maxset, reprlib.aRepr.maxlong)
class MyR(reprlib.Repr):
    def repr_Circle(self, x, level):
        return "CIRCLE"
class Circle:
    pass
print(MyR().repr(Circle()))
print(sorted(k for k in vars(reprlib.aRepr) if k.startswith("max")))

# Cites: S-05-4 (row operator); S-05-3 native (no _operator exposure)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import operator
names = ["add", "and_", "attrgetter", "concat", "contains", "countOf",
         "delitem", "eq", "floordiv", "ge", "getitem", "gt", "iadd",
         "iand", "iconcat", "idiv", "ilshift", "imatmul", "imod", "imul",
         "index", "inv", "ior", "ipow", "irshift", "is_", "is_not", "isub",
         "itemgetter", "itruediv", "ixor", "le", "length_hint", "lshift",
         "lt", "matmul", "methodcaller", "mod", "mul", "ne", "neg", "not_",
         "or_", "pos", "pow", "rshift", "setitem", "sub", "truth",
         "truediv", "xor", "abs"]
present = [n for n in names if hasattr(operator, n)]
print(len(present) == len(names), len(names))
print(operator.add(1, 2), operator.sub(5, 1), operator.mul(3, 4),
      operator.truediv(7, 2), operator.floordiv(7, 2), operator.mod(7, 2),
      operator.pow(2, 10), "matmul-present" if hasattr(operator, "matmul") else "no")
print(operator.eq(1, 1), operator.ne(1, 2), operator.lt(1, 2),
      operator.le(2, 2), operator.gt(3, 2), operator.ge(3, 3))
print(operator.and_(0b1101, 0b0111), operator.or_(1, 2), operator.xor(5, 3),
      operator.lshift(1, 4), operator.rshift(16, 2))
print(operator.neg(-3), operator.pos(3),
      operator.abs(3) if hasattr(operator, "abs") else 3)
print(operator.contains([1, 2, 3], 2), operator.countOf([1, 1, 2], 1))
print(operator.truth(0), operator.truth([0]), operator.not_(0),
      operator.is_(None, None), operator.is_not(1, 2))
print(operator.length_hint(range(10**6)), operator.index(3))
ig = operator.itemgetter(1, 0)
print(ig(["a", "b"]), ig.__class__.__name__)
ag = operator.attrgetter("real")
print(ag(3 + 4j))
class Thing:
    def shout(self, n):
        return "A" * n
print(operator.methodcaller("shout", 3)(Thing()))
d = {"k": 1}
operator.setitem(d, "z", 2)
print(operator.getitem(d, "k"), d["z"])
l = [1, 2, 3]
operator.delitem(l, 0)
print(l)
ia = [1]
operator.iadd(ia, [2])
print(ia)
print(operator.abs(-7), sorted([3, 1, 2], key=operator.neg))
print(all(k in operator.__all__ for k in ("attrgetter", "itemgetter")),
      30 < len([n for n in dir(operator) if not n.startswith("_")]))
print(operator.inv if False else operator.not_(1),
      hasattr(operator, "invert") or hasattr(operator, "inv"))

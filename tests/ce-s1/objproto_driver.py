# Leaf 1005b differential driver: the object-protocol C-API band.
# Runs byte-identical under antipythony (objproto extension via build-ext)
# and CPython 3.14 (same source compiled against Python.h).
import objproto


class Box:
    """A plain class instance for the attr probes."""

    kind = "box"  # class attr

    def __init__(self, v):
        objproto.setattrstr_c(self, "v", v)

    def doubled(self):
        return objproto.getattr_c(self, "v") * 2


b = Box(21)

# ---- Attr: get / get-string / optional / set / set-string / del ----
print("A1", objproto.getattr_c(b, "v"))
print("A2", objproto.getattrstr_c(b, "kind"))
print("A3", objproto.getattr_opt_c(b, "v"))
print("A4", objproto.getattr_opt_c(b, "nope"))
b2 = Box(5)
objproto.setattr_c(b2, "v", 50)
print("A5", b2.v)
objproto.setattrstr_c(b2, "w", "str-val")
print("A6", b2.w)
objproto.delattr_c(b2, "w")
print("A7", "w" in dir(b2))


def expect_error(fn, *args):
    try:
        fn(*args)
    except Exception as e:  # noqa: BLE001 — the differential pin IS the text
        return f"{type(e).__name__}: {e}"
    return "NO-ERROR"


print("A8", expect_error(objproto.getattr_c, b, "missing"))
print("A9", expect_error(objproto.getattr_c, 5, "missing"))
print("A10", expect_error(objproto.getattrstr_c, b, "missing"))
print("A11", expect_error(objproto.getattr_c, b, 42))
print("A12", expect_error(objproto.setattr_c, 5, "q", 1))
print("A13", expect_error(objproto.delattr_c, 5, "q"))
print("A14", expect_error(objproto.delattr_c, b, "missing"))
print("A15", expect_error(objproto.getattr_c, None, "missing"))
print("A16", expect_error(objproto.getattr_opt_c, b, 42))

# ---- Length / LengthHint ----
print("L1", objproto.len_c([1, 2, 3]))
print("L2", objproto.len_c("abcd"))
print("L3", objproto.len_c({"k": 1}))
print("L4", expect_error(objproto.len_c, 5))
print("L5", objproto.lengthhint_c([1, 2], 99))
# L6 NOTE: CPython's list_iterator carries __length_hint__ (remaining
# count); our iterator-registry Seq state does not expose a remaining
# count to the C band yet (a registered ap-run export would be needed).
# A GENERATOR is hint-less on BOTH runtimes — pin the default through it
# (the length-hint FALLBACK contract) and disclose the iterator-hint gap.
def _g6():
    yield 1


print("L6", objproto.lengthhint_c(_g6(), 7))


class WithHint:
    def __length_hint__(self):
        return 30


print("L7", objproto.lengthhint_c(WithHint(), 4))


class LenFirst:
    def __len__(self):
        return 9

    def __length_hint__(self):
        return 2


print("L8", objproto.lengthhint_c(LenFirst(), 4))


class BadHint:
    def __length_hint__(self):
        return "x"


print("L9", expect_error(objproto.lengthhint_c, BadHint(), 4))


class NegHint:
    def __length_hint__(self):
        return -1


print("L10", expect_error(objproto.lengthhint_c, NegHint(), 4))
print("L11", objproto.lengthhint_c(object(), 123))


class BoomHint:
    def __length_hint__(self):
        raise ValueError("boom")


print("L12", expect_error(objproto.lengthhint_c, BoomHint(), 4))

# ---- RichCompare / IsTrue / Not ----
LT, LE, EQ, NE, GT, GE = 0, 1, 2, 3, 4, 5
print("R1", objproto.richcompare_c(1, 2, LT))
print("R2", objproto.richcompare_c(2, 2, EQ))
print("R3", objproto.richcompare_c("a", "b", LT))
print("R4", objproto.richcompare_c([1], (1,), EQ))
print("R5", expect_error(objproto.richcompare_c, 1, "a", LT))
print("R6", expect_error(objproto.richcompare_c, 1, "a", LE))
print("R7", expect_error(objproto.richcompare_c, 1, "a", GT))
print("R8", expect_error(objproto.richcompare_c, 1, "a", GE))


class NoCmp:
    pass


n1, n2 = NoCmp(), NoCmp()
print("R9", objproto.richcompare_c(n1, n2, EQ))
print("R10", expect_error(objproto.richcompare_c, n1, n2, LT))


class LtOnly:
    def __lt__(self, other):
        return True


print("R11", objproto.richcompare_c(LtOnly(), LtOnly(), GT))
print("R12", objproto.richcompare_c(LtOnly(), LtOnly(), LT))
print("R13", objproto.richcomparebool_c(1, 2, LT))
print("R14", objproto.richcomparebool_c(b"x", b"y", EQ))
x = object()
print("R15", objproto.richcomparebool_c(x, x, EQ))
print("R16", objproto.richcomparebool_c(x, x, NE))
nan = float("nan")
print("R17", objproto.richcompare_c(nan, nan, NE))
print("T1", objproto.istrue_c(0), objproto.istrue_c(3))
print("T2", objproto.istrue_c(""), objproto.istrue_c("x"))
print("T3", objproto.istrue_c(None), objproto.not_c(None))
print("T4", objproto.not_c([]), objproto.not_c([1]))


class Falsy:
    def __bool__(self):
        return False


print("T5", objproto.istrue_c(Falsy()), objproto.not_c(Falsy()))


class ZeroLen:
    def __len__(self):
        return 0


print("T6", objproto.istrue_c(ZeroLen()))

# ---- Iter: GetIter / IterNext over list, dict, generator, str ----
print("I1", objproto.iterdrain_c([10, 20, 30]))
print("I2", objproto.iterdrain_c((1, 2)))
print("I3", objproto.iterdrain_c("ab"))


def gen():
    yield "g1"
    yield "g2"


print("I4", objproto.iterdrain_c(gen()))
print("I5", objproto.iterdrain_c({"k1": 1, "k2": 2}.keys()))
it = objproto.getiter_c([1, 2])
print("I6", objproto.iternext_c(it), objproto.iternext_c(it), objproto.iternext_c(it))
print("I7", expect_error(objproto.getiter_c, 5))
print("I8", expect_error(objproto.iternext_c, 5))

# ---- Hash ----
print("H1", objproto.hash_c(42), objproto.hash_c(True), objproto.hash_c(False))
print("H2", objproto.hash_c("abc") == hash("abc"))
print("H3", objproto.hash_c(None) == hash(None))
print("H4", objproto.hash_c(2.5) == hash(2.5))
print("H5", expect_error(objproto.hash_c, [1]))
print("H6", expect_error(objproto.hash_c, {"k": 1}))
print("H7", objproto.hash_c((1, 2)) == hash((1, 2)))


class Unhashable:
    __hash__ = None


print("H8", expect_error(objproto.hash_c, Unhashable()))

import types  # noqa: E402 — mid-file import keeps the C surface above clean

mm = types.ModuleType("mmod")
mm.qq = 1

# ---- Dir ----
# NOTE (two documented gaps, both PRE-EXISTING at the runtime layer, not
# bridge bugs): (1) dir() over BUILTIN-type instances (str/int) is empty
# at the PYTHON level too (`dir("abc")` == [] under antipythony; CPython
# shows the type surface); (2) a USER-CLASS handle passed through C
# round-trips via the spec reverse-lookup (state.rs from_value) and a
# non-spec class pins to a Str — so dir_c(SomeClass) is a documented
# S1 miss (dir_c(instance) IS the working surface). The differential
# pins what BOTH runtimes provide: instance dir (MRO + attrs), module
# ns, sortedness.
d = objproto.dir_c(b)
print("D1", type(d).__name__, "doubled" in d, "v" in d)
print("D2", d == sorted(d))
dm = objproto.dir_c(mm)
print("D3", "qq" in dm, "__name__" in dm, dm == sorted(dm))


# ---- Call regression (10.04 surface still intact after the band) ----
print("C1", objproto.call1_c(lambda a: a * 3, 7))
print("C2", objproto.call1_c(str.upper, "shout"))

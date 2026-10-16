# T-05 values corpus V024
# Cites: S-01-26
# Pins: frozenset equality/hash is element-order-independent (in-process), a
# frozenset works as dict key and set element, empty/single-element reprs are
# stable (non-empty multi repr order UNSPECIFIED - masked, not printed).
fs1 = frozenset([1, 2])
fs2 = frozenset([2, 1])
print(fs1 == fs2, hash(fs1) == hash(fs2))
d = {fs1: "ok"}
print(d[fs2])
print(repr(frozenset()), repr(fs1.__class__ is frozenset))
big = frozenset([1, 2, 3])
print(len(big), 2 in big, sorted(big))
print(frozenset() in {frozenset()})

# T-05 values corpus V023
# Cites: S-01-25, C-26
# Pins: set equality/membership/operators are order-independent; MULTI-ELEMENT
# set order is UNSPECIFIED so every observable uses sorted() or a single-element
# repr (deterministic-masking decision). `{}` is a dict, empty set is set().
print({1, 2, 3} == {3, 2, 1}, 1 in {1.0}, {True} == {1})
s = {3, 1, 2}
print(sorted(s), len(s), repr({42}))
print(sorted({1, 2} | {2, 3}), sorted({1, 2, 3} - {1}), sorted({1, 2, 3} & {2, 3}), sorted({1, 2, 3} ^ {3, 4}))
print({1, 2} <= {1, 2}, {1} < {1, 2}, {1, 2} >= {2}, {1, 2} > {1, 2})
print(type({}).__name__, repr(set()))
s2 = set()
s2.add("x")
s2.discard("q")
print(sorted(s2))

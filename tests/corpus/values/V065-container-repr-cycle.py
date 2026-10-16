# T-05 values corpus V065
# Cites: S-01-44, S-01-23
# Pins: container repr recursion with CPython punctuation (`, ` separators,
# `{1: 'x'}` dict collision inside a list, 1-tuple comma), str(container) ==
# repr(container), all empty-container reprs, self-referential list ellipsis.
print(repr([1, "a", (2,), {1: None, True: "x"}]))
print(str([1, "a"]) == repr([1, "a"]))
print(repr([]), repr(()), repr({}), repr(set()), repr(frozenset()), repr(None), repr(True), repr(False))
xs = [1]
xs.append(xs)
print(repr(xs))
ys = {}
ys["self"] = ys
print(repr(ys))
t = (1,)
print(repr([t, (t,)]), repr({(1, 2): [3.0]}))

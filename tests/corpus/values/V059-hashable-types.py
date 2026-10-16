# T-05 values corpus V038
# Cites: S-01-39
# Pins: the v1 hashable set (int, bool, float, str, bytes, tuple-of-hashable,
# frozenset, range, class objects) all hash; list/dict/set raise the exact
# unhashable TypeError texts.
vals = (1, True, 1.5, "s", b"b", (1, 2), frozenset({1}), range(3), int)
print([isinstance(hash(v), int) for v in vals])
print(hash((1, 2)) == hash((1, 2)))
for label, thunk in (("list", lambda: hash([1, 2])), ("dict", lambda: hash({1: 2})), ("set", lambda: hash({1}))):
    try:
        thunk()
    except Exception as e:
        print(label, type(e).__name__ + ":", e)
try:
    hash((1, [2]))
except Exception as e:
    print("nested", type(e).__name__ + ":", e)

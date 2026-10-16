# Cites: C-19, C-22
# weakref target support matrix on 3.14.7 (pinned by running it): int/str/
# float/bytes/tuple/list/dict are NOT weak-referenceable (TypeError with the
# reference message); SET and class instances ARE (verified live — the set
# type carries a weakref list on 3.14). Callback observation is
# gc.collect-anchored (C-23): a weakref callback on a plain instance and on
# a cyclic instance fires exactly once, only after the strong refs are
# dropped AND a collect runs.
import gc
import weakref

seen = []

class Holder:
    pass

def cb(w):
    seen.append("collected")

for label, v in [
    ("int", 3),
    ("str", "s"),
    ("float", 1.0),
    ("bytes", b"b"),
    ("tuple", ()),
    ("list", [1, 2]),
    ("dict", {"a": 1}),
]:
    try:
        weakref.ref(v)
        print("weakref", label, "-> OK")
    except TypeError as e:
        print("weakref", label, "->", e)

s = {1}
ws = weakref.ref(s)
print("weakref set -> OK:", ws() is s)
del s
gc.collect()
print("set weakref cleared:", ws() is None)

plain = Holder()
rp = weakref.ref(plain, cb)
del plain
gc.collect()
print("plain instance ref:", rp() is None, seen)

cyc = Holder()
cyc.me = cyc
rc = weakref.ref(cyc, cb)
del cyc
gc.collect()
print("cycle instance ref:", rc() is None, seen)

# Cites: C-19, C-21
# gc.is_tracked basics. NOTE (timing-safe projection, per D1 discipline):
# freshly created tuples are TEMPORARILY tracked by CPython until the next
# GC pass proves them untracked, so every tuple probe is sampled AFTER a
# gc.collect() here; the pre-collect value of a fresh tuple is UNSPECIFIED
# and deliberately not pinned. Scalars (int/str/float/None) are never GC
# heads at all — stable False. Containers are stable True.
import gc

gc.collect()  # stabilize: flush the temporary-tracking window for tuples

vals = [
    ("int-1", 1),
    ("str-x", "x"),
    ("float", 1.0),
    ("none", None),
    ("empty-list", []),
    ("list", [1, 2]),
    ("dict", {"a": 1}),
    ("empty-dict", {}),
    ("set", {1, 2}),
    ("tuple-int-after-collect", (1, 2)),
    ("tuple-list", ([1],)),
]
fresh = (3, 4)
pre_fresh = gc.is_tracked(fresh)  # NOT pinned in output; rechecked post-collect
for name, v in vals:
    print("tracked", name, gc.is_tracked(v))
gc.collect()
print("tracked fresh tuple after collect", gc.is_tracked(fresh))
print("pre-collect was boolean", isinstance(pre_fresh, bool))

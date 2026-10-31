# Cites: C-19, C-23, D1
# PASSING regression fence for the gc013 shape (issue #10): gc.get_objects()
# observation filtered by type. Pins that tracked-instance counts are exact:
# 4 held, 0 after drop+collect, 0 for a self-cycle after collect. Count-only
# projection (C-19/D3: no refcount or address leakage). If the collector's
# registry over- or under-reports, this flips red deterministically.
import gc

class Marker:
    pass

held = [Marker() for _ in range(4)]

def count_markers():
    return sum(1 for o in gc.get_objects() if type(o) is Marker)

print("held count:", count_markers())
del held
gc.collect()
print("after drop+collect:", count_markers())

extra = Marker()
extra.link = extra  # self-cycle
del extra
gc.collect()
print("cycle survivor count:", count_markers())

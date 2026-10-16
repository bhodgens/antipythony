# Cites: C-19, C-21, D1
# gc.get_objects() observation WITHOUT address leakage: count only the
# live instances of our own class. Count is deterministic: the list is
# taken while a known set of instances is held alive, and after dropping
# them + collecting, their survivors are zero. No repr/id of any object.
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

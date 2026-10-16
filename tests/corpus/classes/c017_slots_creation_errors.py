# Cites: S-03-13
# Non-string slot element and slot/class-variable conflict fail at class
# creation.
try:
    class S1:
        __slots__ = (1,)
except TypeError as e:
    print("TypeError:", e)
try:
    class S2:
        x = 1
        __slots__ = ("x",)
except ValueError as e:
    print("ValueError:", e)
class StrSlots:
    __slots__ = "xy"
ss = StrSlots()
ss.x = 1
ss.y = 2
print(ss.x, ss.y, hasattr(ss, "__dict__"))

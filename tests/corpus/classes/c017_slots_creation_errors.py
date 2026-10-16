# Cites: S-03-13
# Non-string slot element and slot/class-variable conflict fail at class
# creation. NOTE (CONTRACT-GAP: T-06-GAP-6): a BARE string __slots__ ("xy")
# is treated by 3.14.7 as the single slot name "xy", NOT as an iterable of
# the chars 'x' and 'y' — spec text "non-empty iterable of strings" is
# ambiguous here. Pinned: bare-string slots create one slot named "xy"; the
# char names fail. (Recorded from 3.14.7; C-01 drift governs.)
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
ss.xy = 3
print(ss.xy, hasattr(ss, "__dict__"))
try:
    ss.x = 1
except AttributeError as e:
    print("AttributeError:", e)

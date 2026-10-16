# T-05 values corpus V027
# Cites: S-01-28
# Pins: bool(x) prefers __bool__, falls back to __len__ (nonzero -> truthy),
# else True; a __bool__ of True beats a __len__ of 0.
class LenZero:
    def __len__(self):
        return 0
class LenThree:
    def __len__(self):
        return 3
class BoolOverridesLen:
    def __len__(self):
        return 0
    def __bool__(self):
        return True
class Plain:
    pass
print(bool(LenZero()), bool(LenThree()), bool(BoolOverridesLen()), bool(Plain()))
print(bool([]), bool(len), bool(""))

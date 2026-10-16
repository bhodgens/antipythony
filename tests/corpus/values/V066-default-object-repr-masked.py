# T-05 values corpus V066
# Cites: S-01-45
# Pins: default instance repr has the stable prefix `<module.Class object at 0x`;
# the address digits are UNSPECIFIED so this case MASKS them (deterministic-
# masking decision: split at ' at 0x' and print only the prefix + 'MASKED').
# str(x) falls back to __repr__; when __str__ is defined but not __repr__ they
# diverge; containers recurse with repr, not str.
class Plain:
    pass
p = Plain()
r = repr(p)
head, sep, addr = r.partition(" object at 0x")
print(head, bool(sep))
print(str(p) == repr(p))
class OnlyStr:
    def __str__(self):
        return "STR"
print(str(OnlyStr()), repr(OnlyStr()).partition(" object at 0x")[0])
class Both:
    def __str__(self):
        return "STR"
    def __repr__(self):
        return "REPR"
print(f"{Both()}", repr(Both()), repr([Both()]), str(Both()))

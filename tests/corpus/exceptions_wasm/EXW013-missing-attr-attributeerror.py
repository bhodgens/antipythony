# Cites: S-03-09 S-07-28
# Pins: a missing attribute call raises catchable AttributeError (bughunt
# D12d — the null attr_get result trapped before).
class C:
    pass
c = C()
try:
    c.missing()
except AttributeError:
    print("AttributeError caught")

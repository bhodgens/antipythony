# T-05 values corpus V034
# Cites: S-01-35
# Pins: a == b implies hash(a) == hash(b) across the numeric tower in-process,
# including 0/-0.0/False all hashing to 0.
print(1 == 1.0 == True, hash(1) == hash(1.0), hash(1) == hash(True), hash(True) == 1)
print(hash(0) == 0, hash(0.0) == hash(0), hash(-0.0) == hash(0.0), hash(False) == hash(0))
print(hash(2) == hash(2.0), 2 == 2.0)

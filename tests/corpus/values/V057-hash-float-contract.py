# T-05 values corpus V036
# Cites: S-01-37
# Pins: integral floats hash as their int; 0.0/-0.0 both 0; hash(inf)==314159
# and hash(-inf)==-314159. hash(nan) is UNSPECIFIED and NEVER printed
# (deterministic-masking decision).
print(hash(2.0) == 2, hash(0.0) == 0, hash(-0.0) == 0)
print(hash(float("inf")), hash(float("-inf")))
print(hash(1e10) == hash(int(1e10)), hash(1e100) == hash(10**100))

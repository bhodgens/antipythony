import struct

# NaN payload canonicalization probe (leaf 04.02 pre-probe)
for b in [
    0x7FF8000000000000,
    0x7FF8000000000001,
    0x7FF4000000000001,
    0xFFFFFFFFFFFFFFFF,
    0x7FFFFFFFFFFFFFFF,
    0x7FF0000000000001,
    0x7FF8ABCDEF012345,
    0xFFF8000000000000,
]:
    f = struct.unpack(">d", struct.pack(">Q", b))[0]
    print(hex(b), repr(f), str(f) == repr(f))

print(repr(float("nan")), repr(float("-nan")), repr(float("inf")), repr(float("-inf")))

# batching sanity: repr of a list of floats via one subprocess
vals = [0.1, 1e16, -0.0, float("nan"), 5e-324]
print("|".join(repr(v) for v in vals))

# int repr spot: bigint + negative zero
print(repr(-(10**40)), repr(0), repr(-0), repr(10**25))

# T-05 values corpus V035
# Cites: S-01-36
# Pins: hash(int) == int for small ints; the -1 -> -2 exception; the
# 2**61-1 modulus wrap for large ints with the exact modulus value; results are
# never -1.
m = 2**61 - 1
print(hash(42) == 42, hash(-1), hash(0), hash(m - 1) == m - 1)
print(hash(10**100) == 10**100 % m, hash(-(10**100)) == -(10**100 % m))
print(hash(m), hash(-m))
import sys
print(sys.hash_info.modulus, sys.hash_info.width)
big = 2**64 + 12345
print(hash(big) == big % m, hash(-big) == -(big % m))

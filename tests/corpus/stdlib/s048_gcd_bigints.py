# Cites: S-05-4 (row math), C-12 (bigint args), C-15/C-16 pins
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import math
import sys
print(math.gcd(2**64, 2**32), math.lcm(2**32, 2**31))
print(math.comb(64, 32) == 1832624140942590534)
print(math.isqrt(10**40), math.isqrt(10**40 - 1) < 10**20)
print(math.prod([2] * 64) == 2**64)
print(repr(math.fsum([2.0**53, 1.0, -2.0**53])),
      repr(sum([2.0**53, 1.0, -2.0**53])))
print(repr(math.pi), repr(math.e), repr(math.sqrt(2)))
print(math.factorial(50) % 1000, math.dist([0, 0, 0], [1, 2, 2]))
print(math.perm(10, 3), math.comb(10, 3))
print(math.remainder(0.5, 1.0), math.remainder(1.5, 1.0),
      math.remainder(2.5, 1.0))
print(repr(math.nextafter(-1.0, 0.0)), repr(math.ulp(2.0)),
      repr(math.ulp(5e-324)))
print(math.isinf(1e308 * 10), math.exp2(10) == 2.0**10)
print(math.ldexp(0.5, 1023) == 2.0**1022,
      math.frexp(sys.float_info.max))
print(math.isqrt(2**1000) == 2**500, math.gcd(2**1000, 2**500) == 2**500)

# T-05 values corpus V061
# Cites: S-01-41, C-15, C-17
# Pins: shortest round-trip float repr byte-exact: fixed vs exponent thresholds
# (1e16 -> 1e+16, 1e15 -> 1000000000000000.0), two-digit exponents (1e-07),
# subnormal 5e-324, specials inf/-inf/nan, -0.0 keeps sign, always . or e.
print(repr(0.1), repr(1e16), repr(1e-7), repr(1.5e308), repr(float("inf")), repr(float("nan")), repr(-0.0))
print(repr(1.0), repr(1e15), repr(0.0001), repr(3.5e-05), repr(12345678.0), repr(9999999999999998.0))
print(repr(10000000000000001.0), repr(5e-324), repr(1.7976931348623157e308), repr(2.2250738585072014e-308), repr(float("-inf")))
print(repr(1 / 3), repr(10 / 3), repr(float("1e999")), repr(2.675), repr(123.456))
print(float(repr(0.1)) == 0.1, repr(float(repr(1 / 3))) == repr(1 / 3))

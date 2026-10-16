# T-05 values corpus V001
# Cites: S-01-1, C-12, C-13
# Pins: arbitrary-precision int: underscores, radix prefixes, arithmetic/bitops
# on huge values, ~x == -x-1, arithmetic right shift. No overflow anywhere.
print(1_000_000, 0b1010, 0o777, 0xDEAD, 0Xdead_beef)
n = 2**100 + 3
print(n, -n, n * n, n - n)
print(2**200 * 3**150 % 1_000_000_007)
x = 123456789012345678901234567890
print(x << 200 >> 200, ~x, ~0, ~(-1))
print((-7) >> 1, (-1) >> 63, x & 0xFF, x | 0, x ^ x)

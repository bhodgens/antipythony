# T-05 values corpus V002
# Cites: S-01-2, C-14
# Pins: int**int exponent rules (exact, 0**0==1, negative exponent -> float),
# ** precedence vs unary minus and right-associativity, ZeroDivisionError text
# for 0**-1, MemoryError (not OverflowError) for 1 << (2**62) per C-14.
print(2 ** 200)
print(0 ** 0, 2 ** -3, type(2 ** -3).__name__, (-2) ** 3)
print(-7 ** 2, 3 ** 3 ** 2, 2 ** 10 ** 2)
try:
    0 ** -1
except ZeroDivisionError as e:
    print(type(e).__name__ + ":", e)
try:
    1 << (2**62)
except Exception as e:
    print(type(e).__name__, repr(str(e)))

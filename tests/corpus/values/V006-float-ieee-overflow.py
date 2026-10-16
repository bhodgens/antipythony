# T-05 values corpus V006
# Cites: S-01-8, C-16
# Pins: IEEE-754 binary64 round-to-nearest-even (0.1+0.2), silent float overflow
# to inf (no exception), float modulo/division-by-zero error texts as actually
# measured on the reference (see CONTRACT-GAPS CG-1: C-16's quoted texts differ).
print(0.1 + 0.2, 1e308 * 10, type(1e308 * 10).__name__, -1e308 * 10)
print(float("1e999"), float("inf") - float("inf"), float("inf") + 1)
try:
    1.0 / 0.0
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    1.0 % 0.0
except Exception as e:
    print(type(e).__name__ + ":", e)

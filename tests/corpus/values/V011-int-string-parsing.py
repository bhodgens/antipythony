# T-05 values corpus V011
# Cites: S-01-13
# Pins: int(str) base-10 default with surrounding whitespace and optional sign;
# explicit bases 16/32/2/36 (case-insensitive); exact ValueError texts for
# int("12.5") and float("abc").
print(int("  42  "), int("+7"), int("-0007"), int("1_0", 2), int("ff", 16), int("FF", 16), int("-1b", 32), int("z", 36))
try:
    int("12.5")
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    float("abc")
except Exception as e:
    print(type(e).__name__ + ":", e)
print(float(" 1.5e3 "), float("-inf"), float("1_0.0"))

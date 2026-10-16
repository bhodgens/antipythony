# T-05 values corpus V015
# Cites: S-01-17
# Pins: str/bytes strict separation — both concat directions with their exact
# distinct texts, non-int multiplier text, and that * by int still works.
try:
    "x" + b"y"
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    b"y" + "x"
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    "x" * 1.5
except Exception as e:
    print(type(e).__name__ + ":", e)
print("x" * 3, b"y" * 2, "ab" in "xabz", b"ab" in b"xabz")

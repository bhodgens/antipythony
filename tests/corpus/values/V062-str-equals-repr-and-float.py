# T-05 values corpus V062
# Cites: S-01-41
# Pins: str(float) == repr(float) in 3.x (print emits repr), and print of a
# float is NOT the bare value if it needs repr layout.
x = 1e16
print(x, str(x) == repr(x))
print(0.1 + 0.2, repr(0.1 + 0.2), str(0.1 + 0.2) == repr(0.1 + 0.2))
print(-0.0, repr(-0.0))
print(f"{0.1 + 0.2}")

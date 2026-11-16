# Cites: S-02-32
# else is skipped when the try body raises; handler runs instead.
try:
    raise ValueError("x")
except ValueError:
    print("handler")
else:
    print("else skipped")

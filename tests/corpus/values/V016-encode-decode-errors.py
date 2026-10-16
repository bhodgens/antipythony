# T-05 values corpus V016
# Cites: S-01-18
# Pins: UTF-8 encode/decode round-trip (repr shows \xc3\xa9 bytes), default
# codecs are UTF-8, the three exact error texts for ascii-decode,
# surrogate-encode and unknown-codec, and bytes yielding ints.
print("café".encode("utf-8"), b"caf\xc3\xa9".decode("utf-8"), "café".encode(), b"Hi"[0], len("café".encode("utf-8")), len("café"))
try:
    b"\xff".decode("ascii")
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    "\ud800".encode("utf-8")
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    "x".encode("bogus-codec")
except Exception as e:
    print(type(e).__name__ + ":", e)

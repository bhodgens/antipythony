# T-05 values corpus V012
# Cites: S-01-14
# Pins: str is a sequence of CODE POINTS not graphemes - len() of precomposed
# "caf\u00e9" (NFC, 4 code points) vs "cafe"+COMBINING ACUTE U+0301 (NFD, 5
# code points); the two forms are NOT equal (normalization is never implicit).
# Item assignment, out-of-range index, and float-key index with the exact
# texts. The float index goes through a VARIABLE key: the literal form
# "abc"[1.5] makes CPython 3.14 emit a SyntaxWarning (missed-comma hint) on
# stderr, which would be parse-leaf noise inside this values slice.
nfc = "caf\u00e9"
nfd = "cafe\u0301"
print(len(nfc), len(nfd), nfc == nfd)
s = "abc"
try:
    s[0] = "z"
except Exception as e:
    print(type(e).__name__ + ":", e)
k = 5
try:
    s[k]
except Exception as e:
    print(type(e).__name__ + ":", e)
f = 1.5
try:
    s[f]
except Exception as e:
    print(type(e).__name__ + ":", e)

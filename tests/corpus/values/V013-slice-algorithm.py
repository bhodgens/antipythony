# T-05 values corpus V013
# Cites: S-01-15
# Pins: the full slice-index algorithm: negative bounds, silent clamping (never
# IndexError), step 0 -> exact ValueError, negative step backward iteration,
# same-type-out for str/list/tuple/bytes slices.
s = "abcdefgh"
print(s[1:4], s[-3:], s[::-1], repr(s[10:20]), repr(s[-100:100]))
print([0, 1, 2][5:], repr((1, 2, 3)[-1::-1]), b"abcdef"[1:5:2], "abc"[::2])
print(s[2:2], s[4:1], s[7:100])
try:
    s[::0]
except Exception as e:
    print(type(e).__name__ + ":", e)

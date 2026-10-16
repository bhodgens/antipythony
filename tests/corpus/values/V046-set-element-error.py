# T-05 values corpus V025
# Cites: S-01-26
# Pins: unhashable set DISPLAY element uses the set-element phrasing, distinct
# from the dict-key phrasing; set.add uses the plain unhashable text.
try:
    {[1]}
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    s = set()
    s.add([2])
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    frozenset([[3]])
except Exception as e:
    print(type(e).__name__ + ":", e)

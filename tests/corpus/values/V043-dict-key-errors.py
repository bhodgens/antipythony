# T-05 values corpus V022
# Cites: S-01-24
# Pins: unhashable dict DISPLAY key exact text (names the type twice), the
# lookup-subscript form, and KeyError whose str() is the repr of the key.
try:
    {[1]: 2}
except Exception as e:
    print(type(e).__name__ + ":", e)
d = {"a": 1}
try:
    d[[1, 2]]
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    d["zz"]
except KeyError as e:
    print(type(e).__name__ + ":", str(e), repr(e))
try:
    d[(1, [2])]
except Exception as e:
    print(type(e).__name__ + ":", e)

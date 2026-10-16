# T-05 values corpus V009
# Cites: S-01-11, C-14
# Pins: int(float) truncates toward zero incl. int(-0.0)==0; NaN -> ValueError
# and infinity -> OverflowError with exact texts; float(10**400) ->
# OverflowError: int too large to convert to float; 2**10000 does NOT raise (C-14).
print(int(3.7), int(-3.7), int(-0.0), repr(int(-0.0)))
try:
    int(float("nan"))
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    int(float("inf"))
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    float(10**400)
except Exception as e:
    print(type(e).__name__ + ":", e)
print(len(str(2**10000)), str(2**10000)[-5:])

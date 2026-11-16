# Cites: S-03-28, S-02-30
# except Exception catches a LookupError subclass (hierarchy match).
try:
    raise KeyError("k")
except Exception:
    print("base caught")

# Cites: S-02-30
# Multiple except clauses: first matching type wins.
try:
    raise TypeError("t")
except ValueError:
    print("value")
except TypeError:
    print("type")
except Exception:
    print("base")

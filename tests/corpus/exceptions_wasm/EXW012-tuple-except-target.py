# Cites: S-02-30 S-07-28
# Pins: `except (TypeError, KeyError)` matches EITHER element and the
# handler runs (bughunt D12c — the tuple target was an Unreachable trap).
# Message printing rides str(e) for TypeError only (the KeyError repr-quote
# surface is a separate exc_msg gap, not this leaf).
try:
    raise TypeError("t")
except (TypeError, KeyError):
    print("first caught TypeError")
try:
    raise KeyError("k")
except (TypeError, KeyError):
    print("second caught KeyError")
try:
    raise ValueError("v")
except (TypeError, KeyError):
    print("UNREACHABLE")
except ValueError:
    print("value falls to the second handler")

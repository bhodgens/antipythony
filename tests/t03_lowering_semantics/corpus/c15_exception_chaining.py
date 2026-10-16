# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-34
# Cites: S-02-35
# Implicit chaining (__context__, 'During handling...') and explicit raise X from Y (__cause__, 'direct cause...') — anchors A02-07/A02-08 forms.
try:
    try:
        1 / 0
    except ZeroDivisionError:
        int("nope")
except ValueError as outer_e:
    ctx = outer_e.__context__
    print("context:", type(ctx).__name__, "suppressed:", outer_e.__suppress_context__)
try:
    raise ValueError("bad") from TypeError("cause")
except ValueError as e2:
    print("cause:", type(e2.__cause__).__name__, "suppressed:", e2.__suppress_context__)

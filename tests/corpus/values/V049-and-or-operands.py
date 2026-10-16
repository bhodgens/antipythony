# T-05 values corpus V028
# Cites: S-01-29
# Pins: and/or return an OPERAND (never a coerced bool) and short-circuit: the
# side-effect probe must not run when the result is already decided.
print(3 and 5, 0 or "x", "" or [], 1 and 0, None or "d")
print([2] and 0.0, "a" and "b", {} or {"k": 1})
def side():
    print("SIDE")
    return 1
print(0 and side())
print(1 or side())
print(type(3 and 5).__name__, type(0 or "x").__name__)

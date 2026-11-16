# Cites: S-01-30
# Pins: and/or return the operand VALUE, not a bool; short-circuit order.
print(0 and 5, 3 and 5, 0 or 7, 3 or 7, '' or 'x', 'a' and 'b')
calls = []
def side(x):
    calls = calls
    return x
print(1 and 2 and 3, 0 and 99)

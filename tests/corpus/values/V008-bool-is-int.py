# T-05 values corpus V008
# Cites: S-01-9, S-01-10, S-01-32
# Pins: bool is a subclass of int (isinstance, equality, arithmetic results are
# int), type(True) is bool / type(1) is int, repr of True/False/None never 1/0,
# True is 1 is False while True == 1 is True. Variables (not literals) feed the
# `is` so no S-01-31 SyntaxWarning pollutes stderr; True and 1 are distinct
# objects in any case (S-01-32 pins this identity fact, not a caching detail).
print(isinstance(True, int), True == 1, False == 0, True + True, True * 3)
print(type(True).__name__, type(1).__name__, type(True + True).__name__)
print(repr(True), repr(False), repr(None), repr(None) == "None")
t = True
n = 1
print(t is n, t == n)

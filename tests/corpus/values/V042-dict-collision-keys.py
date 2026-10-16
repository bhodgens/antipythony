# T-05 values corpus V021
# Cites: S-01-23, S-01-35
# Pins: keys collide by hash+equality (1, 1.0, True are ONE key); re-assignment
# overwrites the value but keeps the FIRST insertion position and key object.
d = {1: "a", 1.0: "b", True: "c"}
print(repr(d), len(d))
e = {True: "t", 1: "i", 1.0: "f"}
print(repr(e), len(e))
f = {0: "z", False: "b0", 0.0: "fl"}
print(repr(f), len(f))

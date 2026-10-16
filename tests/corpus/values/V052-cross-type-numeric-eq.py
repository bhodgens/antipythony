# T-05 values corpus V031
# Cites: S-01-33
# Pins: cross-type numeric equality recurses structurally through containers;
# equality between unrelated types is False, never an error.
print(1 == 1.0 == True, 0 == -0.0)
print([True] == [1], (1, 2) == (True, 2.0), [1, [2.0]] == [True, [2]])
print(1 == "1", [] == "", "" == None, 0 == [], False == "")
print({"a": 1} == {"a": True}, 1 == 1 == 1)

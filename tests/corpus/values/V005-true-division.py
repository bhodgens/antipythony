# T-05 values corpus V005
# Cites: S-01-6, S-01-9
# Pins: / is always true division returning float even for exact quotients;
# bool arithmetic yields int (True+True==2) except `not` (returns bool).
print(6 / 3, 3 / 2, type(6 / 3).__name__, 7 / -2)
print(True + True, True * 3, type(True + True).__name__, type(not 0).__name__)

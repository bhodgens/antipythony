# Cites: S-01-27 (falsy table), S-01-28 (bool protocol), S-01-29 (and/or return operands), S-01-32 (None singleton identity), C-31 (tier modes invisible)
# Stresses: truthiness + short-circuit sites flip falsy/truthy and user-type (__bool__)
# operands every iteration: bool-IC guards churn; `is` on the sentinel pins operand identity
# after any deopt (identity, never id()).
class Sentinel:
    def __bool__(self):
        return False

SENT = Sentinel()
pick_a = 0
pick_b = 0
ident = 0
for i in range(30000):
    left = i if i % 2 else 0
    right = SENT if i % 3 == 0 else f"v{i % 11}"
    a = left or right
    b = left and right
    if a is SENT:
        ident += 1
    if isinstance(a, str):
        pick_a += 1
    if isinstance(b, int) and not isinstance(b, bool):
        pick_b += 1
print(pick_a, pick_b, ident)

# T-05 values corpus V004
# Cites: S-01-5
# Pins: divmod(a,b) == (a//b, a%b) with identical sign rules, including mixed
# int/float operands where the result tuple takes the promoted operand type.
print(divmod(7, -2), divmod(-7, 2), divmod(-7, -2), divmod(7, 2))
print(divmod(7.0, 2), divmod(7, 2.0), divmod(-7.0, 2.0))
ok = True
for a in range(-6, 7):
    for b in (-4, -3, 1, 2, 3):
        if divmod(a, b) != (a // b, a % b):
            ok = False
print("divmod-invariant", ok)

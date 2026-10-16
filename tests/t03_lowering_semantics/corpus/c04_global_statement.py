# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-02-3
# global makes all uses of the name refer to the module global.
x = 0
def bump():
    global x
    x += 1
    return x
print(bump(), x, bump(), x)

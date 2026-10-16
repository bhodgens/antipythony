# T-05 values corpus V003
# Cites: S-01-3, S-01-4, C-12
# Pins: complete // and % sign table for ints, float analogue (3.0 / 1.0),
# the fundamental invariant a == (a//b)*b + (a%b) across a sweep.
print(7 // 2, -7 // 2, 7 // -2, -7 // -2)
print(7 % 2, -7 % 2, 7 % -2, -7 % -2)
print(7.0 // 2.0, -7.0 // 2.0, -7.0 % 2.0, 7.0 % -2.0)
ok = True
for a in range(-9, 10):
    for b in (-5, -2, -1, 1, 2, 5):
        if a != (a // b) * b + (a % b):
            ok = False
print("invariant", ok)

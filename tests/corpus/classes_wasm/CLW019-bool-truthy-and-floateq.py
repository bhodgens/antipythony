# Cites: S-01-24 S-01-27
# Pins: a boxed False is falsy in if/not/bool() (D1); float==float compares
# payloads across DISTINCT objects incl. -0.0 == 0.0 true (D6).
b = False
if b:
    print("then")
else:
    print("else")
print(bool(b), not b)
x = 1.5
y = x + 0.0
print(x == y, 1.5 == 1.5, 1.5 != 1.5)
print(-0.0 == 0.0)

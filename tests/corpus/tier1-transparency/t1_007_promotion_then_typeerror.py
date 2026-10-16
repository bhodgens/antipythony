# Cites: S-01-6 (true division), S-01-7 (int/float promotion), S-01-8 (IEEE-754), S-02-29 (exception message rules), S-02-30 (except dispatch), C-31 (tier modes invisible)
# Stresses: a multiply site specializes on int*float for 30k calls, then receives a str:
# the specialization guard must fail into the reference TypeError message byte-exactly
# (post-deopt error path), and int/float results keep exact IEEE values.
def scale(v):
    return v * 2.5

acc = 0.0
for i in range(30000):
    acc += scale(i % 100)
print(repr(round(acc, 6)))
try:
    scale("abc")
except TypeError as e:
    print("TypeError:", e)
print(repr(scale(4)))

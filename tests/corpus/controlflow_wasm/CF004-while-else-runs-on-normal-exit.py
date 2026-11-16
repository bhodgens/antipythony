# Cites: S-02-16
# Pins: else runs iff the condition became false (zero-iteration loop too).
n = 3
while n:
    n = n - 1
else:
    print('else ran', n)
while 0:
    pass
else:
    print('zero-iter else ran')

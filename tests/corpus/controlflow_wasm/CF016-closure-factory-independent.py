# Cites: S-02-14 S-02-13
# Pins: each factory call produces independent cells; closures outlive the
# defining frame.
def adder(n):
    def add(x):
        return x + n
    return add
a5 = adder(5)
a7 = adder(7)
print(a5(1), a7(1), a5(a7(0)))

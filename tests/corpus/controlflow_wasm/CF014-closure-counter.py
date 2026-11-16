# Cites: S-02-12 S-02-13
# Pins: closures capture the VARIABLE (cell); writes via one closure are
# observed by the other; the factory's cell outlives its frame.
def make():
    count = 0
    def inc():
        nonlocal count
        count = count + 1
        return count
    def get():
        return count
    return inc
inc = make()
inc2 = make()
print(inc(), inc(), inc())
print(inc2(), inc())

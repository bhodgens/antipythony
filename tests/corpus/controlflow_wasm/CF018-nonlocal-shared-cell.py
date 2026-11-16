# Cites: S-02-13 S-02-3
# Pins: explicit nonlocal writes share ONE cell across all closures — the
# reader observes the setter's write and the bump's addition (42 + 8).
def pair():
    v = 0
    def setter(x):
        nonlocal v
        v = x
    def bump():
        nonlocal v
        v = v + 8
    def reader():
        return v
    setter(42)
    bump()
    return reader
print(pair()())

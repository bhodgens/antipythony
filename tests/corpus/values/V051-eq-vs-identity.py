# T-05 values corpus V030
# Cites: S-01-30, S-01-31, S-01-32
# Pins: == is by value; `is` is used ONLY on None / sentinels / a name compared
# to itself (small-int caching and interning are UNSPECIFIED per S-01-31, so no
# cross-object `is` is asserted); None is the singleton identity test; None
# compares unequal to False and 0.
a = [1, 2]
b = a
print(a == [1, 2], a is a, a is b)
x = None
print(x is None, x == None)
print(None is None, None == False, None == 0)
s = "sentinel-value"
print(s is s)

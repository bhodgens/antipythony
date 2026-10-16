# T-05 values corpus V039
# Cites: S-01-40, S-01-1
# Pins: repr(int) is bare decimal digits: no plus, no leading zeros, no
# separators, minus only when negative; repr(-0)==0; str(int)==repr(int);
# grouping format does not change the value.
print(repr(-5), repr(10**25), repr(0), repr(-0), repr(2))
print(str(42) == repr(42), repr(1_000_000))
print(format(10**20, ","), format(-1234, "+,d"), format(10**50, "d") == str(10**50))
print(f"{10**40}", repr(-(10**25)))

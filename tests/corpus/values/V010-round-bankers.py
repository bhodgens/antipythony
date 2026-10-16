# T-05 values corpus V010
# Cites: S-01-12, S-06-18
# Pins: banker's rounding (ties to even) at the binary64 level, including the
# stored-below-tie cases round(2.675, 2)==2.67 and round(0.125, 2)==0.12;
# no ndigits -> int, with ndigits -> float.
print(round(2.5), round(3.5), round(-2.5), round(1.5), round(0.5))
print(round(2.675, 2), round(0.125, 2), repr(round(1.5, 0)))
print(type(round(2.5)).__name__, type(round(2.5, 1)).__name__)
print(round(7 / 3, 4), round(10**20 + 0.0), repr(round(-0.0)))

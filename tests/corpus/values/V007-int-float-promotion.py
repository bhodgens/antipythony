# T-05 values corpus V007
# Cites: S-01-7, C-12
# Pins: mixed int/float arithmetic promotes to float with exact conversion of
# the int operand (no double rounding); type names printed so promotion is
# observable in bytes.
print(1 + 2.0, type(1 + 2.0).__name__)
print(3 * 0.5, type(3 - 1.0).__name__, type(3 / 1).__name__)
print(2**53 + 1 == 2**53 + 1, float(2**53) + 1 == float(2**53))
x = 1
x += 0.5
print(x, type(x).__name__)

# T-05 values corpus V068
# Cites: C-18, S-01-16
# Pins: float format specs through format()/f-strings are byte-exact: e, .6f,
# >12.4g, n, 010.2%, .0f over the C-15 boundary value set (matrix printed
# deterministically: fixed value list x fixed spec list, one line per pair).
values = [0.0, -0.0, 5e-324, 1e-16, 1e15, 1e16, 1e17, 123456789012345.0, 1 / 3, 2 / 3, float.fromhex("0x1.0p+53")]
specs = ["", "e", ".6f", ">12.4g", "n", "010.2%", ".0f"]
for v in values:
    print(" | ".join(format(v, sp) for sp in specs))
print(f"{1e16:e}", f"{0.1:.2f}", f"{1/3:>8.3f}", f"{0.25:%}")

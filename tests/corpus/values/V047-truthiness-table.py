# T-05 values corpus V026
# Cites: S-01-27
# Pins: the exhaustive falsy table - False, None, every numeric zero, every
# empty collection - and that "0"/[0]/(None,)/nan are truthy.
print(bool([]), bool(""), bool(0), bool(0.0), bool(None), bool(()), bool({}), bool("0"), bool([0]))
print(bool(set()), bool(frozenset()), bool(b""), bool(0j), bool(-0.0), bool(-0))
print(bool((None,)), bool(float("nan")), bool([None]), bool(" "), bool({"a": 0}))
print(bool(1e-324), bool(0.1))

# T-05 values corpus V020
# Cites: S-01-22
# Pins: dict equality ignores insertion order but requires same length and
# == values for every key.
print({"a": 1, "b": 2} == {"b": 2, "a": 1})
print({"a": 1} == {"a": 1, "b": 2}, {"a": 1} == {"a": 2})
print(len({"a": 1, "b": 2}), "b" in {"a": 1, "b": 2})
print({"a": 1, "b": 2} != {"b": 2, "a": 1})

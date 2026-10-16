# T-05 values corpus V018
# Cites: S-01-21
# Pins: dict insertion order across iteration, keys/values/items, list(d) and
# repr(d); re-assigning an existing key keeps its ORIGINAL position.
d = {"b": 1, "a": 2}
d["b"] = 3
print(list(d), d["b"])
print(list(d.keys()), list(d.values()), list(d.items()))
print(repr({"z": 0, "m": 1, "a": 2}))
e = {}
e["x"] = 1
e["y"] = 2
e["x"] = 9
print(repr(e))

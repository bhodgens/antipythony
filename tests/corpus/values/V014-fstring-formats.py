# T-05 values corpus V014
# Cites: S-01-16
# Pins: f-string conversion flags !s/!r/!a, format specs (pad, zero-pad, comma,
# .2f), the debug form printing `expr=value` (source text, not just name),
# nested same-type quotes, nested f-strings and conditional expressions.
name = "world"
x = 3.256
v = 5
print(f"{name}, {x:.2f}, {2 + 3}, {x!r}")
print(f"{42:>6}|{42:06}|{1234567:,}")
print(f"{v=}", f"{1 + 2=}", f"{name=}")
d = {"k": 7}
print(f'{d["k"]}')
print(f"{'a' if v else 'b'}|{f'{x!a}'}|{[i * i for i in range(3)]}")
print(f"{v=!r}")

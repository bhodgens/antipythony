# T-01 valid fixture: nested quotes inside f-string expressions.
# Cites: S-01-16 (task-named grammar: f-string nested quotes). PEP 701 is 3.12+ grammar, in the 3.14 pin.
d = {"key": "value"}
a = f'{d["key"]}'
b = f"{d['key']}"
c = f"""{d["key"]} and {f"{d[chr(107)]}"}"""
e = f"{ "literal in expr" }"
print(a, b, c, e)

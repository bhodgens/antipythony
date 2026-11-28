# Cites: S-01-31 S-02-11
# Pins: str-in-str substring membership (audit W2 — the $str container
# RefCast to $tuple trapped); '' in s True / x in '' False; dict/set `in`
# unchanged; list/tuple IN unchanged (CF020's surface).
print('a' in 'abc', 'z' not in 'abc')
print('' in 'abc', 'x' in '', 'ab' in 'abc', 'ac' in 'abc')
print('key' in {'key': 1})
print(2 in [1, 2], 3 not in (1, 2))

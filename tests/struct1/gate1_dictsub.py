# F-STRUCT-1 chain probes — gate-1 scope note.
# dict-subclass __eq__/__repr__ staying identity-based is PRE-EXISTING on
# main (verified: D({'a':1}) == {'a':1} is False on main too). Not gated here.
class D(dict):
    pass


d1 = D({'a': 1})
print("LEN", len(d1) == 1)
print("GET", d1['a'] == 1)
print("DICT-ROUNDTRIP", dict(d1) == {'a': 1})
d2 = D(a=1)
print("KW-LEN", len(d2) == 1)
print("KW-GET", d2['a'] == 1)
print("KW-DICT-ROUNDTRIP", dict(d2) == {'a': 1})

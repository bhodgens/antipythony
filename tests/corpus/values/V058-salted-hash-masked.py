# T-05 values corpus V037
# Cites: S-01-38, S-06-16
# Pins: str/bytes hashes are salted per process and MUST NOT be printed as
# bytes (S-01-38 UNSPECIFIED) - deterministic-masking decision. Only the
# in-process contract is observable: equal strings/bytes hash equal and keyed
# containers behave consistently.
print(hash("abcdef") == hash("abcdef"), hash(b"xyz") == hash(b"xyz"))
d = {"m": 1, "a": 2, "z": 3}
print(list(d), d["a"], len(d))
s = {"p", "q", "r"}
print("p" in s, "r" in s, "z" in s, len(s))
print(hash("") == hash(""))

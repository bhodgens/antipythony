# T-01 valid fixture: f-string debug form `=`.
# Cites: S-01-16 (f-string debug form). Task-named grammar: f-string debug `=`.
x = 42
s = f"{x=}"
t = f"{x = }"
u = f"{x=:{5}}"
v = f"{x=!r}"
print(s, t, u, v)

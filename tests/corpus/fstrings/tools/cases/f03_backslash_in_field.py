# Backslash allowed inside replacement field since 3.12
print(f"{chr(10)!r}|{"a\tb"!r}")

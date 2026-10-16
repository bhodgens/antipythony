# T-01 valid fixture: f-string conversions and format specs.
# Cites: S-01-16 (`!r/!s/!a` conversions, `:format_spec` clauses).
x = 3.14159
t = 1083
a = f"{x!r}|{x!s}|{x!a}"
b = f"{x:.2f}|{x:>10.3e}|{t:%Y-%m-%d}"
width = 8
c = f"{x:{width}.{2}f}"
print(a, b, c)

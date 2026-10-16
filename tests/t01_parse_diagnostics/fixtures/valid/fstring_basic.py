# T-01 valid fixture: f-string interpolation, nesting, concat.
# Cites: S-01-16 (f-string interpolation with conversion/format-spec clauses; plan-01 Expr list `fstring`).
name = "world"
greeting = f"hello {name}!"
inner = f"{f'{name.upper()}'}"
multi = f"line1\nline2 {name} tail"
print(greeting, inner, multi)

# T-03 corpus program (lowering semantics, gates leaf 01.03).
# Cites: S-01-16
# f-strings: str() default, !r/!s/!a conversions, :format_spec, nested expressions (anchor A01-12).
name = "world"
x = 3.256
data = {"k": [1, 2]}
s1 = f"{name}, {x:.2f}, {2 + 3}, {x!r}"
s2 = f"{42:>6}|{42:06}|{1234567:,}"
s3 = f"nested {data['k'][1]!r} ascii {name!a}"
print(s1)
print(s2)
print(s3)

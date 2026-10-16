# T-01 valid fixture: string literal forms: raw, bytes, named escapes, implicit concatenation.
# Cites: plan-01 Expr list `literals`; spec 01 grammar surface (task: valid-program acceptance over named syntax). String literal type set is core 3.14 grammar (C-03).
s = "tab\there\n"
r = r"raw\n no escape \n"
b = b"bytes \x41"
br = br"raw bytes \x"
n = "\N{GREEK SMALL LETTER ALPHA}"
u = "unicode \u00e9 \U0001F600"
cat = "implicit" "concat"
tri = """triple
lines"""
print(s, r, b, br, n, u, cat, tri)

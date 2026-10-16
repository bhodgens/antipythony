# T-05 values corpus V063
# Cites: S-01-42
# Pins: repr(str) quote selection (single default; doubles iff only-quote
# present; both -> single with escaped inner apostrophe), escape set \t \n \r
# \xNN lowercase for controls, backslash doubling, printable non-ASCII passes
# through, eval(repr(s)) == s round-trip, str is NOT repr for print.
print(repr("it's"), repr('say "hi"'), repr("both ' and \""))
print(repr("\t\n\x00\x07\r"), repr("\\"))
print(repr("caf\u00e9 \u4e2d"), repr("\U0001F600"))
vals = ["it's", 'say "hi"', "both ' and \"", "tab\there", "\x00zero", "", "caf\u00e9"]
print(all(eval(repr(v)) == v for v in vals))
print("a\nb")
print(repr("a\nb"))
print(repr("\x7f"), repr("\x1f"))

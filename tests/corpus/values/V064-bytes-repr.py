# T-05 values corpus V064
# Cites: S-01-43, S-01-18
# Pins: repr(bytes) is b'...' with the same quote-selection rule and \xNN
# escaping for non-printable/non-ASCII bytes; ASCII printable stays raw.
print(repr(b"caf\xc3\xa9"))
print(repr(b"\x00abc'\"\\\\"))
print(repr(b"it's"), repr(b'say "hi"'), repr(b"both ' and \""))
print(repr(b""), repr(b"\xff\xfe"), repr(b"printable-0123 ~!@#$%^&*()"))

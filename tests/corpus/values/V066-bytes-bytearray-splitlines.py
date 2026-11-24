# T5 bytes.splitlines / bytearray.splitlines pins (F1 fix wave).
# Cites: S-01-17 (str/bytes strict separation — the bytes boundary set),
# S-01-15 (bytes slicing/sequence semantics).
# Pins: bytes terminator set is ONLY \n \r \r\n (\v \f \x1c are NOT bytes
# boundaries — str-only universal-newline chars), keepends both ways,
# empty/only-terminator/trailing-chunk cases, and the join round-trip.
# bytearray mirrors bytes and yields bytearray-typed items; bytes yields
# bytes-typed items.
for t in [b"\n", b"\r\n", b"\r"]:
    b = b"a" + t + b"b" + t
    print(repr(b.splitlines(keepends=True)))
    print(repr(b.splitlines()))
print(repr(b"a\vb\fc\x1cd".splitlines(keepends=True)))
print(repr(b"".splitlines(keepends=True)), repr(b"".splitlines()))
print(repr(b"\n".splitlines(keepends=True)))
print(repr(b"xyz".splitlines(keepends=True)))
print(repr(b"a\r\n\r\nb".splitlines()))
print(repr(bytearray(b"a\nb\r\n").splitlines(keepends=True)))
print(repr(bytearray(b"a\nb").splitlines()))
print(repr(bytearray().splitlines(keepends=True)))
print(type(bytearray(b"a\nb").splitlines()[0]).__name__)
print(type(b"a\nb".splitlines()[0]).__name__)
b = b"line one\nline two\r\nline three\r"
print(b"".join(b.splitlines(keepends=True)) == b)

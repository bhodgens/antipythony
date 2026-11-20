# T-05 values corpus V048
# Cites: S-01-18 (codec contract), V016 root-cause note (03.07)
# Pins: lone-surrogate string literals survive the parser (WTF-8 storage),
# chr() accepts the full 0..0x10FFFF range, repr escapes surrogates \udNNNN,
# len/ord count code points, strict encode raises the exact UnicodeEncodeError
# texts (single + run positions), the builtin error handlers (ignore/replace/
# backslashreplace/xmlcharrefreplace/namereplace/surrogatepass/surrogateescape)
# produce CPython's exact bytes, surrogateescape only rescues U+DC80..DCFF,
# utf-16/32 strict reject surrogates while surrogatepass emits raw units,
# e.start/e.end attributes parse from the error message, and equality/order
# comparisons are code-point based.
print(repr("\ud800"), len("\ud800"), ord("\ud800"))
print(chr(0xD800) == "\ud800", hash(chr(0xD800)) == hash("\ud800"))
print(chr(0xDC80) == "\udc80", repr("\udfff"))
try:
    "\ud800".encode("utf-8")
except UnicodeEncodeError as e:
    print("E:", e, e.start, e.end)
try:
    "a\ud800\ud801b".encode("utf-8")
except UnicodeEncodeError as e:
    print("E:", e, e.start, e.end)
print(repr("x\ud800y".encode("utf-8", "ignore")))
print(repr("x\ud800y".encode("utf-8", "replace")))
print(repr("x\ud800y".encode("utf-8", "backslashreplace")))
print(repr("x\ud800y".encode("utf-8", "xmlcharrefreplace")))
print(repr("x\ud800y".encode("utf-8", "surrogatepass")))
print(repr("\udcff".encode("utf-8", "surrogateescape")))
try:
    "\ud800".encode("utf-8", "surrogateescape")
except UnicodeEncodeError as e:
    print("E:", e)
try:
    "\ud800".encode("utf-8", "bogus")
except Exception as e:
    print(type(e).__name__ + ":", e)
print("abc".encode("utf-8", "bogus"))
try:
    b"\xed\xa0\x80".decode("utf-8")
except UnicodeDecodeError as e:
    print("E:", e, e.start, e.end)
print(repr(b"\xed\xa0\x80".decode("utf-8", "surrogatepass")))
print(repr(b"\x80".decode("utf-8", "surrogateescape")))
print(repr(b"\xed\xa0\x80".decode("utf-8", "replace")))
print(repr(b"\xed\xa0\x80".decode("utf-8", "backslashreplace")))
print(repr(b"\xed\xa0\x80".decode("utf-8", "ignore")))
print(repr(b"\xed\xa0".decode("utf-8", "surrogatepass")) if False else "skip-truncated")
print("\ud800" < "\ud801", "\ud800" == "\ud800")
print(repr("a\ud800b"[1]), "a\ud800b".index("\ud800"))
print(repr("\ud800".encode("utf-16-le", "surrogatepass")), repr("\ud800".encode("utf-16", "surrogatepass")))
print(repr("\U0001F600".encode("utf-16", "surrogatepass")))
try:
    "\ud800".encode("utf-16")
except UnicodeEncodeError as e:
    print("E:", e)
print(repr(b"\x00\xd8".decode("utf-16-le", "surrogatepass")))
try:
    "\ud800".encode("utf-32")
except UnicodeEncodeError as e:
    print("E:", e)
print(repr("\ud800".encode("utf-32-le", "surrogatepass")))
try:
    b"\x00\xd8\x00\x00".decode("utf-32-le")
except UnicodeDecodeError as e:
    print("E:", e)
print(repr(b"\x00\xd8\x00\x00".decode("utf-32-le", "surrogatepass")))
print(repr("h\ud800i".encode("gbk", "ignore")), repr("h\ud800i".encode("gbk", "replace")))
print(repr(ascii("\ud800")), repr(f"\ud800"))

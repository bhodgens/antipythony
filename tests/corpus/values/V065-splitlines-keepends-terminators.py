# T5 str.splitlines(keepends=True) terminator classes (F1 fix pin).
# Cites: S-01-14 (str sequence semantics), S-01-15 (str/bytes slicing),
# tests/t5-probe/splitlines_keepends_repro.py (keepends dropped ALL
# terminators). Pins every CPython universal-newline terminator class —
# \n \r\n \r \v \f \x1c \x1d \x1e \x85 \u2028 \u2029 — with keepends=True,
# keepends=False, the \r\n two-char pairing (never splits between \r and \n),
# empty/only-terminator strings, and the join round-trip ("".join(parts) == s).
terms = ["\n", "\r\n", "\r", "\v", "\f", "\x1c", "\x1d", "\x1e", "\x85", "\u2028", "\u2029"]
for t in terms:
    s = "a" + t + "b" + t
    print(repr(s.splitlines(keepends=True)))
    print(repr(s.splitlines()))
print(repr("".splitlines(keepends=True)), repr("".splitlines()))
print(repr("\n".splitlines(keepends=True)), repr("\n".splitlines()))
print(repr("x\r\n".splitlines(keepends=True)), repr("x\r".splitlines(keepends=True)))
print(repr("a\r\n\r\nb".splitlines(keepends=True)))
print(repr("a\r\rb".splitlines(keepends=True)))
print(repr("abc".splitlines(keepends=True)))
print(repr("中文\nテスト\r\n終".splitlines(keepends=True)))
s = "import abc\nimport builtins\nimport sys\r\nimport os\rimport json\vimport re\f"
parts = s.splitlines(keepends=True)
print("".join(parts) == s)

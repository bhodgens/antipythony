"""splitlines differential probe: antipythony vs CPython 3.14.7."""
cases = []

# str: every terminator class, keepends both ways
for t in ["\n", "\r\n", "\r", "\v", "\f", "\x1c", "\x1d", "\x1e", "\x85", "\u2028", "\u2029"]:
    s = "a" + t + "b" + t
    cases.append(("str_kt", repr(s.splitlines(keepends=True))))
    cases.append(("str_nk", repr(s.splitlines(keepends=False))))
    cases.append(("str_kt_kw", repr(s.splitlines(keepends=True))))

# str edge cases
cases.append(("empty_kt", repr("".splitlines(keepends=True))))
cases.append(("empty_nk", repr("".splitlines())))
cases.append(("only_nl_kt", repr("\n".splitlines(keepends=True))))
cases.append(("only_nl_nk", repr("\n".splitlines())))
cases.append(("trailing_nokt", repr("abc".splitlines(keepends=True))))
cases.append(("mixed_kt", repr("a\nb\r\nc\rd\x85e\u2028f\u2029".splitlines(keepends=True))))
cases.append(("crlf_run", repr("a\r\n\r\nb".splitlines(keepends=True))))
cases.append(("trailing_crlf", repr("x\r\n".splitlines(keepends=True))))
cases.append(("trailing_cr", repr("x\r".splitlines(keepends=True))))
cases.append(("default_no_arg", repr("a\nb".splitlines())))
cases.append(("keepends_0", repr("a\nb".splitlines(keepends=0))))
cases.append(("keepends_empty_str", repr("a\nb".splitlines(keepends=""))))
# unicode text around terminators
cases.append(("cjk", repr("中文\nテスト\r\n終".splitlines(keepends=True))))
cases.append(("crlf_only", repr("\r\n".splitlines(keepends=True))))
cases.append(("double_cr", repr("a\r\rb".splitlines(keepends=True))))

# bytes: only \n \r \r\n
for t in [b"\n", b"\r\n", b"\r"]:
    b = b"a" + t + b"b" + t
    cases.append(("bytes_kt", repr(b.splitlines(keepends=True))))
    cases.append(("bytes_nk", repr(b.splitlines(keepends=False))))
# bytes: \v \f \x1c are NOT terminators for bytes
cases.append(("bytes_vf", repr(b"a\vb\fc\x1cd".splitlines(keepends=True))))
cases.append(("bytes_empty", repr(b"".splitlines(keepends=True))))
cases.append(("bytes_only", repr(b"\n".splitlines(keepends=True))))
cases.append(("bytes_trailing", repr(b"xyz".splitlines(keepends=True))))
cases.append(("bytes_crlf_run", repr(b"a\r\n\r\nb".splitlines())))
cases.append(("bytes_kw", repr(b"a\nb".splitlines(keepends=True))))

# bytearray mirrors bytes
ba = bytearray(b"a\nb\r\n")
cases.append(("bytearr_kt", repr(ba.splitlines(keepends=True))))
cases.append(("bytearr_nk", repr(bytearray(b"a\nb").splitlines())))
cases.append(("bytearr_empty", repr(bytearray().splitlines(keepends=True))))
cases.append(("bytearr_types", repr(type(bytearray(b"a\nb").splitlines()[0]).__name__)))
cases.append(("bytes_types", repr(type(b"a\nb".splitlines()[0]).__name__)))

# the join round-trip (corruption case)
s = "import abc\nimport builtins\nimport sys\r\nimport os\rimport json\vimport re\f"
parts = s.splitlines(keepends=True)
cases.append(("roundtrip", repr("".join(parts) == s)))
b = b"line one\nline two\r\nline three\r"
bp = b.splitlines(keepends=True)
cases.append(("bytes_roundtrip", repr(b"".join(bp) == b)))

# no-terminator single line, multi-empty lines
cases.append(("single", repr("hello".splitlines(keepends=True))))
cases.append(("blank_lines", repr("\n\n\n".splitlines(keepends=True))))
cases.append(("blank_lines_nk", repr("\n\n\n".splitlines())))

fails = 0
for name, got in cases:
    print(f"{name}={got}")
print(f"TOTAL {len(cases)}", flush=True)

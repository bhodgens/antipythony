"""One-time generator for the tests/corpus/fstrings battery case files.

Each case is a self-contained program whose stdout/stderr/exit-code under
python3.14 becomes the checked-in expected/*.out|.err|.rc (same layout as
tests/corpus/stdlib/). Deterministic output only (sorted, fixed floats).
"""
import pathlib

CASES = {
    # --- quoting / nesting (PEP 701) ---
    "f01_nested_same_quotes": '''\
# Same-quote nesting inside the replacement field (PEP 701, 3.12+)
x = "inner"
print(f"outer {x} {"lit"} end")
''',
    "f02_nested_dict_subscript": '''\
d = {"k": "v"}
print(f"{d['k']}|{d["k"]}")
''',
    "f03_backslash_in_field": '''\
# Backslash allowed inside replacement field since 3.12
print(f"{chr(10)!r}|{"a\\tb"!r}")
''',
    # --- format specs ---
    "f04_spec_fixed": '''\
x = 3.14159
print(f"[{x:.2f}]")
''',
    "f05_spec_align_right": '''\
x = "hi"
print(f"[{x:>10}]")
''',
    "f06_spec_center": '''\
x = "hi"
print(f"[{x:^10}]")
''',
    "f07_spec_fill_align": '''\
x = 7
print(f"[{x:*>10}]")
''',
    "f08_spec_sign_d": '''\
x = 42
y = -7
print(f"[{x:+d}]|[{y:+d}]")
''',
    "f09_spec_alt_hex": '''\
x = 255
print(f"[{x:#x}]|[{x:#X}]|[{x:#o}]")
''',
    "f10_spec_alt_bin": '''\
x = 5
print(f"[{x:#b}]")
''',
    # --- conversions ---
    "f11_conv_r": '''\
x = "text"
print(f"[{x!r}]")
''',
    "f12_conv_s": '''\
class C:
    def __str__(self): return "S"
    def __repr__(self): return "R"
print(f"[{C()!s}]|[{C()!r}]")
''',
    "f13_conv_a": '''\
x = "caf\\u00e9"
print(f"[{x!a}]")
''',
    # --- nesting ---
    "f14_nested_fstring": '''\
x = 3
print(f"outer[{f"inner{x}"}]end")
print(f"{f"{f"deep"}"}")
''',
    "f15_fstring_in_spec": '''\
v = 2.71828
print(f"[{v:{8}.{3}}]")
''',
    "f16_spec_nested_width": '''\
x = 42
for w in (3, 8):
    print(f"[{x:{w}}]")
''',
    # --- debug suffix ---
    "f17_debug_plain": '''\
x = 42
print(f"{x=}")
''',
    "f18_debug_spaces": '''\
x = 42
print(f"{x = }")
''',
    "f19_debug_spec": '''\
x = 42
print(f"{x=:>10}")
''',
    "f20_debug_conv": '''\
x = "txt"
print(f"{x=!r}")
''',
    "f21_debug_expr": '''\
d = {"k": [1, 2]}
print(f"{d['k'] = }")
''',
    # --- multiline / triple-quoted ---
    "f22_triple_multiline": '''\
x = 5
s = f"""line1 {x}
line2 {x * 2}
"""
print(repr(s))
''',
    "f23_multiline_field": '''\
x = 3
print(f"""{
x
}""")
''',
    # --- expressions inside ---
    "f24_dict_literal": '''\
print(f"[{ {'a': 1}['a'] }]")
''',
    "f25_lambda_call": '''\
print(f"[(lambda v: v * 2)(21)]")
''',
    "f26_walrus": '''\
print(f"[{(w := 42)}]|[{w}]")
''',
    "f27_conditional": '''\
x = 10
print(f"[{x if x > 5 else 'small'}]")
''',
    "f28_method_call_subscript": '''\
words = ["Hello", "World"]
print(f"[{words[0].upper()}]|[{words[1][0:3]}]|[{', '.join(words)}]")
''',
    "f29_genexpr": '''\
print(f"[{sum(i for i in range(4))}]|[{max(str(n) for n in range(3))}]")
''',
    # --- implicit concatenation / braces / mixed ---
    "f30_escaped_braces": (
        'x = 1\n'
        'print(f"[{{literal}}]|[{{{x}}}]\")\n'
        'print(f"{{}}")\n'
        'name = "w"\n'
        'print(f"{name} = {{{name.upper()}}}")\n'
    ),
    "f31_implicit_concat_mixed": '''\
cname = "myvar"
s = '"#define X {" f"{cname};" "%s"'
print(s)
t = "a" f"b{x}" "c" f"{cname}" if (x := 9) else ""
print(t)
''',
    "f32_concat_plain_between": '''\
x, y = 1, 2
s = f"{x}" "mid" f"{y}"
print(s)
print(repr("l1" f"{x+1}" "\\t" f"{y+1}"))
''',
    "f33_bytes_adjacent_error": '''\
# str f-string adjacent to bytes literal: SyntaxError in CPython
x = "s"
print(f"{x}" b"tail")
''',
    "f34_spec_literal_braces": '''\
# nested spec with expression AND literal text mixed
x, w = 3.5, 10
print(f"[{x:>{w}.2f}]")
''',
    "f35_empty_and_self": '''\
print(f"{''!r}")
print(f"{f''!r}")
''',
}

root = pathlib.Path(__file__).resolve().parent
cases = root / "cases"
cases.mkdir(exist_ok=True)
for name, src in CASES.items():
    (cases / f"{name}.py").write_text(src)
print(f"wrote {len(CASES)} cases")

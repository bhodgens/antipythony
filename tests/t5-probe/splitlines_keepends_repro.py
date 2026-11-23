"""T5 Wall-2 F1 repro: str.splitlines(keepends=True) drops terminators.

CPython 3.14.7: ['import abc\n', 'import builtins\n'] and the bytes variant works.
antipythony: ['import abc', 'import builtins'] (terminators GONE) and
bytes.splitlines raises AttributeError.
"""
s = "import abc\nimport builtins\n"
parts = s.splitlines(keepends=True)
print("str keepends:", parts)
assert "\n" in parts[0], "F1: keepends=True lost the newline"

print("joined:", repr("".join(parts)))

b = b"a\nb\n"
print("bytes keepends:", b.splitlines(keepends=True))

print("DONE", flush=True)

# Cites: S-05-7, S-05-8, S-05-9, S-05-10, S-05-11, S-05-12, S-05-13
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import builtins
want = ["None", "True", "Ellipsis", "NotImplemented", "bool", "int", "float",
        "complex", "str", "bytes", "bytearray", "list", "dict", "tuple", "set",
        "frozenset", "object", "type", "range", "slice", "property",
        "classmethod", "staticmethod", "super", "abs", "all", "any", "ascii",
        "bin", "callable", "chr", "delattr", "divmod", "enumerate", "filter",
        "format", "getattr", "hasattr", "hash", "hex", "id", "isinstance",
        "issubclass", "iter", "len", "map", "max", "min", "next", "oct",
        "open", "ord", "pow", "print", "repr", "reversed", "round", "setattr",
        "sorted", "sum", "vars", "zip"]
print(all(hasattr(builtins, n) for n in want), len(want))
for n in ["eval", "exec", "compile", "globals", "locals", "dir", "aiter",
          "anext"]:
    assert hasattr(builtins, n), n
print("conditional-builtins-present")
for n in ["ArithmeticError", "Exception", "ValueError", "TypeError",
          "ZeroDivisionError", "OverflowError", "StopIteration",
          "ModuleNotFoundError", "NameError", "OSError", "SystemExit"]:
    assert hasattr(builtins, n), n
print(IOError is OSError, EnvironmentError is OSError)
# Anchor #2 (S-05-9): exact byte pin from spec 05.
print(abs(-3), sorted([3, 1, 2]), isinstance(True, int), issubclass(bool, int),
      divmod(17, 5), pow(2, 10), len({1, 2}))
print(list(map(str, range(3))), list(filter(None, [0, 1, "", 2])))
print(list(enumerate("ab", start=10)), list(reversed([1, 2, 3])),
      sum([1, 2], 4), all([True, 1]), any([0, False, 2]))
# S-05-11 compile/eval error shape (single-line pin per spec 06 S-06-7 rule).
try:
    eval("1+")
except SyntaxError as e:
    print(type(e).__name__, e.msg)
print(eval("[i*i for i in range(4)]"))
print(compile("def f(): pass", "<t11>", "exec") is not None)
print(ascii("é"), bin(5), oct(5), hex(5), chr(65), ord("a"), format(2.5, ".1f"),
      repr(1j), max([1, 9, 3]), min([1, 9, 3]), round(2.5), round(-1.5, 1))
print(isinstance(next(iter([1])), int), callable(print), hasattr(print, "sep") if False else len("vars-check"),
      vars().get("baked_check") is None)
baked_check = 1
print(len(dir(int)) > 10, "append" in dir(list))

# T-05 spot probes — reference-byte check harness (NOT part of the corpus;
# verifies that the error-message texts quoted in docs/spec/01 are the texts
# this machine's python3.14 actually prints before we pin them.)
import sys


def tail(e):
    return f"{type(e).__name__}: {e}"


def check(label, fn, expected):
    try:
        got = fn()
    except Exception as e:
        got = tail(e)
    print(label, "OK" if got == expected else f"MISMATCH got={got!r} want={expected!r}")


def mk_add():
    def f():
        "x" + b"y"
    return f


def mk_concat_bytes():
    def f():
        b"y" + "x"
    return f


def mk_mul():
    def f():
        "x" * 1.5
    return f


def mk_assign():
    def f():
        s = "abc"
        s[0] = "z"
    return f


def mk_index():
    def f():
        s = "abc"
        return s[5]
    return f


def mk_slice():
    def f():
        s = "abc"
        return s[::0]
    return f


def mk_idx_float():
    def f():
        s = "abc"
        return s[1.5]
    return f


def mk_unhashable_list():
    def f():
        return hash([1, 2])
    return f


def mk_unhashable_dict():
    def f():
        return hash({1: 2})
    return f


def mk_unhashable_set():
    def f():
        return hash({1})
    return f


def mk_dictkey():
    def f():
        return {[1]: 2}
    return f


def mk_setelem():
    def f():
        return {[1]}
    return f


def mk_tuple_del():
    def f():
        t = (1,)
        del t[0]
    return f


def mk_lt():
    def f():
        return 1 < "x"
    return f


def mk_decode():
    def f():
        return b"\xff".decode("ascii")
    return f


def mk_encode():
    def f():
        return "\ud800".encode("utf-8")
    return f


def mk_codec():
    def f():
        return "x".encode("bogus-codec")
    return f


check("S-01-17 str+bytes", mk_add(), 'TypeError: can only concatenate str (not "bytes") to str')
check("S-01-17 bytes+str", mk_concat_bytes(), "TypeError: can't concat str to bytes")
check("S-01-17 mul", mk_mul(), "TypeError: can't multiply sequence by non-int of type 'float'")
check("S-01-14 assign", mk_assign(), "TypeError: 'str' object does not support item assignment")
check("S-01-14 index", mk_index(), "IndexError: string index out of range")
check("S-01-15 step0", mk_slice(), "ValueError: slice step cannot be zero")
check("S-01-14 idxfloat", mk_idx_float(), "TypeError: string indices must be integers, not 'float'")
check("S-01-39 hash list", mk_unhashable_list(), "TypeError: unhashable type: 'list'")
check("S-01-39 hash dict", mk_unhashable_dict(), "TypeError: unhashable type: 'dict'")
check("S-01-39 hash set", mk_unhashable_set(), "TypeError: unhashable type: 'set'")
check("S-01-24 dict key", mk_dictkey(), "TypeError: cannot use 'list' as a dict key (unhashable type: 'list')")
check("S-01-26 set elem", mk_setelem(), "TypeError: cannot use 'list' as a set element (unhashable type: 'list')")
check("S-01-19 tuple del", mk_tuple_del(), "TypeError: 'tuple' object doesn't support item deletion")
check("S-01-34 lt", mk_lt(), "TypeError: '<' not supported between instances of 'int' and 'str'")
check("S-01-18 decode", mk_decode(),
      "UnicodeDecodeError: 'ascii' codec can't decode byte 0xff in position 0: ordinal not in range(128)")
check("S-01-18 encode", mk_encode(),
      "UnicodeEncodeError: 'utf-8' codec can't encode character '\\ud800' in position 0: surrogates not allowed")
check("S-01-18 codec", mk_codec(), "LookupError: unknown encoding: bogus-codec")

# C-14 int->str digit limit
def mk_limit():
    return str(10**5000)
print("C-14 limit str:", end=" ")
try:
    mk_limit()
except Exception as e:
    print(repr(tail(e)))
print("C-14 limit int:", end=" ")
try:
    int("1" * 5000)
except Exception as e:
    print(repr(tail(e)))

# C-14 MemoryError
print("C-14 memory:", end=" ")
try:
    x = 1 << (2**62)
    print("NO RAISE")
except MemoryError as e:
    print(f"MemoryError: {e}")
except OverflowError as e:
    print(f"OverflowError: {e}")

# error tuple texts (S-01-2)
check("S-01-2 0**-1", lambda: 0 ** -1, "ZeroDivisionError: zero to a negative integer power")
check("S-01-11 inf2int", lambda: int(float("inf")), "OverflowError: cannot convert float infinity to integer")
check("S-01-11 nan2int", lambda: int(float("nan")), "ValueError: cannot convert float NaN to integer")
check("S-01-13 badint", lambda: int("12.5"), "ValueError: invalid literal for int() with base 10: '12.5'")
check("S-01-13 badfloat", lambda: float("abc"), "ValueError: could not convert string to float: 'abc'")
check("S-06-7 1/0", lambda: 1 / 0, "ZeroDivisionError: division by zero")
check("S-06-7 1.0/0.0", lambda: 1.0 / 0.0, "ZeroDivisionError: division by zero")
check("C-16 1.0%0.0", lambda: 1.0 % 0.0, "ZeroDivisionError: division by zero")
check("S-01-11 bigfloat", lambda: float(10**400), "OverflowError: int too large to convert to float")

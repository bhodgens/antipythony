#!/usr/bin/env python3
"""Generate ap-parse test fixtures (50 valid + 5 invalid Python programs)."""
import os, sys

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)

VALID = {}

VALID["literals_int"] = """\
a = 42
b = -17
c = 0x1_FFFF
d = 0o755
e = 0b1010_1010
f = 10**100
"""

VALID["literals_float"] = """\
x = 3.14
y = 1e10
z = 2.5e-3
w = 0.0
huge = 1_000_000.500_001
"""

VALID["literals_complex"] = """\
a = 1j
b = 2 + 3j
c = complex(1, 2)
d = 4.5J - 0.5j
"""

VALID["literals_str"] = """\
a = "hello"
b = 'world'
c = \"\"\"triple
double\"\"\"
d = '''triple
single'''
e = "escapes: \\n \\t \\\\ \\x41 \\u00e9 \\N{BULLET}"
f = rb"raw\\bytes"
"""

VALID["literals_bytes_bytearray"] = """\
a = b"bytes"
b = rb'raw\\bytes'
c = b"\\x00\\xff"
d = bytearray(b"mutable")
e = b"concat" b"enation"
"""

VALID["literals_bool_none_ellipsis"] = """\
t = True
f = False
n = None
e = ...
def g(): ...
"""

VALID["literals_collection_displays"] = """\
lst = [1, 2, 3]
empty_list = []
tup = (1, "two", 3.0)
single = (42,)
empty_tuple = ()
"""
# NOTE: dict/set in own fixture below to keep every fixture focused.

VALID["literals_dict_display"] = """\
d = {"one": 1, "two": 2}
empty = {}
mixed = {1: "a", "b": 2, (3, 4): "tuple-key"}
unpacked = {**mixed, "extra": True}
"""

VALID["literals_set_display"] = """\
s = {1, 2, 3}
empty = set()
sing = {42}
unpacked = {*s, 4, 5}
"""

VALID["list_comprehension"] = """\
squares = [x * x for x in range(10)]
evens = [x for x in range(20) if x % 2 == 0]
pairs = [(x, y) for x in "ab" for y in (1, 2) if x != "a" or y == 1]
"""

VALID["dict_comprehension"] = """\
lengths = {w: len(w) for w in ["a", "bb", "ccc"]}
filtered = {k: v for k, v in lengths.items() if v > 1}
swapped = {v: k for k, v in lengths.items()}
"""

VALID["set_comprehension"] = """\
lower = {c.lower() for c in "Hello World"}
unique_lens = {len(w) for w in ["a", "bb", "cc", "ccc"]}
"""

VALID["generator_expression"] = """\
total = sum(x * x for x in range(10))
chunked = list((i, chr(i)) for i in range(5) if i % 2)
lazy = (n for n in range(100) if n % 7 == 0)
"""

VALID["nested_comprehension_clauses"] = """\
matrix = [[r * c for c in range(3)] for r in range(3)]
flat = [v for row in matrix for v in row]
conditional = [x if x > 0 else -x for x in (-2, 3, -4)]
"""

VALID["generator_yield"] = """\
def counter(n):
    i = 0
    while i < n:
        yield i
        i += 1

def squares(n):
    for i in counter(n):
        yield i * i
"""

VALID["generator_yield_from"] = """\
def inner():
    yield 1
    yield 2
    return "done"

def outer():
    result = yield from inner()
    yield result
    yield from (3, 4)
"""

VALID["generator_send_throw"] = """\
def echo():
    while True:
        try:
            received = yield "ready"
        except ValueError:
            yield "caught"
        else:
            yield received
"""

VALID["try_except_finally"] = """\
def risky():
    try:
        return 1 / 0
    except ZeroDivisionError:
        return None
    finally:
        pass
"""

VALID["try_except_else_finally"] = """\
def full_shape():
    try:
        value = int("10")
    except ValueError:
        value = 0
    else:
        value += 1
    finally:
        cleanup = True
    return value
"""

VALID["try_multiple_except"] = """\
def parse_pair(s):
    try:
        a, b = s.split(",")
        return int(a), int(b)
    except (ValueError, TypeError):
        return (0, 0)
    except AttributeError as exc:
        raise RuntimeError("bad input") from exc
"""

VALID["try_except_star"] = """\
def task_group_demo():
    try:
        raise ExceptionGroup("eg", [ValueError("x")])
    except* ValueError as e:
        pass
    except* TypeError:
        pass
"""

VALID["try_nested_reraise"] = """\
def layered():
    try:
        try:
            raise KeyError("inner")
        finally:
            log = True
    except KeyError:
        raise
"""

VALID["with_single"] = """\
with open("data.txt") as fh:
    data = fh.read()
"""

VALID["with_multi_items"] = """\
with open("a") as a, open("b") as b, lock():
    pass
"""

VALID["with_parenthesized_items"] = """\
with (
    open("a") as a,
    open("b") as b,
    ctx() as c,
):
    result = a.read() + b.read()
"""

VALID["with_no_as"] = """\
with suppress_value():
    x = 1
"""

VALID["match_literal"] = """\
def http_status(code):
    match code:
        case 200:
            return "ok"
        case 404:
            return "missing"
        case _:
            return "other"
"""

VALID["match_capture_wildcard"] = """\
def first_of(seq):
    match seq:
        case []:
            return None
        case [head]:
            return head
        case [head, *rest]:
            return (head, rest)
"""

VALID["match_guard"] = """\
def classify(n):
    match n:
        case int(x) if x < 0:
            return "negative"
        case int(x) if x == 0:
            return "zero"
        case int(x):
            return "positive"
"""

VALID["match_sequence_mapping"] = """\
def shape(v):
    match v:
        case {"kind": "point", "x": x, "y": y}:
            return (x, y)
        case {"kind": "circle", **rest}:
            return rest
        case [x, y] | (x, y):
            return (x, y)
"""

VALID["match_class_pattern"] = """\
class Point:
    __slots__ = ("x", "y")
    def __init__(self, x, y):
        self.x = x
        self.y = y

def quadrant(p):
    match p:
        case Point(x=0, y=0):
            return "origin"
        case Point(x=x, y=y) if x > 0 and y > 0:
            return "first"
        case Point():
            return "elsewhere"
"""

VALID["match_or_pattern"] = """\
def route(cmd):
    match cmd.split():
        case ["quit" | "exit"]:
            return "bye"
        case ["load", path] | ["open", path]:
            return path
        case _:
            return "unknown"
"""

VALID["fstring_basic"] = """\
name = "world"
a = f"hello {name}"
b = f'count: {42}'
c = f\"\"\"multi {name}
line\"\"\"
d = F"upper prefix"
e = rf"raw {name}\\d+"
"""

VALID["fstring_conversion_nesting"] = """\
rows = [["a", 1], ["b", 2]]
a = f"{rows!r}"
b = f"{[str(cell) for cell in row]!s}"
c = f"{format(deep, '>4')}"
"""

VALID["fstring_same_quote_inner"] = """\
who = "world"
a = f"hello {who} bye"
b = f'mixed {who} quotes'
c = f"expr {who.upper()} end"
"""

VALID["fstring_format_spec"] = """\
pi = 3.14159
a = f"{pi:.2f}"
b = f"{pi:>10.4f}"
c = f"{255:x} {255:#08b} {255:,}"
d = f"{pi=:.1f}"
e = f"{'center':^12}"
"""

VALID["fstring_debug_equals"] = """\
value = 7
name = "x"
a = f"{value=}"
b = f"{value = }"
c = f"{name!r}"
d = f"{name!s:>{len(name)}}"
e = f"mixed {value=} and {name!r}"
"""

VALID["lambda_basic"] = """\
add = lambda a, b: a + b
apply = lambda f, x: f(x)
result = apply(lambda v: v * 2, 21)
"""

VALID["lambda_defaults_kwonly"] = """\
f = lambda a, b=2, *args, k=3, **kw: (a, b, args, k, kw)
g = lambda *, key: key
h = lambda a, /, b: (a, b)
"""

VALID["decorator_simple"] = """\
def trace(fn):
    def wrapper(*args, **kwargs):
        return fn(*args, **kwargs)
    return wrapper

@trace
def compute(x):
    return x + 1
"""

VALID["decorator_with_args"] = """\
def repeat(n):
    def deco(fn):
        def wrapper(*a, **kw):
            return [fn(*a, **kw) for _ in range(n)]
        return wrapper
    return deco

@repeat(3)
def ping():
    return "pong"
"""

VALID["decorator_stacked_class"] = """\
def annotate(cls):
    cls.annotated = True
    return cls

def register(fn):
    fn.registered = True
    return fn

@register
@annotate
class Widget:
    @staticmethod
    def make():
        return Widget()

@register
def reset():
    pass
"""

VALID["class_multi_inheritance"] = """\
class Base:
    def root(self):
        return "root"

class MixinA(Base):
    def name(self):
        return "a"

class MixinB:
    def tag(self):
        return "b"

class Combined(MixinA, MixinB, Base):
    def all(self):
        return (self.name(), self.tag(), self.root())
"""

VALID["class_metaclass_kwarg"] = """\
class Meta(type):
    pass

class Configured(dict, metaclass=Meta):
    option = "on"
"""

VALID["class_slots_property"] = """\
class Vec:
    __slots__ = ("x", "y")

    def __init__(self, x, y):
        self.x = x
        self.y = y

    @property
    def magnitude(self):
        return (self.x ** 2 + self.y ** 2) ** 0.5

    @magnitude.setter
    def magnitude(self, m):
        self.x, self.y = m, 0
"""

VALID["class_dataclass_like"] = """\
from dataclasses import dataclass, field

@dataclass
class Item:
    name: str
    tags: list[str] = field(default_factory=list)
    count: int = 1
"""

VALID["async_def_await"] = """\
async def fetch():
    return 42

async def main():
    value = await fetch()
    inner = await fetch_inner()
    return value + inner

async def fetch_inner():
    return 8
"""

VALID["async_for_with"] = """\
async def stream_lines(reader):
    async for line in reader:
        yield line.strip()

class AsyncCtx:
    async def __aenter__(self):
        return self

    async def __aexit__(self, *exc):
        return False

async def guarded():
    async with AsyncCtx() as c:
        async for item in stream_lines(c):
            if item:
                return item
    return None
"""

VALID["async_comprehension"] = """\
async def collect(agen):
    results = [item async for item in agen if item is not None]
    paired = {i: v async for i, v in azip()}
    return results, paired

async def azip():
    yield 0, "zero"
"""

VALID["global_nonlocal"] = """\
counter = 0

def bump():
    global counter
    counter += 1

def make_accumulator():
    total = 0
    def add(n):
        nonlocal total
        total += n
        return total
    return add
"""

VALID["imports_all_forms"] = """\
import os
import sys, math
import os.path as osp
from collections import OrderedDict
from typing import (
    Any,
    Callable,
)
from os import path, sep
from . import sibling
from ..pkg import thing
from .rel import name as alias
"""

VALID["star_args_kwargs_kwonly"] = """\
def full(a, b=1, /, c=2, *args, d, e=4, **kwargs):
    return (a, b, c, args, d, e, kwargs)

def call():
    return full(0, 1, 2, 3, 4, d=5, extra=6)
"""

VALID["posonly_defaults"] = """\
def clamp(v, lo=0, hi=10, /):
    return max(lo, min(hi, v))

def kinds(*, a, b="b"):
    return a + b
"""

VALID["walrus"] = """\
if (n := 10) > 5:
    big = n

chunks = [data for data in (chunk := [1, 2, 3])]

while (line := next_line()) is not None:
    seen = line
"""

VALID["chained_comparisons"] = """\
a = 1 < 2 < 3
b = 1 <= x < y != z
c = "a" in "abc" and "b" not in "bcd"
d = obj is other is not None
e = 1 == 1 == 1
"""

VALID["conditional_expressions"] = """\
grade = "pass" if score >= 60 else "fail"
nested = "hi" if a else ("mid" if b else "lo")
in_call = print("yes" if flag else "no")
"""

VALID["type_alias_312"] = """\
type Vector = list[float]
type Pair[T] = tuple[T, T]
type Recursive[T] = T | list[Recursive[T]]

def first[P, *Ts, **Kw](p: P, *ts: Ts, **kw: Kw) -> P:
    return p
"""

VALID["generic_class_312"] = """\
class Box[T]:
    def __init__(self, item: T) -> None:
        self.item = item

    def get(self) -> T:
        return self.item

class Registry[KT, VT](dict):
    def add(self, k: KT, v: VT) -> None:
        self[k] = v
"""

VALID["del_pass_assert"] = """\
x = 1
del x
assert True, "always"
assert 1 + 1 == 2

def stub():
    pass
"""

VALID["aug_assign_annotations"] = """\
count: int = 0
count += 5
count -= 2
count *= 3
count //= 2
count %= 4
count **= 2
ratio: float
ratio = count / 8
flag: bool | None = None
"""

VALID["operators_bitwise"] = """\
a = 0b1100 & 0b1010
b = 0b1100 | 0b1010
c = 0b1100 ^ 0b1010
d = ~a
e = b << 2
f = b >> 1
g = a + b * c - d // e % f
h = -x if x else +x
"""

VALID["attribute_subscript_calls"] = """\
obj.attr.deep[0]["key"][1:2]
obj.attr.deep[0]["k"][1:5:2]
fn(*args, **kwargs)
obj.method(a)(b)[c].attr
slices = m[::2, 1, :]
"""

VALID["starred_assign_aug"] = """\
a, *b, c = range(10)
first, second = second, first
*x, = [1]
del x
"""

INVALID = {
"invalid_unclosed_paren": """\
x = (1 + 2
y = 3
""",
"invalid_bad_indent": """\
def f():
    x = 1
  y = 2
""",
"invalid_positional_after_kwarg": """\
f(x=1, 2)
""",
"invalid_stray_bracket": """\
x = ]
""",
"invalid_bad_fstring": """\
msg = f'{'
""",
}
for name, body in VALID.items():
    with open(os.path.join(OUT, f"valid_{name}.py"), "w") as fh:
        fh.write(body)

for name, body in INVALID.items():
    with open(os.path.join(OUT, f"invalid_{name}.py"), "w") as fh:
        fh.write(body)

print(f"wrote {len(VALID)} valid, {len(INVALID)} invalid fixtures to {OUT}")

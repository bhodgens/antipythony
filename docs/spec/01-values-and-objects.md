# Spec 01 — Values and Objects

Clause prefix: `S-01-*`. Test cases cite these ids; normative keywords are **MUST**,
**SHOULD**, and **UNSPECIFIED**. An UNSPECIFIED property is one antipythany may choose
freely but observable program behavior MUST NOT depend on it — corpus cases MUST NOT
assert on it either.

Reference behavior: CPython 3.14 (all quoted outputs produced by
`python3.14` 3.14.7, macOS arm64). This doc is written from the master plan, the
compatibility contract (00.03), and CPython semantics — never from implementation
code. Where this doc quotes byte-level output (repr, error text), the quote IS the
requirement: antipythany output that differs byte-wise from CPython on any corpus
case is a bug.

Gate rule (from master.md): all observable behavior here is gated by `ap-conform`
byte-diff of stdout, stderr, and exit code.

---

## 1. Numbers

### 1.1 Integers

- **S-01-1 (MUST)** `int` is arbitrary precision. `+ - * // % ** <<` and decimal
  formatting never overflow or lose precision for any magnitude a program can
  express (bounded only by memory). Literals support underscores (`1_000_000` ==
  1000000) and the prefixes `0b`, `0o`, `0x`. Signed right shift is arithmetic
  (`-5 >> 1` == `-3`); `~x` == `-x-1`.
- **S-01-2 (MUST)** `int ** int`:
  - non-negative exponent: exact arbitrary-precision integer, overflow-free
    (`2 ** 200` prints `1606938044258990275541962092341162602522202993782792835301376`);
  - `0 ** 0` == 1; `0 ** -1` raises `ZeroDivisionError: zero to a negative power`;
  - negative exponent: result is `float` (`2 ** -3` == 0.125);
  - `**` binds tighter than unary `-` and is right-associative:
    `-7 ** 2` == -49, `3 ** 3 ** 2` == 19683.

### 1.2 Floor division and modulo sign rules

- **S-01-3 (MUST)** `//` is floor division: the result is `floor(a/b)` — it truncates
  toward negative infinity, for `int` AND `float` operands alike. Complete sign
  table for `7 // 2, -7 // 2, 7 // -2, -7 // -2` → `3, -4, -4, 3`.
- **S-01-4 (MUST)** `%` takes the sign of the **divisor** (or is zero).
  `7 % 2, -7 % 2, 7 % -2, -7 % -2` → `1, 1, -1, -1`. The fundamental invariant
  `a == (a // b) * b + (a % b)` MUST hold exactly for all int operands, and its
  float analogue MUST match CPython's observed results (`7.0 // 2.0` → `3.0`,
  `-7.0 % 2.0` → `1.0`).
- **S-01-5 (MUST)** `divmod(a, b)` returns `(a // b, a % b)` with identical sign
  rules, including mixed types: `divmod(7, -2)` → `(-4, -1)`, `divmod(7.0, 2)` →
  `(3.0, 1.0)` (result tuple takes the promoted operand type).

### 1.3 True division and type promotion

- **S-01-6 (MUST)** `/` is always true division and always returns `float`, even for
  exactly-representable quotients: `6 / 3` prints `2.0`, `3 / 2` prints `1.5`.
- **S-01-7 (MUST)** Mixed int/float arithmetic promotes to `float`; the int operand
  is converted exactly before the operation (no double rounding). `1 + 2.0` == 3.0
  with `type(...) is float`.
- **S-01-8 (MUST)** `float` is IEEE-754 binary64; arithmetic rounds to nearest even.
  `0.1 + 0.2` prints `0.30000000000000004`. Overflow of a float operation yields
  `inf` (or `-inf`) silently, not an exception: `1e308 * 10` prints `inf`.
  Division by zero raises for BOTH operand types with identical text:
  `ZeroDivisionError: division by zero` (`1 / 0` and `1.0 / 0.0`).

### 1.4 bool is a subclass of int

- **S-01-9 (MUST)** `bool ⊂ int`: `isinstance(True, int)` is True, `True == 1`,
  `False == 0`, `True + True` == 2, `True * 3` == 3. Bool arithmetic results are
  `int` (never `bool`) except `not` (returns `bool`) and bool-valued contexts.
  `type(True) is bool` and `type(1) is int`.
- **S-01-10 (MUST)** `repr(True)` == `True`, `repr(False)` == `False`,
  `repr(None)` == `None` — never `1`/`0`.

### 1.5 float ↔ int conversion

- **S-01-11 (MUST)** `int(float)` truncates toward zero: `int(3.7)` == 3,
  `int(-3.7)` == -3, `int(-0.0)` == 0. Conversions of the special values raise with
  exact text: `int(float("nan"))` → `ValueError: cannot convert float NaN to
  integer`; `int(float("inf"))` → `OverflowError: cannot convert float infinity to
  integer`. `float(big_int)` where the value exceeds the float range →
  `OverflowError: int too large to convert to float` (for `10**400`).
- **S-01-12 (MUST)** `round()` uses banker's rounding (ties to even) at the
  representable-binary64 level: `round(2.5)` → 2, `round(3.5)` → 4,
  `round(-2.5)` → -2, `round(1.5)` → 2; `round(2.675, 2)` → 2.67 (because 2.675
  is stored below the tie); `round(0.125, 2)` → 0.12; `round(float)` with no ndigits
  returns `int`, with ndigits returns `float`.
- **S-01-13 (MUST)** `int(str)` parses base-10 by default with surrounding whitespace
  and optional sign allowed (`int("  42  ")` == 42), supports an explicit base
  (`int("ff", 16)` == 255, `int("-1b", 32)` == -43), and rejects non-integers with
  exact text: `int("12.5")` → `ValueError: invalid literal for int() with base 10: '12.5'`.
  `float("abc")` → `ValueError: could not convert string to float: 'abc'`.

## 2. Strings

- **S-01-14 (MUST)** `str` is an immutable sequence of Unicode code points;
  `len()` counts code points (NOT grapheme clusters): `len("café")` == 4 and
  `len("a\u0301")` == 2. Item assignment raises
  `TypeError: 'str' object does not support item assignment`. Single indexing of an
  out-of-range position raises `IndexError: string index out of range`. Indexing with
  a float raises `TypeError: string indices must be integers, not 'float'`.
- **S-01-15 (MUST)** Slicing (`s[start:stop:step]`, also on `list`/`tuple`/`bytes`)
  follows CPython's slice-index algorithm exactly: negative indices count from the
  end; out-of-range slice bounds clamp silently (never IndexError) —
  `"abcdefgh"[10:20]` → `''` and `[0,1,2][5:]` → `[]`; `step 0` raises
  `ValueError: slice step cannot be zero`; negative step iterates backward:
  `"abcdefgh"[::-1]` → `hgfedcba`. Slice results of str are str (same type in, same
  type out).
- **S-01-16 (MUST)** f-strings (`f"..."`) interpolate expressions with `str()` by
  default; `!r` forces `repr()`, `!s` forces `str()`, `!a` forces `ascii()`; an
  optional `:format_spec` is passed to `format()`. Observed:
  `f"{name}, {x:.2f}, {2 + 3}, {x!r}"` with `name="world"`, `x=3.256` →
  `world, 3.26, 5, 3.256`. Padding/sign/grouping specs used by the corpus MUST match:
  `f"{42:>6}"` → `    42`, `f"{42:06}"` → `000042`, `f"{1234567:,}"` → `1,234,567`.
  Since Python 3.12 nested same-type quotes are legal (`f"{d["k"]}"` works). The
  debug form `f"{v=}"` prints `v=5` for `v = 5`. Arbitrary expressions (including
  nested f-strings and conditional expressions) are legal inside `{}`.
  `%`-formatting and `str.format` method behavior beyond these basics is owned by
  spec 05 (builtins) — this clause pins only f-string semantics.
- **S-01-17 (MUST)** `str` and `bytes` are strictly separated: implicit
  conversion NEVER happens in operators. `str + bytes` raises exactly
  `TypeError: can only concatenate str (not "bytes") to str`; the reverse raises
  `TypeError: can't concat str to bytes`. `s * 2` requires an int multiplier —
  `"x" * 1.5` raises `TypeError: can't multiply sequence by non-int of type 'float'`.
- **S-01-18 (MUST)** `"café".encode("utf-8")` → `b'caf\xc3\xa9'`;
  `b'caf\xc3\xa9'.decode("utf-8")` → `café`; encoding/decoding default to UTF-8.
  Error texts MUST match byte-for-byte:
  - `b"\xff".decode("ascii")` → `UnicodeDecodeError: 'ascii' codec can't decode byte 0xff in position 0: ordinal not in range(128)`
  - `"\ud800".encode("utf-8")` → `UnicodeEncodeError: 'utf-8' codec can't encode character '\ud800' in position 0: surrogates not allowed`
  - `"x".encode("bogus-codec")` → `LookupError: unknown encoding: bogus-codec`
  `bytes` iter/indexes yield `int` (`b"Hi"[0]` == 72); `len(bytes)` counts BYTES while
  `len(str)` counts code points.

## 3. Containers

- **S-01-19 (MUST)** `list` is mutable; equality is element-wise and recursive
  (`[1, 2] == [True, 2.0]` is True by numeric equality); `list == tuple` is False
  (type-dependent). Assignment creates aliasing, not copies: after `ys = xs;
  ys.append(4)`, `xs` shows `[1, 2, 3, 4]` and `xs is ys` is True. `xs += [5]` and
  `xs.extend(...)` mutate in place; `xs + [y]` builds a new list. `del xs[i]` removes;
  `del t[0]` on a tuple raises `TypeError: 'tuple' object doesn't support item deletion`.
- **S-01-20 (MUST)** `tuple` is immutable; the 1-tuple requires the trailing comma:
  `(1,)` is a tuple of length 1, `(1)` is the int 1, `()` is the empty tuple.
  Tuple display order and unpacking order are left-to-right.
- **S-01-21 (MUST)** `dict` preserves **insertion order**: iteration, `keys()`,
  `values()`, `items()`, `list(d)`, and `repr(d)` MUST enumerate keys in first-
  insertion order. Re-assigning an existing key keeps its original position
  (`{"b":1,"a":2}` then `d["b"]=3` → `['b', 'a']`); deleting then re-adding a key
  moves it to the end; dict display and `{**a, "k": v}` build in written order.
  This ordering is language-mandated in 3.7+ and MUST hold in antipythany.
- **S-01-22 (MUST)** `dict` equality is order-INdependent: `{"a":1,"b":2} ==
  {"b":2,"a":1}` is True (same length; every key present with `==` values).
- **S-01-23 (MUST)** Dict keys collide by hash+equality, forming equivalence
  classes: `1`, `1.0`, and `True` are the same key. Re-assignment overwrites the
  value, keeping the FIRST insertion position: `{1:"a", 1.0:"b", True:"c"}` reprs as
  `{1: 'c'}` and has length 1.
- **S-01-24 (MUST)** Unhashable dict keys raise exactly
  `TypeError: cannot use 'list' as a dict key (unhashable type: 'list')` (message
  names the key type twice); missing-key lookup raises `KeyError` whose str is the
  repr of the key (see §6 and S-02 exception message rules).
- **S-01-25 (MUST)** `set` is unordered and mutable; equality, membership, and the
  operators `| & - ^ <= <` are order-independent. `{1,2,3} == {3,2,1}` is True;
  `1 in {1.0}` is True; `{True} == {1}` is True. `{}` is an empty dict; the empty set
  is ONLY `set()`. Set iteration/repr element order is **UNSPECIFIED** (§7.4): tests
  MUST sort before comparing multi-element sets; single-element set repr is stable.
  `{1: "x"}` dict-display cannot mix with set display. A set-display element that is
  unhashable raises the set-element form (see S-01-26).
- **S-01-26 (MUST)** `frozenset` is the immutable, hashable set:
  `frozenset` with elements `frozenset([1,2]) == frozenset([2,1])` and
  `hash(frozenset([1,2])) == hash(frozenset([2,1]))` in-process; a frozenset can be a
  dict key/set element. `frozenset()` reprs as `frozenset()`; empty `set()` reprs as
  `set()`; non-empty frozenset reprs as `frozenset({1, 2})`.
  Unhashable set DISPLAY elements raise
  `TypeError: cannot use 'list' as a set element (unhashable type: 'list')` (the
  set-element phrasing, distinct from the dict-key phrasing of S-01-24).

## 4. Truthiness

- **S-01-27 (MUST)** Falsy values, exhaustively: `False`, `None`, numeric zero of
  every type (`0`, `0.0`, `-0.0`, `0j`-style zeros), every empty
  sequence/collection (`""`, `()`, `[]`, `{}`, `set()`, `frozenset()`, `b""`), and any
  object whose `__bool__`/`__len__` protocol (§S-01-28) yields False/0. Everything
  else — including `"0"`, `[0]`, `(None)`, and any other instance — is truthy.
  Observed: `bool([]) bool("") bool(0) bool(0.0) bool(None) bool(()) bool({})
  bool("0") bool([0])` → `False False False False False False False True True`.
- **S-01-28 (MUST)** `bool(x)` calls `x.__bool__()` if defined; else
  `x.__len__()` if defined (truthy iff length != 0); else True. `__bool__` takes
  precedence over `__len__`. Observed: a class with `__len__` returning 0 is falsy;
  a class with `__len__` returning 0 but `__bool__` returning True is truthy.
- **S-01-29 (MUST)** `and`/`or` short-circuit and return an OPERAND (or, for `or`,
  the final operand) — never a coerced bool, and never evaluate the right operand
  when the result is decided: `3 and 5` → 5, `0 or "x"` → `x`, `"" or []` → `[]`,
  `1 and 0` → 0, `None or "d"` → `d`. `not x` always returns exactly `True` or
  `False` (`not 0` → `True`, `not not 0` → `False`).

## 5. Equality vs identity

- **S-01-30 (MUST)** `==` compares by value (via `__eq__`, reflected operands,
  falling back to identity); `is` compares object identity. `x == y` does not imply
  `x is y` (`[1,2] == [1,2]` True, `[1,2] is [1,2]` False) and `x is y` always implies
  `x == y` for reflexive `__eq__`.
- **S-01-31 (UNSPECIFIED)** Object caching/aliasing is an implementation detail:
  small-int caching, string interning, tuple/frozen singleton caching are NOT part of
  observable behavior. Whether `1000 is 1000` or `"hello" is "hello"`-style
  expressions hold for two separately-created equal values is UNSPECIFIED. Corpus
  cases MUST NOT assert `is`/`is not` on any value other than `None`, sentinels, or a
  name compared to itself bound to the same object (`s = "x"; s is s` is legal).
  CPython 3.14 additionally emits a SyntaxWarning for `is`/`is not` with literal
  operands; antipythany SHOULD warn identically on the corpus but MUST NOT change
  runtime semantics because of it.
- **S-01-32 (MUST)** `None` is a singleton: `x is None` is THE identity test;
  `None == False` is False; `None == 0` is False. `True is 1` is False (distinct
  objects), `True == 1` is True.
- **S-01-33 (MUST)** Cross-type numeric equality: `1 == 1.0 == True` is True;
  `0 == -0.0` is True; `[True] == [1]` is True; `(1, 2) == (True, 2.0)` is True —
  equality recurses structurally using numeric equality for elements.
  `float("nan") == float("nan")` is False (also `!=` is True), and NaN in a
  containment test (`x in iterable`) is found via identity first: `{n: 1} ` with
  `n = float("nan")` then `n in s2` is True. Equality between unrelated types
  (`1 == "1"`, `[] == ""`, `"" == None`) is False, never an error.
- **S-01-34 (MUST)** Ordering comparisons (`< > <= >=`) between non-orderable
  instances raise exactly
  `TypeError: '<' not supported between instances of 'int' and 'str'` (operator and
  the two type names in order). Chained comparisons evaluate each operand at most
  once and short-circuit: `a < b < c` == `(a < b) and (b < c)` with `b` computed once
  (observed side-effect list `[1, 2, 0]` for `c(1) < c(2) < c(0)`).

## 6. Hashing

- **S-01-35 (MUST)** Core invariant: within one process, `a == b` MUST imply
  `hash(a) == hash(b)` for hashable a, b. Consequences that MUST hold:
  `hash(1) == hash(1.0) == hash(True) == 1`, `hash(2) == hash(2.0)`,
  `hash(0) == hash(0.0) == hash(-0.0) == hash(False) == 0` — even though
  `0 == -0.0` (S-01-33 requires their hashes equal; CPython returns 0 for both).
- **S-01-36 (MUST)** `hash(int) == int` as a contract for the 64-bit build, with the
  two documented exceptions CPython has: `hash(-1)` returns `-2`, and ints with
  magnitude ≥ `2**61 - 1` (the modulus `sys.hash_info.modulus`) return
  `±(int mod (2**61 - 1))`, never -1. Observed: `hash(10**100) == 10**100 %
  (2**61-1)` is True; `hash(-(10**100)) == -(10**100 % (2**61-1))`;
  `hash(2**61 - 1)` → 0. Antipythany MUST reproduce `hash(42) == 42`-style exact
  values for small ints and the -1/-2 exception; the modulus mapping MUST match
  for large ints on the corpus (64-bit modulus 2305843009213693951).
- **S-01-37 (MUST)** `hash(float)`: integral floats hash as their int (`hash(2.0) ==
  2`); non-integral floats follow CPython's rational-mod-prime formula; `-0.0` and
  `0.0` both hash to 0; `hash(inf)` == 314159, `hash(-inf)` == -314159.
  `hash(nan)` is **UNSPECIFIED** (CPython varies per process — observed 273563235 and
  268509015 in two runs; tests MUST NOT assert it).
- **S-01-38 (UNSPECIFIED)** `hash(str)` and `hash(bytes)` are salted per process
  (PYTHONHASHSEED; siphash13 in CPython 3.14). Observed across two runs of the same
  script: `hash("abcdef")` printed `1018421229137590393` then `-6975793468212893174`.
  Antipythany MUST NOT promise any specific string-hash value across processes, and
  corpus MUST NOT print `hash("…")` / `hash(bytes)` directly. In-process: equal
  strings hash equal (`hash("abc") == hash("abc")` True), and set/dict behavior keyed
  on strings MUST be internally consistent.
- **S-01-39 (MUST)** Hashable types in the v1 value model: int, bool, float, complex
  (out of scope), str, bytes, tuple (recursively, if all elements hashable),
  frozenset, range, and class objects. `hash([1,2])` raises exactly
  `TypeError: unhashable type: 'list'`; `hash({1:2})` → `TypeError: unhashable type:
  'dict'`; `hash({1})` where `{1}` is a set literal → `TypeError: unhashable type:
  'set'`.

## 7. repr vs str

Byte-exactness is the gate: `ap-conform` diffs the bytes `print(repr(x))` and
`print(str(x))` emit. These rules define what "correct" means for built-in types.

### 7.1 Integers and singletons

- **S-01-40 (MUST)** `repr(int)` == decimal digits, no `+`, no leading zeros, no
  separators, `-` only for negatives: `repr(-5)` → `-5`, `repr(10**25)` →
  `10000000000000000000000000`. `repr(-0)` == `repr(0)` == `0`. `str(int)` ==
  `repr(int)`. `repr(None|True|False)` → `None|True|False` (S-01-10).

### 7.2 Floats — shortest round-trip

- **S-01-41 (MUST)** `repr(float)` MUST be byte-exact with CPython for every float a
  corpus case produces. Algorithm contract: among all decimal strings that round-trip
  to the same binary64 value, choose the one with the fewest significant digits
  (ties resolve to the digit string closest to the true value — CPython's `repr`
  "r" format / David Gay / Grisu-Ryū family). Layout rules observed:
  - always contains a `.` or an exponent (never reprs as int-looking): `repr(1.0)` → `1.0`;
  - fixed notation for `1e-4 ≤ |x| < 1e16` and their rounded equivalents:
    `repr(0.0001)` → `0.0001`, `repr(3.5e-05)` → `3.5e-05`,
    `repr(12345678.0)` → `12345678.0`, `repr(9999999999999998.0)` →
    `9999999999999998.0`;
  - exponential notation for `|x| < 1e-4` or `|x| ≥ 1e16` after rounding:
    `repr(1e16)` → `1e+16` (also for the value 10000000000000001.0, whose shortest
    round-trip reprs to `1e+16`), `repr(1e-7)` → `1e-07`, `repr(1.5e308)` →
    `1.5e+308`, `repr(5e-324)` → `5e-324`;
  - exponent form is `Xe±YY`: at least two exponent digits, `e-07` not `e-7`,
    no trailing-zero padding (`1.7976931348623157e+308`, `2.2250738585072014e-308`);
  - specials: `repr(inf)` → `inf`, `repr(-inf)` → `-inf`, `repr(nan)` → `nan`;
  - negative zero keeps its sign: `repr(-0.0)` → `-0.0` (while `str(int(-0.0))` → `0`);
  - `str(float)` == `repr(float)` in 3.x.
  Observed spot table (single run): `repr(0.1) repr(1e16) repr(1e-7) repr(1.5e308)
  repr(float("inf")) repr(float("nan")) repr(-0.0)` →
  `0.1 1e+16 1e-07 1.5e+308 inf nan -0.0`, and `repr(1/3)` →
  `0.3333333333333333`, `repr(10/3)` → `3.3333333333333335`.

### 7.3 Strings

- **S-01-42 (MUST)** `repr(str)` MUST be byte-exact with CPython. Rules:
  - Quote selection: single quotes by default; switch to double quotes iff the value
    contains `'` and does NOT contain `"`; when it contains both, keep single quotes
    and escape the inner `'`. Observed: `repr("it's")` → `"it's"` (doubles),
    `repr('say "hi"')` → `'say "hi"'`, `repr('both \' and "')` → `'both \' and "'`.
  - Escapes: `\t \n \r` are always escaped to `\t`, `\n`, `\r`; backslash doubles
    (`repr("\\")` → `'\\'`); control chars below 0x20 and 0x7f become `\xNN`
    lowercase-hex: `repr("\t\n\x00\x07\r")` → `'\t\n\x00\x07\r'`. Printable
    non-ASCII passes through unescaped in `repr` (UTF-8 default locale):
    `repr("café 中")` → `'café 中'`; `repr("\U0001F600")` → `'😀'`.
    (`ascii()` — used by `!a` — escapes them as `\xe9`/`\U0001f600` but is out of
    v1 corpus scope; spec 05 owns it.)
  - `repr(str)` of a str value NEVER returns the value unchanged when escapes or
    quote flips are required; round-trip property `eval(repr(s)) == s` holds.
  - `str(s)` of a `str` is `s` itself — `print("a\nb")` emits a real newline while
    `print(repr("a\nb"))` emits `a\nb` literally (observed).
- **S-01-43 (MUST)** `repr(bytes)` is `b'…'` with the same quote-selection rule and
  `\xNN` escaping for all non-printable/non-ASCII bytes:
  `repr(b"caf\xc3\xa9")` → `b'caf\xc3\xa9'`; `repr(b"\x00abc'\"\\")` →
  `b'\x00abc\'"\\'`. bytes with both quote kinds prefer doubles: same rule as
  S-01-42.

### 7.4 Containers and objects

- **S-01-44 (MUST)** Container reprs recurse with element reprs and CPython's exact
  punctuation: `repr([1, "a", (2,), {1: None, True: "x"}])` →
  `[1, 'a', (2,), {1: 'x'}]` (no inner spaces after `[`, `, ` between items; dict
  items are `k: v`; 1-tuple keeps its comma `(2,)`). `str(container)` == `repr(container)`
  (`str([1,'a'])` → `[1, 'a']`). Empty containers: `repr([])` → `[]`, `repr(())` →
  `()`, `repr({})` → `{}`, `repr(set())` → `set()`, `repr(frozenset())` → `frozenset()`.
  A self-referential list reprs with a cycle ellipsis: `xs=[1]; xs.append(xs);
  print(repr(xs))` → `[1, [...]]`.
  Set/frozenset non-empty reprs list elements in hash-table order — element ORDER in
  `repr({…})` is UNSPECIFIED (see S-01-25); tests print only 0- or 1-element sets or
  `sorted()` results. A non-empty frozenset reprs as `frozenset({1, 2})`.
- **S-01-45 (MUST)** For instances without `__repr__`: `repr(x)` is
  `<module_or_main.ClassName object at 0xHEXADDR>` where the address is
  UNSPECIFIED (never byte-compared; tests compare only the stable prefix). Default
  `str(x)` falls back to `__repr__`. If `__str__` is defined but `__repr__` is not,
  `str()` uses `__str__` and `repr()` still uses the object fallback — they diverge.
  Observed: `class S` with both → `f"{S()}"` → `STR`, `repr(S())` → `REPR`;
  `repr([S()])` → `[REPR]` (containers recurse with repr, NOT str).

---

## Corpus anchors

Each anchor is the exact script body; expected bytes were produced by running it with
`/opt/homebrew/bin/python3.14` (3.14.7) as a file, `python3.14 <abs-path-to-case>.py`.
The `File "..."` path inside any traceback reflects the invocation path (both impls in
`ap-conform` receive the same path). All anchors exit 0 with empty stderr unless noted.

1. `A01-01-floor-div-mod-signs.py` (S-01-3/4/5) — code:
   `print(7 // 2, -7 // 2, 7 // -2, -7 // -2)` /
   `print(7 % 2, -7 % 2, 7 % -2, -7 % -2)` /
   `print(7.0 // 2.0, -7.0 % 2.0, divmod(7, -2))` →
   stdout `3 -4 -4 3⏎1 1 -1 -1⏎3.0 1.0 (-4, -1)⏎`, exit 0.
2. `A01-02-bigint-no-overflow.py` (S-01-1/2) — code:
   `print(2 ** 200)` / `print((10**50 + 1) * (10**50 - 1))` / `print(-7 ** 2, 3 ** 3 ** 2)` →
   stdout `1606938044258990275541962092341162602522202993782792835301376⏎999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999⏎-49 19683⏎`, exit 0.
3. `A01-03-float-to-int-rounding.py` (S-01-11/12/13) — code:
   `print(int(3.7), int(-3.7), round(2.5), round(3.5), round(0.125, 2))` /
   `print(int("ff", 16), int(" 42 "))` →
   stdout `3 -3 2 4 0.12⏎255 42⏎`, exit 0.
4. `A01-04-float-repr-shortest.py` (S-01-41) — code:
   `print(repr(0.1), repr(1e16), repr(1e-7), repr(1.5e308), repr(float("inf")), repr(float("nan")), repr(-0.0))` →
   stdout `0.1 1e+16 1e-07 1.5e+308 inf nan -0.0⏎`, exit 0.
5. `A01-05-str-repr-quotes-escapes.py` (S-01-42) — code:
   `print(repr("it's"), repr('say "hi"'), repr('both \' and "'))` /
   `print(repr("\t\n\x00\x07\r"))` →
   stdout `"it's" 'say "hi"' 'both \' and "'⏎'\t\n\x00\x07\r'⏎` (bytes as produced; the
   second line contains literal backslash-t etc.), exit 0.
6. `A01-06-str-slice-immutable.py` (S-01-14/15) — code:
   `s = "abcdefgh"` / `print(s[1:4], s[-3:], s[::-1], repr(s[10:20]))` /
   `print(len("café"), len("a\u0301"))` →
   stdout `bcd fgh hgfedcba ''⏎4 2⏎`, exit 0.
7. `A01-07-str-bytes-separation.py` (S-01-17/18) — code:
   `print("café".encode("utf-8"), b"caf\xc3\xa9".decode("utf-8"))` /
   `try: "x" + b"y"` / `except TypeError as e: print("TypeError:", e)` →
   stdout `b'caf\xc3\xa9' café⏎TypeError: can only concatenate str (not "bytes") to str⏎`, exit 0.
8. `A01-08-dict-order-equality-collision.py` (S-01-21/22/23) — code:
   `d = {"b": 1, "a": 2}` / `d["b"] = 3` / `print(list(d), {"a": 1, "b": 2} == {"b": 2, "a": 1})` /
   `print({1: "a", 1.0: "b", True: "c"})` →
   stdout `['b', 'a'] True⏎{1: 'c'}⏎`, exit 0.
9. `A01-09-truthiness-table.py` (S-01-27) — code:
   `print(bool([]), bool(""), bool(0), bool(0.0), bool(None), bool(()), bool({}), bool("0"), bool([0]))` →
   stdout `False False False False False False False True True⏎`, exit 0.
10. `A01-10-hash-contract.py` (S-01-35/36) — code:
    `print(hash(42) == 42, hash(True), hash(1) == hash(1.0), hash(0.0) == hash(-0.0), hash(-1))` →
    stdout `True 1 True True -2⏎`, exit 0.
11. `A01-11-repr-containers-empty.py` (S-01-23/44) — code:
    `print(repr([1, "a", (2,), {1: None, True: "x"}]))` /
    `print(repr(()), repr(set()), repr(frozenset()), repr(None), repr(True), repr(False))` →
    stdout `[1, 'a', (2,), {1: 'x'}]⏎() set() frozenset() None True False⏎`, exit 0.
12. `A01-12-fstring-formatting.py` (S-01-16) — code:
    `name = "world"` / `x = 3.256` / `print(f"{name}, {x:.2f}, {2 + 3}, {x!r}")` /
    `print(f"{42:>6}|{42:06}|{1234567:,}")` →
    stdout `world, 3.26, 5, 3.256⏎    42|000042|1,234,567⏎`, exit 0.

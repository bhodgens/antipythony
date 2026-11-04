# Spec 03 — Classes

Part of the antipythony spec pack (plan 00.05). Normative source for test leaf T-06
(classes) and implementation leaf 02.03 (ap-obj classes/protocols). Every behavior
below is CPython 3.14.7 semantics; error-message strings quoted in this doc were
observed by running the pinned snippets on `/opt/homebrew/bin/python3.14`
(3.14.7) and are part of the byte-exact conformance gate (compat-contract).
Clause ids `S-03-nn` are what corpus cases cite in their header comments.

## Amendment register

| A-id | Date | Clause | What changed | Evidence (live probe, CPython 3.14.7) |
| --- | --- | --- | --- | --- |
| A-3 | 2026-10-16 | S-03-27 | Hook-order correction: `__set_name__` and `B.__init_subclass__` run INSIDE `type.__new__` (before `M.__new__` returns), in that order, and `M.__init__` runs last; the old "then … run after init" order is wrong. | `class M(type)` tracing `__new__` start/end vs descriptor `__set_name__` and base `__init_subclass__` prints → `new-start / set_name / init_subclass / new-end / init` |
| A-4 | 2026-10-16 | S-03-34 | "(nested tuples allowed)" deleted: on 3.14.7 every nested-tuple except target raises the catching-classes `TypeError` at catch time. Flat tuples of exception classes still match any element. | `except ((KeyError, ValueError),):` while a `KeyError` is in flight → `TypeError: catching classes that do not inherit from BaseException is not allowed` (all nesting variants); `except (KeyError, ValueError):` catches normally |

Scope: user-visible semantics of class definition, instantiation, attribute lookup,
MRO, descriptors, special-method dispatch, operator protocol, metaclass hooks,
inheritance, `isinstance`/`issubclass`, and the exception class hierarchy. Value
repr/str/identity semantics for builtin types are in spec 01; exception *propagation
and traceback text* are in spec 02 — this doc pins exception *class structure* only.

---

## 3.1 Class definition

**S-03-01 (MUST)** A `class` statement evaluates its base-argument expressions and
keyword arguments, then executes the class body in a fresh namespace, and finally
creates a class object via the resolved metaclass:
`metaclass(name, bases, namespace)`. The name is bound in the *enclosing* scope to
the resulting object. `class A:` with no bases is exactly `class A(object):`.
Observed: `class A: pass; print(A.__bases__, issubclass(A, object))` →
`(<class 'object'>,) True`.

**S-03-02 (MUST)** Metaclass resolution for `class C(B, ..., metaclass=M): ...` with
one or more bases: the *most derived* metaclass among `M` (if given) and
`type(base)` for every base is selected; if `M` is not a subclass of a base's
metaclass, class creation fails with `TypeError`. With zero bases, an explicitly
given `metaclass=M` is used verbatim — even when `M` is not a subclass of `type`
(creation then fails only if calling `M` fails). Observed (3.14.7):
- `class NotAMeta: pass` then `class C(metaclass=NotAMeta): pass` →
  `TypeError: NotAMeta() takes no arguments` (no "must be a subclass of type" check).

**S-03-03 (MUST)** The class body executes as a normal code block: names assigned at
body level become entries of the namespace passed to the metaclass; nested function
definitions inside the body do NOT close over class-scope names (class scope is not
an enclosing scope for functions). Decorators in the body are applied at body
execution time, in source order, bottom-up per target.

**S-03-04 (MUST)** The created class object carries at least: `__name__` (the simple
name string), `__qualname__` (dotted, including enclosing classes; not a key of the
final `__dict__` in 3.14 — set as a slot), `__module__` (the defining module's
`__name__`, e.g. `'__main__'` for a top-level script), `__dict__` (a read-only
mapping of the class namespace — mutation raises
`TypeError: 'mappingproxy' object does not support item assignment`), `__bases__`,
`__mro__`, and `type(C)` equal to the resolved metaclass.

## 3.2 Instantiation

**S-03-05 (MUST)** `C(*args, **kwargs)` is dispatched as
`type(C).__call__(C, *args, **kwargs)`. Calling a non-type raises the ordinary call
protocol of its type (observed: `5()` →
`TypeError: 'int' object is not callable`, plus a compile-time `SyntaxWarning:
'int' object is not callable; perhaps you missed a comma?`).

**S-03-06 (MUST)** The default `type.__call__` path is: (1) `obj = cls.__new__(cls,
*args, **kwargs)`; (2) if `isinstance(obj, cls)` holds, (3) call
`type(obj).__init__(obj, *args, **kwargs)`. If `__new__` returns something that is
not an instance of `cls`, `__init__` is NOT called on it and the returned object is
the result of the call. Observed:

```python
class A:
    def __new__(cls):
        print("new")
        return "not an A"
    def __init__(self):
        print("init")
print(A())
```
→ stdout `new` then `not an A`; `type(A()).__name__` is `str`. (`__new__` returning
an instance of a *subclass* of `cls` still triggers `__init__`.)

**S-03-07 (MUST)** Arity cross-check on the no-custom-`__new__` fast path: when
`cls.__new__` is `object.__new__` and `cls.__init__` is not overridden, calling
`C(x)` raises `TypeError: C() takes no arguments`; when `__init__` IS overridden,
the arguments reach `__new__` and the binding error surfaces from there (observed:
`TypeError: A.__new__() takes 1 positional argument but 2 were given`).

**S-03-08 (MUST)** `__init__` must return `None`; any other return value raises
`TypeError: __init__() should return None, not 'int'` (type name interpolated).

## 3.3 Attribute lookup on instances

**S-03-09 (MUST)** `obj.name` is resolved through `type(obj).__getattribute__(obj,
name)`. `object.__getattribute__` finds the first `name` in `mro(type(obj))` whose
class dict contains it (in MRO order; a `__getattr__` defined in the class body
counts as a found entry):
- If that entry is a **data descriptor** (its type defines both `__get__` and
  `__set__`, or both `__get__` and `__delete__`) → return `descr.__get__(obj,
  type(obj))`, *even if* `name` is in `obj.__dict__`.
- Else if `name` is in `obj.__dict__` → return that value directly.
- Else if the entry exists → return `descr.__get__(obj, type(obj))` (functions
  become bound methods; non-data descriptors get the instance-dict miss).
- Else, if the class defines `__getattr__` → call `type(obj).__getattr__(obj, name)`;
  otherwise raise `AttributeError` with message
  `'C' object has no attribute 'nope'` (single-quoted type and name).
Observed: a `property` shadows an instance-dict entry (`print(c.x)` → `prop`);
a non-data descriptor is shadowed by the instance dict (→ `99`); with a class
`__getattr__`, misses yield `"gen:missing2"` while `c.explicit` (in instance dict)
yields `1`.

**S-03-10 (MUST)** `obj.name = v`: `object.__setattr__` finds the first MRO entry
that is a data descriptor → calls `descr.__set__(obj, v)` (instance dict NOT
consulted); otherwise writes/overwrites `obj.__dict__[name]`. A class-level
non-data descriptor or plain class attribute does **not** intercept instance
assignment — it is shadowed in the instance dict. Observed: assigning to a
`property` without a setter raises
`AttributeError: property 'x' of 'C' object has no setter`; a class-level
`property` itself IS writable on the class object (`C.x = 5` succeeds); assigning
to an undeclared attribute of a `__slots__` instance raises the slots message
(S-03-13).

**S-03-11 (MUST)** `del obj.name`: data descriptor with `__delete__` →
`descr.__delete__(obj)`; else `obj.__dict__[name]` must exist or
`AttributeError` (observed message identical in shape to S-03-09; note that
shadowing deletion of an *inherited* attribute reports the instance's class:
`'C' object has no attribute 'x'` even when `x` exists on the class).
`__delattr__` override is honored.

**S-03-12 (MUST)** Attribute lookup on a **class object** `C.name`:
(1) search `mro(C)` for a data descriptor → `__get__(None, C)`; (2) `C.__dict__`
(plus `__getattr__` fallback on C's own metaclass if absent); (3) search
`mro(C)` for a non-data descriptor / `__objclass__`-carrying entry → invoke its
`__get__(None, C)`; (4) metaclass *attribute* lookup: first `mro(type(C))` entry
whose class dict contains `name`, then the metaclass instance dict. Observed:
with `class M(type): mattr = 7` and `class C(metaclass=M)`, `C.mattr` → `7` but
`C().mattr` → `AttributeError: 'C' object has no attribute 'mattr'` (no
metaclass fallback for instances). `type`'s own slots (`__name__`, `__mro__`,
`__bases__`, …) resolve before any user class attribute of the same name.

## 3.4 `__slots__`

**S-03-13 (MUST)** A class defining `__slots__` (a non-empty iterable of strings)
has instances with **no** `__dict__`; each slot name becomes a data descriptor on
the class. Setting an undeclared attribute raises
`AttributeError: 'A' object has no attribute 'z' and no __dict__ for setting new
attributes`; reading one raises the ordinary
`AttributeError: 'A' object has no attribute 'z'`. An element that is not a string
raises `TypeError: __slots__ items must be strings, not 'int'` at class creation.
A `__slots__` entry conflicting with a same-named class variable raises
`ValueError: 'x' in __slots__ conflicts with class variable`.

**S-03-14 (MUST)** `__dict__` may be requested as a slot member (`__slots__ =
("x", "__dict__")` re-enables the instance dict). A subclass that declares no
`__slots__` gets an instance dict regardless of its bases, so parent slot storage
remains reachable via both `b.x` (descriptor) and `vars(b)`.

## 3.5 MRO and C3 linearization

**S-03-15 (MUST)** Every class has a linearized MRO computed at creation:
`mro(object) = (object,)`;
`mro(C) = C + merge(mro(B1), ..., mro(Bn), (B1, ..., Bn))` where `merge` repeatedly
takes the first head `H` of some list that is not in the tail (anywhere-but-head
position) of any list, removes `H` from the head of every list containing it, and
fails with `TypeError: Cannot create a consistent method resolution order (MRO) for
bases A, B` when no such head exists. Duplicate bases raise earlier, during base
processing: `TypeError: duplicate base class X`. Attribute lookup scans this MRO in
order (S-03-09/12); `C.__mro__` exposes the tuple; `object` is always the last
element.

Worked examples — each asserts against observed CPython 3.14.7 output:

| # | Hierarchy | `__mro__` names (observed) |
| --- | --- | --- |
| E1 | `O`; `B(O)`; `C(O)`; `D(B, C)` | `['D', 'B', 'C', 'O', 'object']` |
| E2 | `A`; `B(A)`; `C(A)`; `D(B, C)`; `E(D, C)` | `['E', 'D', 'B', 'C', 'A', 'object']` |
| E3 | `A`; `B(A)`; `C(A)`; `D(A)`; `X(B, C, D)` | `['X', 'B', 'C', 'D', 'A', 'object']` |

E1 merge trace: heads `B, C, O` — `B` is not in any tail → take `B`; then heads
`C, O` → `C`; then `O` vs `O,object` → `O`; then `object`. E3 shows why `A` cannot
precede `B`,`C`,`D`: `A` appears in the tail of `mro(B)`.

**S-03-16 (MUST)** `class C(B1, ..., Bn)` is rejected with the inconsistent-MRO
`TypeError` (S-03-15) whenever the linearization does not exist; a redundant
ordering that C3 can linearize (e.g. `class D(B, C, A)` where `A` is a base of both)
is accepted, yielding `['D', 'B', 'C', 'A', 'object']`.

## 3.6 `super()`

**S-03-17 (MUST)** `super(S, obj)` requires `isinstance(obj, S)` or
`isinstance(obj, type) and issubclass(obj, S)`, else
`TypeError: super(type, obj): obj (instance of int) is not an instance or subtype
of type (A).`. The two-arg form may be constructed anywhere; bare `super()` requires
the enclosing frame to have a first argument `__class__` cell — outside a method
body it raises `RuntimeError: super(): no arguments` at call time.

**S-03-18 (MUST)** `super(S, obj).name` searches `mro(type(obj))` strictly **after**
the first occurrence of `S`, applying the full descriptor protocol of S-03-09/12;
it does not consult `S`'s own dict nor use static parentage. The canonical observed
chase: diamond `A`; `B(A)`; `C(A)`; `D(B, C)` each defining `m` and calling
`super().m()` → `D().m()` prints `D B C A` (MRO order — `C` before `A`, never
`B`→`A` early), with `D.__mro__` names `['D', 'B', 'C', 'A', 'object']`.

**S-03-19 (MUST)** For a classmethod body, `super()` resolves the second argument as
the *class* (not the instance): `class B(A)` with a `@classmethod m` calling
`super().m()` on `A`'s classmethod resolves with `type(obj)`-as-`B` semantics, so
`cls` propagates — observed: `B.m()` → `B->B` when the parent returns
`cls.__name__`.

## 3.7 Descriptors

**S-03-20 (MUST)** An object whose *type* defines `__get__` is a descriptor. A
descriptor whose type also defines `__set__` and/or `__delete__` is a **data
descriptor**; the class attribute is the descriptor object itself
(`type(C.__dict__["x"])` is `property`, `classmethod`, `staticmethod`, or the
user class) and access routes through `__get__(obj_or_None, objtype)` per
S-03-09/S-03-12.

**S-03-21 (MUST)** `property(fget=None, fget-only→read-only, ...)`: `C.p` →
the property object; `c.p` → `fget(c)`; missing setter → `AttributeError`
(`property 'x' of 'C' object has no setter`). `classmethod` binds `cls` (metaclass
of the instance's class) on both `C.cm()` and `C().cm()` → both give the same
`cls`-bound callable. `staticmethod` returns the underlying function unchanged from
both `C.sm` and `c.sm`. Plain functions are non-data descriptors producing a bound
`method` whose `==` compares `(__func__, __self__)` but whose identity is fresh on
each attribute access (`c.f == c.f` → `True`; `c.f is c.f` → `False`).

## 3.8 Special-method (dunder) lookup

**S-03-22 (MUST)** Implicit special-method invocation looks the method up on
`type(obj)` via the MRO, **bypassing the instance `__dict__`** (and instance-level
monkeypatching entirely). Observed: with `a.__str__`/`a.__repr__` set in the
instance dict, `str(a)` still yields the type default
`<__main__.A object at 0x…>`; with a class `__len__` overridden by
`c.__len__ = lambda: 99`, `len(c)` still returns `3`.

**S-03-23 (MUST)** The bypassed set covers at least: `__new__`, `__init__`,
`__call__`, `__str__`, `__repr__`, `__len__`, `__iter__`, `__next__`, `__contains__`,
`__getitem__`, `__setitem__`, `__delitem__`, `__getattr__`, `__getattribute__`,
`__setattr__`, `__delattr__`, all binary/reflected/unary/augmented operator and
comparison slots (§3.9), `__enter__`, `__exit__`, `__await__`, `__aiter__`,
`__anext__`, `__index__`, `__hash__`, `__bool__`. Explicit lookup
(`obj.__str__()`) still follows normal rules.

**S-03-24 (MUST)** When no implicit special method exists: `len(x)` →
`TypeError: object of type 'A' has no len()`; `iter(x)`/`for` →
`TypeError: 'A' object is not iterable`; `list(x)` on an object with only
`__getitem__` uses the legacy sequence protocol, iterating `0,1,2,…` until
`IndexError` (observed: `list(N())` → `[0, 1, 2]`). `str(x)` falls back to
`repr(x)` and vice versa only via `object.__str__`→`__repr__`; a class defining
only `__str__` keeps the default repr. `__str__`/`__repr__` returning a
non-string raises `TypeError: __str__ returned non-string (type int)`.

## 3.9 Binary operator dunder table

**S-03-25 (MUST)** For `a op b` with a primary slot `__X__` and reflected slot
`__rX__`: (1) if `type(b)` is a **proper subclass** of `type(a)` and overrides
`__rX__`, try `__rX__(b, a)` first; (2) otherwise try `__X__(a, b)`; (3) if that
returns `NotImplemented` (or is absent), try `__rX__(b, a)`; (4) if both return
`NotImplemented`/are absent, raise the appropriate `TypeError`. A slot returning
`NotImplemented` means "defer", never a truthy result: `!=` is the negation of
whichever `__eq__` ran (observed: `a == b` → `'reflected-eq'` (truthy string) with
`a != b` → `False`; default identity inequality holds when only `__eq__` is
defined: `1 == R()` → `'reflected-eq'`, so `1` deferring is not needed —
`int.__eq__(1, R())` returns `NotImplemented`).

| op | primary slot | reflected slot | failure message (observed) |
| --- | --- | --- | --- |
| `a + b` | `__add__` | `__radd__` | `TypeError: unsupported operand type(s) for +: 'int' and 'str'` |
| `a - b` | `__sub__` | `__rsub__` | (same shape with `-`) |
| `a * b` | `__mul__` | `__rmul__` | (same shape with `*`) |
| `a / b` | `__truediv__` | `__rtruediv__` | (same shape with `/`) |
| `a // b` | `__floordiv__` | `__rfloordiv__` | (same shape with `//`) |
| `a % b` | `__mod__` | `__rmod__` | (same shape with `%`) |
| `a ** b` | `__pow__` | `__rpow__` | (same shape with `**`) |
| `a @ b` | `__matmul__` | `__rmatmul__` | (same shape with `@`) |
| `a & b`, `a \| b`, `a ^ b`, `a << n`, `a >> n` | `__and__`, `__or__`, `__xor__`, `__lshift__`, `__rshift__` | `__r*__` counterparts | (same shape) |
| `a < b`, `a <=`, `a >`, `a >=` | `__lt__`, `__le__`, `__gt__`, `__ge__` | `__gt__`, `__ge__`, `__lt__`, `__le__` (swapped + reversed) | `TypeError: '<' not supported between instances of 'int' and 'A'` |
| `a == b`, `a != b` | `__eq__` (default: identity) | `__eq__` (symmetric) | never fails (identity default) |

Observed rule-1 case: `class L: __add__` / `class R(L): __radd__` →
`L() + R()` → `R.radd` (proper subclass reflected first) while `R() + L()` →
`L.add` (left primary, right is not a subclass). Built-in fast paths may
short-circuit before user reflection, e.g. `[1] + (1,)` →
`TypeError: can only concatenate list (not "tuple") to list` (list's own message,
no reflected attempt against `tuple`).

**S-03-26 (MUST)** Augmented `a op= b`: try `type(a).__iX__` first; if absent or
`NotImplemented`, fall back to the full `a op b` machinery of S-03-25 and *rebind*
the target name (so `a` may point to a new object). Unary: `-a`→`__neg__`,
`+a`→`__pos__`, `~a`→`__invert__`, `abs(a)`→`__abs__`, `round(a)`→`__round__`,
`hash(a)`→`__hash__`, `bool(a)`→`__bool__` then `__len__` then True (spec 01).
`in`, `not in` use `__contains__` then iteration fallback.

## 3.10 Metaclass hook points

**S-03-27 (MUST)** Creation pipeline for `class C(B, ..., metaclass=M)`:
`M.__prepare__(M, name, bases)` → namespace mapping; body executes into it;
`M.__new__(M, name, bases, ns)` → class object — and INSIDE the default
`type.__new__`, before it returns, first the `__set_name__` hooks on ns
entries run, then `B.__init_subclass__(cls, **kw)` runs; only after `M.__new__`
returns does `M.__init__(cls, name, bases, ns)` run (Amendment A-3 — the
previously recorded "init first, hooks after" order is contradicted by the
live reference). Observed order with a tracing metaclass + tracing
`__set_name__` descriptor + tracing `__init_subclass__` base:
`M.__new__(start) → set_name → __init_subclass__ → M.__new__(end) →
M.__init__`.

**S-03-28 (MUST)** Instantiation path: `C(...)` → `type(C).__call__(C, ...)` →
`C.__new__` → `__init__` (S-03-05/06). A metaclass `__call__` override wraps every
instantiation of its classes: observed `call C () {}`, `new`, `init`.

**S-03-29 (MUST)** `__init_subclass__` is an implicit classmethod on every class:
at the end of creating a direct subclass, it is called on that subclass with the
class-statement keyword arguments (minus `metaclass`). The default
`object.__init_subclass__` accepts no keyword arguments — `class Sub(Base, oops=1)`
with no override raises `TypeError: Sub.__init_subclass__() takes no keyword
arguments`; an override should consume its own keyword-only args and forward the
rest via `super().__init_subclass__(**kw)` (observed working example sets
`cls.tag = tag` on the subclass; `Base.__subclasses__()` then contains `Sub`).

## 3.11 Inheritance of concrete methods

**S-03-30 (MUST)** A method defined in a base and not overridden is looked up through
the MRO and bound with the *runtime* class as `self`/`cls`: calling an inherited
method on `D()` sees `type(self) is D`; `super()` inside inherited code resolves
relative to the defining class's position in `type(self).__mro__`, not the lexical
owner (the S-03-18 chase is the canonical case: `A.m` reached via three inherited
hops all bound to one `D` instance). Overriding in a subclass replaces only that
subclass's view; siblings are unaffected.

## 3.12 isinstance / issubclass

**S-03-31 (MUST)** `isinstance(x, T)` is True iff `type(x) is T` or `T` appears in
`mro(type(x))`; `issubclass(A, B)` iff `B` appears in `mro(A)` (every class is a
subclass of itself and of `object`; `object.__bases__ == ()`). The second argument
may be a type, a tuple of types (any match), or a `types.UnionType` (X | Y).
`isinstance` dispatches through the *metaclass* `__instancecheck__` (and
`issubclass` through `__subclasscheck__`) when defined (observed: custom
`__instancecheck__` returning `obj == 42` → `isinstance(42, Weird)` True).
Bad second args: `isinstance(1, 2)` → `TypeError: isinstance() arg 2 must be a
type, a tuple of types, or a union`; `issubclass(1, int)` →
`TypeError: issubclass() arg 1 must be a class`. `x.__class__` is the type used for
implicit special-method lookup; for ordinary instances `x.__class__ is type(x)`
(observed `True`).

## 3.13 Exception class hierarchy

**S-03-32 (MUST)** Rooted at `BaseException` (a direct subclass of `object`), the
direct subclasses in CPython 3.14 are exactly (observed, sorted):
`BaseExceptionGroup`, `Exception`, `GeneratorExit`, `KeyboardInterrupt`,
`SystemExit`. Direct subclasses of `Exception` (observed, sorted):
`ArithmeticError`, `AssertionError`, `AttributeError`, `BufferError`, `EOFError`,
`ExceptionGroup`, `ImportError`, `InterpreterError`, `LookupError`, `MemoryError`,
`NameError`, `OSError`, `PatternError`, `ReferenceError`, `RuntimeError`,
`StopAsyncIteration`, `StopIteration`, `SyntaxError`, `SystemError`, `TypeError`,
`ValueError`, `Warning`. Notable chains (MRO verified):
`ValueError → Exception → BaseException → object`;
`ZeroDivisionError → ArithmeticError → Exception → …`; `ExceptionGroup → Exception`
so `except Exception` catches it; `BaseExceptionGroup → BaseException` so it does
not; `KeyboardInterrupt`/`SystemExit`/`GeneratorExit` are `BaseException` but not
`Exception` (observed `issubclass(...)` tuple → `True True False False False`
for `(Exception⊂Base, KI⊂Base, KI⊂Exc, SystemExit⊂Exc, ValueError⊂ArithmeticError)`).

**S-03-33 (MUST)** `raise X` requires `X` to be an instance of a class deriving
from `BaseException`; raising a bare class `X` instantiates it (`raise ValueError`
→ `ValueError()`). Anything else raises `TypeError: exceptions must derive from
BaseException` at the raise site. Observed `str(e)`/`repr(e)`/`e.args` for
`ValueError("a", 2)`: `('a', 2)` / `ValueError('a', 2)` / `('a', 2)` — `args`
tuple semantics are spec 01/02 territory; the hierarchy and catch rules are here.

**S-03-34 (MUST)** An `except` clause target must be a class deriving from
`BaseException`, or a FLAT tuple whose *every* element is such a class — nested
tuples are REJECTED (Amendment A-4: the previously claimed "nested tuples
allowed" is false on 3.14.7); anything else is checked at catch time and raises
`TypeError: catching classes that do not inherit from BaseException is not
allowed`. Observed: with `class NotExc: pass`, `except NotExc:` while a
`KeyError` is in flight → that `TypeError` (it does NOT silently skip the
clause); `except ((KeyError, ValueError),):` and every nesting variant raise
the same `TypeError` at catch time. A flat tuple target matches any element
(`except (KeyError, ValueError):` catches `ValueError`).

---

## Corpus anchors

Snippets verified on CPython 3.14.7 (`/opt/homebrew/bin/python3.14`). Expected
output is byte-exact stdout (stderr empty and exit code 0 unless stated); the
`0x…` in any default repr is a wildcard (address differs per run). Each corpus
case header cites the clause ids it exercises.

A1 (S-03-01, S-03-30) — `class A: pass; print(A.__bases__, issubclass(A, object))`
→ `(<class 'object'>,) True`, rc 0.

A2 (S-03-06) — `class A:\n def __new__(cls): print("new"); return "not an A"\n def __init__(self): print("init")\n a = A(); print(type(a).__name__, a)`
→ `new\nnot an A`, rc 0 (`__init__` never printed).

A3 (S-03-09 data-descriptor priority) —
`class C:\n @property\n def x(self): return "prop"\nc = C(); c.__dict__["x"] = 99; print(c.x)`
→ `prop`, rc 0.

A4 (S-03-09 non-data priority) —
`class ND:\n def __get__(self, o, t=None): return "nd"\nclass C: x = ND()\nc = C(); c.__dict__["x"] = 99; print(c.x)`
→ `99`, rc 0.

A5 (S-03-12 metaclass fallback) —
`class M(type): mattr = 7\nclass C(metaclass=M): pass\nprint(C.mattr)\ntry: C().mattr\nexcept AttributeError as e: print("AttributeError:", e)`
→ `7\nAttributeError: 'C' object has no attribute 'mattr'`, rc 0.

A6 (S-03-13) —
`class A: __slots__ = ("x",)\na = A(); a.x = 1\ntry: a.z = 2\nexcept AttributeError as e: print("AttributeError:", e); print(hasattr(a, "__dict__"))`
→ `AttributeError: 'A' object has no attribute 'z' and no __dict__ for setting new
attributes` then `False`, rc 0.

A7 (S-03-15/18) —
`class A: pass\nclass B(A): pass\nclass C(A): pass\nclass D(B, C): pass\nprint([t.__name__ for t in D.__mro__])`
→ `['D', 'B', 'C', 'A', 'object']`, rc 0.

A8 (S-03-18) —
`class A:\n def m(self): print("A")\nclass B(A):\n def m(self): print("B"); super().m()\nclass C(A):\n def m(self): print("C"); super().m()\nclass D(B, C):\n def m(self): print("D"); super().m()\nD().m()`
→ `D\nB\nC\nA`, rc 0.

A9 (S-03-16) —
`class A: pass\nclass B(A): pass\ntry:\n class X(A, B): pass\nexcept TypeError as e: print("TypeError:", e)`
→ `TypeError: Cannot create a consistent method resolution order (MRO) for bases A, B`, rc 0.

A10 (S-03-22) —
`class C:\n def __len__(self): return 3\nc = C(); c.__len__ = lambda: 99; print(len(c))`
→ `3`, rc 0.

A11 (S-03-25) —
`class L:\n def __add__(self, o): return "L.add"\nclass R(L):\n def __radd__(self, o): return "R.radd"\nprint(L() + R()); print(R() + L())`
→ `R.radd\nL.add`, rc 0.

A12 (S-03-34) —
`class NotExc: pass\ntry:\n try: raise KeyError()\n except NotExc: pass\nexcept TypeError as e: print("TypeError:", e)`
→ `TypeError: catching classes that do not inherit from BaseException is not allowed`, rc 0.

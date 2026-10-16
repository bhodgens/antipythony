# Spec 04 — Modules and Imports

Part of the antipythany spec pack (plan 00.05). Normative source for test leaf T-01
(parse diagnostics, import forms) and for module/import semantics in 01.x (lowering)
and 02.x (runtime). Error strings quoted below were observed by running the pinned
snippets on `/opt/homebrew/bin/python3.14` (3.14.7) and are part of the byte-exact
conformance gate. Clause ids `S-04-nn` are what corpus cases cite in their headers.

Scope: the module object, the `import` statement forms and their binding effects,
v1 import resolution order, the main-module namespace, the module cache, from-import
value-copy semantics, and one-time module execution. Exception *hierarchy* facts for
`ModuleNotFoundError`/`ImportError` live in spec 03 §3.13; scoping of names inside
functions is spec 02.

---

## 4.1 The module object

**S-04-01 (MUST)** An imported (or main) module is an object of type `module`
(`type(m).__name__ == 'module'`) whose public face is its mutable namespace
dictionary: attribute get/set/del on the module read/write that dict
(observed: `counter.extra = 7; print(counter.extra)` → `7`; `vars(m)` and
`m.__dict__` expose the same mapping). Module attributes are ordinary bindings —
there is no per-module immutability for user modules.

**S-04-02 (MUST)** Every module namespace created by the v1 importer carries the
dunder keys `__name__`, `__doc__`, `__package__`, `__loader__`, `__spec__`,
`__file__`, `__cached__` for source-loaded modules, plus `__builtins__`. Observed
key set (sorted, dunders excluding `__builtins__`) for both `__main__` and an
imported file module:
`['__cached__', '__doc__', '__file__', '__loader__', '__name__', '__package__', '__spec__']`.

**S-04-03 (MUST)** `__name__` is the module's full dotted name; `__package__` is
the containing package name or `''` for a top-level module, and `None` for the main
module run as a script (observed `print(repr(__package__))` → `None` at top level
of a script; `pkg.a.deep.__package__` → `pkg.a`; `pkg.__package__` → `pkg`).
`__doc__` is the module's docstring or `None`.

**S-04-04 (MUST)** `__file__` is set for file-loaded modules to the script/source
path as CPython resolves it (on macOS the realpath, e.g.
`/private/tmp/spec_verify/counter.py`); `__cached__` is the derived `.pyc` path.
`__file__` is NOT defined for `python -c` execution — reading it raises `NameError:
name '__file__' is not defined. Did you mean: '__name__'?`. Built-in/v1-registry
modules have no `__file__` attribute at all (observed: `hasattr(sys, '__file__')`
→ `False`). v1: our registry-resolved stdlib modules MUST mirror this — no
`__file__` key unless a source path is genuinely associated.

**S-04-05 (MUST)** `repr(module)` is `<module 'NAME' from 'PATH'>` for a
file-loaded module (observed: `<module 'counter' from
'/private/tmp/spec_verify/counter.py'>`), `<module 'NAME' (built-in)>` for a
built-in (observed: `<module 'sys' (built-in)>`). The main module's repr is
`<module '__main__' from '/abs/path/script.py'>`.

## 4.2 Import statement forms and binding effects

**S-04-06 (MUST)** `import a.b.c` imports the whole chain and binds ONLY the top
name `a` in the importing namespace. Observed (script in dir containing
`pkg/a/deep.py`, `pkg` and `pkg/a` having `__init__.py`):
`import pkg.a.deep` then `sorted(k for k in dir() if k in ('pkg','a','deep'))` →
`['pkg']`; `print(a)` → `NameError: name 'a' is not defined`; `pkg.a.deep.val` →
`42`. Access to `pkg.a` and `pkg.a.deep` works because the importer sets each
submodule as an attribute of its parent package after load (observed
`hasattr(pkg, 'a')` → `True`).

**S-04-07 (MUST)** `from m import n` (also `from a.b import n`, `from a import b`)
binds ONLY `n` in the importing namespace — `m`/`b` are not bound unless also
imported (observed: `from pkg.a.deep import val` then `print(deep)` →
`NameError: name 'deep' is not defined`). If `n` is a submodule, the parent chain
is imported and `n` resolves to the submodule module object (observed:
`from pkg.a import deep` → `deep.val` → `42`).

**S-04-08 (MUST)** `as` rebinding: `import a.b.c as d` binds ONLY the alias `d`
(no top name bound — observed `print(pkg)` after `import pkg.a.deep as d` →
`NameError: name 'pkg' is not defined`); `from m import n as q` binds `q`.

**S-04-09 (MUST)** Imports inside a function body bind locally (not globally) and
hit the same cache (observed: local `import counter` inside `f()` succeeds;
`'counter' in sys.modules` → `True` afterwards).

**S-04-10 (MUST)** `from m import *` is a SyntaxError anywhere but module level:
`SyntaxError: import * only allowed at module level` (compile-time; the caret line
`from counter import *` + `^` is part of byte-exact stderr, spec 02 traceback
formatting). At module level it imports: (a) every name in `m.__all__` if that
attribute exists; (b) otherwise every name in `m` not starting with `_`
(observed: with `__all__ = ['public']` → bound names `['public']` even though
`_hidden` exists; without `__all__` in a module with `alpha`, `beta`, `_gam` →
`['alpha', 'beta']`).

**S-04-11 (MUST)** Binding effects are lexical at execution of the import
statement; `del name` removes only the local binding, not the cache entry:
observed after `import counter; del counter` → `counter.n` →
`NameError: name 'counter' is not defined`, while `'counter' in sys.modules` stays
`True` and a subsequent plain `import counter` restores the binding WITHOUT
re-running module code (see S-04-16).

## 4.3 v1 import resolution order

**S-04-12 (MUST)** For antipythany v1, resolution of an import name X proceeds in
exactly two steps, in order: (1) the **built-in/registry namespace** — the static
set of stdlib v1 modules enumerated in spec 05 plus `sys`, `builtins`, `__main__`
machinery; (2) the **filesystem search** over the sys.path-equivalent list seeded
with the script's directory (CPython: `sys.path[0]` is the containing directory of
the entry script — observed `/private/tmp/spec_verify` for a script in that dir).
The main module is pre-inserted in the cache under `__main__` before its code
runs; no filesystem search applies to it.

**S-04-13 (MUST)** A module found by step (2) is located as `NAME.py` on a search
entry, or as `NAME/__init__.py` (package: importing `NAME` runs `__init__.py` and
gives the object a `__path__` attribute; `NAME.sub` then searches inside those
directories — observed: `import pkg` printed nothing extra for an empty
`__init__.py`, `pkg.__path__` → list containing the pkg dir; `import pkg.a.deep`
printed `pkg.a init` then `deep ran`, in that order, parents before child).

**S-04-14 (MUST)** A name that resolves to neither registry nor filesystem yields
`ModuleNotFoundError` (subclass of `ImportError`, spec 03 S-03-32) with message
`No module named 'X'` where `X` is the full attempted dotted name as far as the
importer got, and with `.name == 'X'` (observed: top-level miss →
`ModuleNotFoundError: No module named 'nosuchmodule' | name attr: 'nosuchmodule'`;
submodule miss `import pkg.nosuch` → `No module named 'pkg.nosuch'`, name
`'pkg.nosuch'`; `from nosuch import thing` → same top-level message).

**S-04-15 (MUST)** `from m import n` where `m` loads but lacks `n` (and `n` is not
an importable submodule) raises `ImportError` with message
`cannot import name 'n' from 'm' (PATH)` (observed: `ImportError: cannot import
name 'nosuch' from 'counter' (/private/tmp/spec_verify/counter.py)`).

**S-04-16 (v1 scope pin, MUST)** Only the main module plus spec-05 stdlib-v1
modules and pure-Python modules found on the search path are resolvable. Any
import that CPython would satisfy with a C extension / third-party binary wheel is
out of scope for v1 and MUST produce the fail-loud compile-time diagnostic
defined in `docs/compat-contract.md` (naming the module, pointing at the
extension policy doc) — never a silent stub, never a runtime fallback. Pure
third-party *source* on the search path resolves through step (2) like any other
module but its own binary dependencies still hit the fail-loud path. (Test hook:
corpus cases may assert the diagnostic exists for `import pydantic_core`-class
targets; exact diagnostic text is owned by the compat contract, not this doc.)

## 4.4 The main module

**S-04-17 (MUST)** The entry script executes as a module named `__main__`:
`__name__ == '__main__'` (observed), it occupies cache key `'__main__'`, and
`import __main__` from anywhere rebinds that same object (observed:
`__main__.__name__` → `__main__`; `__main__ is sys.modules['__main__']` → `True`).
`__package__` is `None` (S-04-03). This is what makes the
`if __name__ == "__main__":` idiom compile-time-visible and runtime-correct.

**S-04-18 (MUST)** Code in the main module runs at program start, after the
runtime banner/builtins setup and with exit-code plumbing of spec 06; uncaught
exceptions propagate to the default handler (traceback format, spec 02) with
frames named `File "<script path>", line N` — the script path form is the same
`__file__` value of S-04-04.

## 4.5 The import cache: identity and single execution

**S-04-19 (MUST)** Import results are memoized in a module cache
(sys.modules-equivalent, keyed by full dotted name). A second import of the same
name returns the **same object** — no second load, no re-execution (observed:
`import counter as c1; import counter as c2; c1.n = 5; print(c1 is c2, c2.n)` →
`True 5`; `sys.modules['counter'] is counter` → `True`; module body printed
`counter module ran` exactly once for two imports).

**S-04-20 (MUST)** Module-level code runs exactly once per process, at first
import. Mutation through one binding is visible through every other binding
because they are the same object (c1/c2 case above). Circular imports MUST NOT
deadlock: the partially-initialized module object is placed in the cache BEFORE
its body executes, so a cycle sees whatever bindings exist at that moment
(observed with `cycle_a`/`cycle_b` where `from_b` is read before assignment:
`to_a = getattr(cycle_a, 'from_b', 'partial')` → `partial`).

## 4.6 From-import copies values, not live bindings

**S-04-21 (MUST)** `from m import x` looks `x` up in `m`'s namespace AT IMPORT
TIME and binds that VALUE (object reference) in the importing namespace. It is
NOT an alias to `m.x`: later rebinding of `m.x` inside `m` (or elsewhere) does not
change the importer's `x`. Pin (observed, module `mutate_a` with `x = 1` and
`bump()` doing `global x; x += 1`):

```python
from mutate_a import x
import mutate_a
mutate_a.bump()
print(x, mutate_a.x)     # -> 1 2   (x is stale; mutate_a.x is live)
```

The same value-copy applies through helper modules: `side.py` containing
`from counter import n` keeps seeing `n == 1` even after the main script sets
`counter.n = 100` (observed: `side.show()` → `1`, `side.n` → `1`). Attribute
style (`m.x` read each time) is the live-binding path. v1 conformance MUST
reproduce the stale outputs above byte-exactly.

**S-04-22 (MUST)** Mutable values are shared by reference, not deep-copied —
`from m import lst` then `lst.append(...)` mutates `m.lst` too (identity is
preserved by S-04-21's value copy; see spec 01 for object identity semantics).
Rebinding is what diverges.

---

## Corpus anchors

Snippets verified on CPython 3.14.7 (`/opt/homebrew/bin/python3.14`). Each anchor
is a script (or script+module pair as stated); expected output is byte-exact
stdout/stderr plus exit code. Paths are shown as CPython printed them for
`/tmp/spec_verify` (macOS realpath `/private/...`); corpus authors MUST anchor on
`sys.path`-relative forms where absolute paths appear, per S-04-04's realpath
note. Case headers cite clause ids.

A1 (S-04-03, S-04-17) — script `m1.py`: `print(__name__, repr(__package__))`
→ `__main__ None`, rc 0.

A2 (S-04-04) — `python3.14 -c "print(__file__)"`
→ stderr `NameError: name '__file__' is not defined. Did you mean: '__name__'?`
(with traceback), rc 1; same script run as a file prints its absolute path, rc 0.

A3 (S-04-06) — with package tree `pkg/a/deep.py` (`val = 42`, prints
`pkg.a init` / `deep ran`): `import pkg.a.deep; print(sorted(k for k in dir() if
k in ('pkg','a','deep'))); print(pkg.a.deep.val)`
→ `pkg.a init\ndeep ran\n['pkg']\n42`, rc 0.

A4 (S-04-07) — `from pkg.a.deep import val; print(val)` → `pkg.a init\ndeep
ran\n42`, rc 0.

A5 (S-04-08) — `import pkg.a.deep as d; print(d.val)` → `pkg.a init\ndeep
ran\n42`, rc 0; adding `print(pkg)` in a follow-on line →
`NameError: name 'pkg' is not defined`, rc 1.

A6 (S-04-19) — `counter.py` = `print('counter module ran')\nn = 1`; script:
`import counter as c1\nimport counter as c2\nc1.n = 5\nprint(c1 is c2, c2.n)`
→ `counter module ran\nTrue 5`, rc 0.

A7 (S-04-21) — `mutate_a.py` = `x = 1\ndef bump():\n    global x\n    x += 1`;
script: `from mutate_a import x\nimport mutate_a\nmutate_a.bump()\nprint(x,
mutate_a.x)` → `1 2`, rc 0.

A8 (S-04-11) — `import counter\ndel counter\ntry: counter.n\nexcept NameError as
e: print('NameError:', e)\nimport counter\nprint(counter.n)`
→ `counter module ran\nNameError: name 'counter' is not defined\n1`, rc 0 (body
ran once only).

A9 (S-04-10) — `def f():\n    from counter import *\n` (whole program)
→ rc 1, stderr ends `SyntaxError: import * only allowed at module level`
with the source line and caret rendered per spec 02 traceback format.

A10 (S-04-10) — `noallmod.py` = `alpha = 1\nbeta = 2\n_og = 3`; script:
`from noallmod import *\nprint(sorted(k for k in dir() if not
k.startswith('__')))` → `['alpha', 'beta']`, rc 0.

A11 (S-04-14) — `try: import nosuchmodule\nexcept ModuleNotFoundError as e:
print(type(e).__name__ + ':', e, '| name attr:', repr(e.name))`
→ `ModuleNotFoundError: No module named 'nosuchmodule' | name attr:
'nosuchmodule'`, rc 0.

A12 (S-04-15) — `try: from counter import nosuch\nexcept ImportError as e:
print(type(e).__name__ + ':', e)`
→ `counter module ran\nImportError: cannot import name 'nosuch' from 'counter'
(/private/tmp/spec_verify/counter.py)`, rc 0.

A13 (S-04-05) — `import counter\nprint(repr(counter))`
→ `counter module ran\n<module 'counter' from
'/private/tmp/spec_verify/counter.py'>`, rc 0.

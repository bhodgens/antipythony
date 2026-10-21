# T1 attempt: scrapy under antipythany (2026-10-20)

Status: **NOT GATED.** `import typing` now succeeds; `import scrapy` still
fails in `scrapy/__init__.py` at `importlib.resources.files()`. ~20
interpreter/runtime fixes landed this attempt (all uncommitted, per leaf
instructions); zero regressions (rust lib tests green, pytest 1912 passed /
0 failed / 629 xfailed / 83 xpassed).

## Packaging layer

- `antipythany add scrapy` correctly fails loud (C-11): uv's transitive
  closure includes C wheels (`_brotli`, `lxml`, `cryptography`, `cffi`,
  `zope.interface` accel `.so`). By design — no change to the refusal.
- NEW: `antipythany add --no-deps "<req> <req> ..."` installs exactly the
  named requirements (uv `--no-deps`) with the purity filter still applied.
  The full pure-Python scrapy closure was staged pin-by-pin this way:
  scrapy 2.19.0, twisted 26.4.0, incremental 24.11.0, constantly 23.10.4,
  Automat 24.8.1, attrs 25.4.0, typing-extensions 4.16.0, queuelib 1.10.0,
  w3lib 2.5.0, parsel 1.12.1, protego 0.7.0, itemloaders 1.5.0,
  itemadapter 0.13.1, jmespath 1.1.0, cssselect 1.3.0, packaging 26.3,
  service-identity 26.1.0, pyasn1 0.6.1, pyasn1-modules 0.4.2, pycparser
  3.11, hyperlink 21.0.0, idna 3.20, setuptools 84.2.0.
- zope.interface 8.7 has NO pure wheel (accel `.so` always shipped);
  zope.interface **8.4** (older) works pure-Python after stripping the
  single `_zope_interface_coptimizations*.so` from the staged wheel —
  merged manually into cache site-packages. Candidate for a
  "strip-accelerators" packaging feature.
- lxml NOT needed for a file:// spider that parses via `response.text` +
  regex (parsel imports lxml lazily per-selector-use, not at import).

## Interpreter/runtime fixes landed (this attempt)

1. `functools.lru_cache(N)` / `lru_cache(maxsize=N)` (non-callable first
   arg) treated as single-stage and wrapped the INT. Now: callable arg =
   single-stage wrapper; otherwise two-stage deco. (vendored
   re/__init__.py `@lru_cache(_MAXCACHE)` was broken at import.)
2. Module-level decorated defs silently DROPPED decorators in BOTH
   `exec_module` and the importer (`module_stmts` + `load_into`). Now
   deferred to source position as `Stmt::LocalDef`. (typing/re/attrs all
   use module-level decorated defs.)
3. Module-level defs had no defining-frame registration, so default-arg
   expressions referencing module globals (`def f(v=_sentinel)`, vendored
   traceback.py) evaluated against the MAIN script globals -> NameError.
   Fixed in importer `module_stmts`/`load_into` (defining_frames +
   closure_slot).
4. `__traceback__` was a single stub frame with no f_code/chain. Now a
   real tb chain from `interp.tb_frames` with f_code (co_filename/
   co_name/co_qualname/co_firstlineno/co_positions), f_lineno, f_globals,
   f_locals, tb_next chain — shared `classes::traceback_value` used by
   both exception attr paths. `traceback.print_exc()` now formats.
5. `co_positions()` virtual for synthetic code objects
   (`<code>.co_positions` marker in `builtins::call_builtin`).
6. `sys.exception()` added (v0: returns None, matching the exc_info stub).
7. `__qualname__` on Type values (traceback reads exc_class.__qualname__).
8. Real tb chain unlocked traceback formatting; the vendored traceback
   StackSummary(list) then needed:
   - list-method surface on list-derived UserInstances (append/extend/...
     bind to a lazily-created `__ap_list__` backing list),
   - `len()` and iteration over that backing list (user_len + user_iter),
   - `__qualname__`/`__module__` on UserClasses.
9. `_typing.TypeVar(...)` etc. marker calls: the _typing stub markers now
   CALL to opaque placeholder objects carrying `__name__` (was a bogus
   NameError with the marker's dotted name).
10. `collections.abc` was an empty native module. Now execs the vendored
    `_collections_abc.py` into a `collections.abc` module (Hashable/
    Iterable/... all resolve).
11. Native `importlib` now exposes `__path__` (vendored importlib dir) and
    dotted names try the file finder BEFORE the native-registry fallback,
    so `import importlib.resources` resolves the vendored subpackage.
12. importlib/resources + zipfile vendored from CPython 3.14.7 (PSF, same
    licensing as the rest of vendored_stdlib).
13. `annotationlib.get_annotations(..., format=...)`: vendored copy predates
    the 3.14 `format=` kwarg typing uses; accepted + ignored.
14. Zero-arg super() inside `__setattr__` overrides: attribute-set protocol
    now stages the method super context (typing
    _BaseGenericAlias.__setattr__).
15. Virtual `object.__setattr__` (super().__setattr__ -> instance-dict
    write) and virtual `object.__init_subclass__` / `type.__init__` in the
    super scan (both object and metaclass-MRO branches).
16. `__init_subclass__` now runs on EVERY subclass creation (was: only when
    the class statement carried kwargs), with the classmethod super
    context staged.
17. User metaclass `__init__(cls, name, bases, dict, **kwds)` invoked at
    class creation (was never called; typing's _ProtocolMeta.__init__
    sets cls.__protocol_attrs__ there).
18. Class-attribute writes on UserClasses land in the class dict
    (`cls.__protocol_attrs__ = ...` inside __init_subclass__/metaclass
    __init__), with class_ref used when type_ref is absent.
19. Regular attr reads on UserClasses scan the MRO dicts; Type-kind attr
    reads fall back to parked PyObject attrs.
20. PEP 560 `__class_getitem__`: `Generic[T]`-style class subscription
    returns an opaque `typing._GenericAlias` placeholder (origin+params
    parked); `class Foo(Generic[T])` resolves the alias to its origin as
    the effective base; subscripting a BuiltinFn marker does the same.
21. `ABCMeta.register` accessed on marker fns -> inert callable.

## Verified working now

- `import typing`, `import twisted` (top-level), `import annotationlib`,
  `import zipfile`, `import collections.abc` (full ABC surface),
  `import importlib.resources` (module loads; see gap 1),
  `traceback.print_exc()` on a plain exception.

## Precise remaining blockers (distance to T1)

1. **`importlib.resources.files` binds None** (BLOCKS scrapy/__init__).
   `import importlib.resources` completes but the module namespace is
   partial; `files` resolves to None and scrapy's `__version__` line
   (`files("scrapy").joinpath("VERSION")`) dies with
   `TypeError: 'NoneType' object is not callable`. The resources package
   exec appears to abort partway under the dotted-native-parent import
   path (its own deps: io, os, contextlib, zipfile import fine
   individually). Needs: correct submodule exec under a native parent
   package + a working `files()` (or a v0 shim returning a traversable
   over the site-packages dir).
2. **Chained-traceback formatting** (`During handling of ...`): printing
   the SECOND exception in a chain still hits
   `AttributeError: 'object' object has no attribute 'filename'` inside
   StackSummary.format_frame_summary — some frame_summary receiver lacks
   its instance attrs (slotted-instance attr path under the metaclass /
   chained-tb route). Plain single tracebacks format correctly.
3. **twisted reactor**: `import twisted` top-level works, but the reactor
   subpackage import dies at `exec(code, globals, locals)` —
   `exec with explicit globals/locals unsupported (v0)`. Twisted's
   `twisted/python/modules.py`-style dynamic exec needs the two-arg exec
   form. Must be fixed before any spider crawl (CrawlerProcess pulls the
   reactor).
4. **None-callable tail in typing/attrs paths**: at least one module-level
   statement in the scrapy import chain still calls a None global
   (`_lazy_annotationlib`-adjacent; located inside
   `_get_protocol_attrs`/`__init_subclass__` flow). After 1-3 land, re-run
   `import scrapy` and chase the remaining one-at-a-time.
5. **abc.register semantics**: currently inert (returns None). Real ABC
   registration is needed if scrapy/attrs rely on isinstance checks
   against registered classes (likely at crawl time, not import time).

## Byte-compare status

NOT REACHED. The CPython 3.14.4 reference environment is staged at
/tmp/t1/cpy (uv venv, scrapy 2.19.0 + full dep tree) and the reference
run produces a correct items.jsonl:

```
{"id": 1, "title": "ALPHA WIDGET", "url": "file:///tmp/t1/page.html"}
{"id": 2, "title": "BETA GADGET", "url": "file:///tmp/t1/page.html"}
{"id": 3, "title": "GAMMA GIZMO", "url": "file:///tmp/t1/page.html"}
{"id": 0, "title": "SUMMARY", "url": "file:///tmp/t1/page.html", "count": 3}
```

Spider: /tmp/t1/spider.py (file:// start URL, regex parse — no lxml —
T1Pipeline uppercases titles, FEEDS jsonlines, TWISTED_REACTOR None).
Once blockers 1-3 clear, the gate is: `antipythany run spider.py` in
/tmp/t1/t1proj, `cmp items.jsonl /tmp/t1/ref/items.jsonl`.

## Repro commands

```
cd /tmp/t1/t1proj
antipythany run --no-cache probe37.py   # import typing -> OK
antipythany run --no-cache probe51.py   # scrapy/twisted import depth
antipythany run --no-cache spider.py    # current blocker: files() None
```

(All changes uncommitted. The `--no-deps` add form, the 20 interpreter
fixes, and the vendored annotationlib edit + new vendored
importlib/resources + zipfile packages are the full diff.)

## Session 2 progress (2026-10-20, continued)

Fixes landed (uncommitted, same leaf):

21. **collections.abc native-parent sys.modules gap**: `import
    collections.abc` built the native `collections` module but never
    inserted it into sys.modules — a later bare `import collections`
    re-ran the vendored collections.py exec and died. Fixed in importer
    (collections.abc arm seeds sys.modules['collections']; native-parent
    arm inserts + links every walked prefix).
22. **functools.singledispatch**: real registry object now (was a no-op
    pass-through). `register(cls)` returns a working decorator; dispatch
    routes to exact-type impls (importlib.resources._common needs it).
23. **2/3-arg exec**: builtin exec now runs the statements over the GIVEN
    globals dict via new `interp::exec_stmts_in_ns`; defs register into
    the live FnRegistry (extend_current) so nested defs resolve. Twisted's
    `_FuncBuilder.add_fns_to_class` (exec with f_globals/f_locals) works.
24. **dataclasses**: class-body AnnAssign now populates `__annotations__`
    in the class ns (field discovery was empty before). object.__setattr__
    virtual (used by frozen init) now writes the instance dict instead of
    no-op.
25. **PEP 562 module __getattr__**: module attr reads fall back to the
    ns's `__getattr__` (typing.ForwardRef lazy hook).
26. **typing_extensions**: typing.Union isinstance (fn-marker) answers
    False instead of TypeError; _warnings `_acquire_lock/_release_lock/
    _warnings_context/_filters_mutated_lock_held/filters` stubs added
    (warnings.py's from-import was failing → `_wm=None` blowup later);
    functools singledispatch (see 23); property.setter on the property
    TYPE (`property.setter(self, fn)` in twisted's _DeprecatedProperty);
    property .setter/.deleter/.getter on plain Function receivers.
27. **Explicit metaclass __new__ call**: `abc.ABCMeta.__new__(mcls, ...)`
    stages the super context (Name receivers only — no double-eval).
28. **annotationlib.ForwardRef.__slots__** surface added (typing.py
    `_FORWARD_REF_HAS_CLASS` reads it).

## Remaining blockers (verified this session)

- **Chained traceback formatting**: printing a traceback whose walk
  invokes `t.tb_next`/FrameSummary paths still dies with
  `super(): no arguments` inside the tb machinery (tb objects' attr reads
  route into method-context machinery). NOT fixed.
- **abc.register**: still inert — NOT fixed (scrapy/zope likely need it
  at crawl time).
- **`import scrapy`**: gets further (twisted + typing_extensions now
  import), but `import scrapy` itself dies in its dep chain:
  automat._typed → zope.interface fails with
  `_InterfaceMetaClass.__new__() missing 1 required positional argument:
  'bases'` (zope metaclass __new__ called through the class-call path
  loses args). NOT fixed.
- **T1 gate**: NOT reached — `import scrapy` still fails.

## Gate status this session

- rust: cargo test --workspace → 362 passed / 0 failed / 1 ignored.
- pytest: 1912 passed / 0 failed / 6 skipped / 630 xfailed / 82 xpassed.
- corpus families: t11_stdlib 284p/2s/77xf/65xp; t05_values 192p/1xf/6xp;
  fstrings 107p; t07_gc 113p/20xf; t10_threading 97p/4xf/7xp. All held.
- cargo fmt --all --check clean.

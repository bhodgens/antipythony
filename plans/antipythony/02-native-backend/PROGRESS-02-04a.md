
## 02.04a — file-based import machinery (IMPLEMENTED, NOT COMMITTED)

### What landed
- `crates/ap-run/src/importer.rs` (new): sys.modules cache -> native registry ->
  file finder over sys.path; module execution in fresh namespaces with
  __name__/__file__/__package__; sys.modules insert BEFORE body (circular
  partial visibility); packages get __path__; relative imports per CPython;
  namespace packages; parent.submodule attribute linking; CPython-exact
  ModuleNotFoundError/ImportError texts; ModuleNotFoundError added to the
  exception tree (ImportError subclass).
- `interp.rs` diff regions (small, as required):
  1. `use crate::{...}` — added `importer`.
  2. FnRegistry: added `by_path_lookup`, `extend_current` (+ private
     `merge_scopes_static`), `entries` made pub(crate); `build` takes
     module_globals.
  3. `run_module` entry: importer::reset_import_state + sys.path bootstrap
     ([script_dir] + vendored_stdlib + ANTIPTYHONY_PATH).
  4. Stmt::Import arm: rewritten to resolution order (sys.modules ->
     native -> file finder), top-package binding preserved.
  5. Stmt::ImportFrom arm: relative (level>0) via resolve_relative_base;
     star-imports implemented (public names of module ns); native path
     keeps byte-exact legacy texts; file path adds "(file)" CPython shape.
  6. attr_get Function arm: __doc__/__module__/__qualname__/__code__ stub/
     __closure__ stub reads + o.attrs passthrough.
  7. attr_get Type arm: method-surface markers, __dict__ facade, __mro__
     singleton, function __code__/__globals__ stubs.
  8. eval_binop: PEP 604 Type|Type stub.
  9. iterable_of: metaclass __iter__ for classes.
  10. unpack_into: generic-iterable fallback.
- `frame.rs`: FnEntry gained `globals` (defining module's namespace) —
  functions from imported modules resolve THEIR globals; fn_entry reads the
  live FnRegistry (imported defs live there).
- `classes.rs`: metaclass __new__ dispatch (user meta __new__ called with
  meta/name/bases/ns; super().__new__ lands in builtin type.__new__);
  metaclass-level plain methods bind the class (ABCMeta.register);
  object virtuals at class level (__init__/__new__/__str__/...);
  exception __traceback__ stub; builtin-type base synthesis (enum.py
  IntEnum(int, ReprEnum)); decorator-builtin base synthesis
  (abstractclassmethod(classmethod)).
- `builtins.rs`: globals()/locals()/reversed()/bytearray (Bytes payload,
  documented); Ellipsis/NotImplemented builtins; sys.implementation,
  sys._getframe/_getframemodulename stubs; range() bigint clamp (divergence);
  zip() empty-arg exhausted-iterator fix (gen_id 0 collision);
  type.__new__ builtin (meta __new__ plumbing); _weakref.ref token.
- `methods.rs`: PEP 585 Type-subscript GenericAlias stub; set.copy().
- `exc_hier.rs` + EXC_TREE: ModuleNotFoundError.
- `vendored_stdlib/` (repo root, NOT in crates/): 34 top-level targets /
  58 .py files verbatim from CPython 3.14.7 Lib/ (PSF-2.0; NOTICE.md).
  Dropped: encodings/ (C codecs), importlib/metadata+resources, bootstrap
  files. Added beyond the brief's list: ntpath.py, _collections_abc.py,
  copyreg.py, __future__.py, _py_abc.py, _weakrefset.py (transitive deps
  of pathlib/abc). NOT vendored (native shadows by design): collections, io,
  operator, functools, string, textwrap, json, os, atexit, sysconfig,
  math/random/hashlib/base64/binascii/tempfile + the brief's native set.
- `crates/ap-run/tests/import_differential.rs` (new): 6 differential tests
  vs python3.14 — package+relative, circular partial visibility, byte-exact
  ImportError texts, sys.modules identity, vendored resolution depth,
  script_dir bootstrap. All green.

### Interp bugs found & fixed (with repro class)
1. Function attribute reads (__doc__) — posixpath.py `splitext.__doc__ = ...
   __doc__`; 1b: attribute WRITES on functions (function __dict__).
2. Decorator builtins as base classes (abc.py abstractclassmethod).
3. async def call returned None (types.py `_c().close()`); stub with
   no-op close/send/throw.
4. object.__init__/__new__/__str__ unreadable at class/instance level
   (types.py WrapperDescriptor/MethodWrapper probes).
5. Builtin-type method reads (type(str.join)), __dict__ facade
   (dict.__dict__['fromkeys']), function __code__/__globals__.
6. Exception __traceback__ missing.
7. `list[int]` subscript + `int | str` union stubs (types.py GenericAlias).
8. Ellipsis/NotImplemented builtins missing.
9. globals()/locals() builtins missing.
10. Module namespace snapshot: imported module attrs invisible after body
    exec (module_ns refreshed post-body; live-dict semantics is a
    documented follow-up — circular imports observe the pre-body view).
11. Builtin-type bases in class defs (IntEnum(int, ...)).
12. bytearray()/reversed() builtins missing.
13. Class iteration via metaclass __iter__ (FlagBoundary unpack) +
    generic-iterable unpack fallback.
14. range(1<<1000) TypeError (clamped — documented divergence).
15. zip() gen_id 0 collision ("iter() re-entry").
16. Metaclass plain methods unbound when read on the class (register).
17. Metaclass __new__ never dispatched (ABCMeta._abc_cache missing) —
    now dispatched with type.__new__ builtin.
18. __mro__ on non-class 'type' values (ABC probes).
19. set.copy() missing.

### Gate status (honest)
- (a) pathlib: import machinery works; pathlib loads deeply but BLOCKED at
  `_collections_abc` -> `Coroutine.register(coroutine)` interplay (ABC
  virtual-registration against the v0 coroutine stub reports a false
  "inheritance cycle"). Not green.
- (b) dataclasses: import machinery works; blocked transitively —
  dataclasses imports enum (same ABC/enum-machinery chain). Not green.
- (c) enum: module imports up to FlagBoundary; BLOCKED at member creation —
  EnumType.__new__ member machinery (`_member_names` on EnumDict instances)
  is Plan-02-scale (dict-derived class instances + full meta __new__
  semantics). Not green.
- (d) unittest: imports blocked on _py_warnings (warnings.py chain) —
  not reached. Not green (best-effort honestly reported).
- (e) circular a<->b partial visibility: GREEN (differential).
- (f) relative imports in a package (from . / from .mod, submodules,
  pkg.mod attr linking): GREEN (differential).
- (g) ImportError texts byte-exact: GREEN (differential, 5 texts).

### Verification
- cargo test --workspace: 200 passed, 0 failed.
- pytest (homebrew python3.14): 1271 passed, 5 skipped, 0 failed
  (baseline 1270).
- cargo fmt: clean.
- ap-conform stdlib slice: 8/48 (baseline 8/48 — the corpus cases that
  would flip need pathlib/dataclasses/enum, blocked as above).
- Environment note: the 3 self-comparison harness failures (t09/t10/t11)
  under `uv run pytest` are PRE-EXISTING — a venv python3.14 (3.14.4)
  shadows the pinned 3.14.7 reference via PATH; verified by stash+rerun on
  baseline. Run pytest with /opt/homebrew/bin/python3.14 to get green.

### Follow-ups for orchestrator triage
- Metaclass __new__ semantics (EnumType member conversion, dict-derived
  instances) — unblocks pathlib/dataclasses/enum corpus slice (est. 38
  cases) and the path-to-targets chain.
- Module ns live-dict identity (module_ns by-value clone today).
- warnings.py chain (_py_warnings) + annotationlib for unittest/inspect.

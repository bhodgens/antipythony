# NEXT-STRUCT1 — Plan 05 leaf: F-STRUCT-1 re-entrant lazy import of a
# partially-initialized package (from-import must consult module __getattr__
# before re-entering the importer)

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-struct1` (branch
`tier1-struct1`, off main `1722168`). Target dir: `target-struct1`
(CARGO_BUILD_JOBS=4; one heavy cargo lane; logs under the worktree).

## VERDICT: CORE FIX GREEN — the F-STRUCT-1 mechanism is fixed and gated;
the full `from pydantic import BaseModel; class M(BaseModel): x: int;
M(x=1)` chain ADVANCED past four walls but a NEW downstream wall remains
(dataclass-generated __repr__ 'tuple' not subscriptable during the
ModelMetaclass __signature__ walk) — recorded, not the leaf brief.

## The fix (commit 3eae982)

1. **import_module seam order** (builtins_modules.rs call_importlib):
   the 2-arg/relative resolution now runs BEFORE import_module_inner's
   dotted-name validation. Pre-fix, `import_module('.sub', package='P')`
   fed the raw dotted name into the leading-dot rejection →
   `ModuleNotFoundError: No module named '.sub'`. PEP-562 `__getattr__`
   lazy-imports of a partially-initialized package now work; from-imports
   consult the sys.modules module dict + `__getattr__` WITHOUT re-entering
   the importer (CPython order).
2. **gate-1a** (classes.rs instantiate): dict-subclass ctor with
   kwargs-only shape (`D(a=1)`) — payload gap → "takes no arguments".
3. **gate-1b** (interp.rs): object-virtual `__init__` marker +
   `__ap_base_dict__init__` marker MERGE ctor args/kwargs into the
   receiver's dict backing (CPython `dict.__init__(d, *args, **kw)`).
   TypedDict/ConfigDict route through these via `super().__init__`.
4. **gate-1c** (classes.rs): metaclass `__call__` that is a plain
   Type/Function (typing_extensions `_TypedDictMeta` sets
   `__call__ = dict`) routes class-call args to the builtin. Scoped AWAY
   from BuiltinFn markers (type.__call__ etc) after a regression check —
   see "regression A/B" below.
5. **gate-2** (builtins.rs + vendored inspect.py): native builtins seeded
   `__module__='builtins'`; isbuiltin/_signature_is_builtin/
   _signature_get_user_defined_method gained the `__module__=='builtins'`
   discriminator (runtime interns ONE 'function' Type for user fns +
   native builtins → `inspect.signature(Cls)` raised "no signature found
   for builtin type" on every user class, killing @dataclass).
   `__partialmethod__` read gained the wave-80-style
   BaseException→AttributeError guard. DEVIATIONS.toml inspect.py row
   re-declared (new deviated_sha256 + note).
6. **gate-2b** (interp.rs): NAMESPACE `_GenericAlias` call arm
   (Type.__getitem__ alias placeholder): `WeakValueDictionary[K,V]()`
   constructs the origin class (previously "'object' object is not
   callable"); weak-container origins route through gcsupport.
7. **staged deps**: fractions.py + zoneinfo/ vendored (verbatim CPython
   3.14.7 backfills, zoneinfo/_tzpath.py is a shim — all declared in
   DEVIATIONS.toml); pydantic_core probe extension gained
   `PydanticSerializationUnexpectedValue`; tests/struct1/
   core_schema_shim.py upgrades the ce core_schema str marker to a module
   ns answering the pydantic 2.14 import surface (Protocol-shaped TYPE
   names, class-shaped core names).

## Gates (executed, not claimed)

- Minimal repro (tests/struct1/repro_min.py + P/): ours RC=0, stdout
  BYTE-IDENTICAL vs CPython 3.14.7 (`P-ATTR: SUB-OK / DONE`).
- `cargo test -p ap-run --release`: **208 passed / 0 failed / 1 ignored**
  (build the debug ap-run-eval + ap-run-probe bins first — the
  fix_pass_differential suite spawns `$CARGO_TARGET_DIR/debug/*`).
- `cargo test -p ap-cli --release --test stdlib_modules_differential`:
  **48 passed / 0 failed**.
- t04_fastapi_reach: **7 passed** (needs `target-t4/debug/antipythony` —
  symlink target-t4 → target-struct1 before running, then remove).
- notices --check: OK (346 deps).
- stdlib-provenance: inspect.py re-declared (net stale count 8→7 vs main;
  the gate is ALREADY RED on main for unrelated sibling files — _ast,
  annotationlib, collections, enum, functools, types, typing stale +
  tokenize.py drift — recorded, not this leaf).

## Chain status (honest)

`from pydantic import BaseModel` via the staged wheel
($ANTIPTYHONY_CACHE/site-packages/pydantic 2.14.1, APX_EXT_DIR
libpydantic_core.so + core_schema_shim) now clears: pydantic/__init__ S1,
_internal._config (TypedDict/ConfigDict), _decorators, _fields,
_forward_ref, _generics, _mock_val_ser, _typing_extra, _known_annotated_metadata,
aliases, plugin._schema_validator, pydantic.aliases, warnings, errors,
config, version, typing_inspection, annotated_types.

NEW DOWNSTREAM WALL (recorded for the next leaf): importing
pydantic._internal._generate_schema → ... → pydantic.main dies with
`TypeError: 'tuple' object is not subscriptable` raised inside the
inspect `_signature_from_callable` walk (`sig = obj.__signature__` /
`obj.__partialmethod__` frames, obj = "unprintable function/type" whose
`__repr__` itself raises the same TypeError). Suspected: a
pydantic-internal object (dataclass-generated, slots=True) whose
`__repr__`/descriptor `__get__` subscripts a tuple — the failing frame
text maps to dataclasses._FuncBuilder.add_fns_to_class line 532
(`annotation_fields, return_type = self.method_annotations[name]`).
Debug aids shipped: APX_SFCDBG already names the obj (its repr crashes —
use the __module__ line); probe drivers in /tmp/probe_main{2,4,5,6}.py
shape. NOT the F-STRUCT-1 brief (the mechanism fix is green).

## Regression A/B (executed)

gate-1c originally also routed BuiltinFn-marker metaclass __call__
(type.__call__ etc) — stash-rebuild-run A/B showed the downstream
'tuple' wall APPEARS only with the broad form. Scoped to Type/Function
kinds; TD(kwargs) still green, minimal repro still green.

## Machine policy

One cargo lane; CARGO_BUILD_JOBS=4; explicit pathspec commits; no
`git add -A`. GOTCHA: `cargo test -p ap-run` needs debug ap-run-eval +
ap-run-probe bins in $CARGO_TARGET_DIR/debug or fix_pass_differential
fails with "spawn fenced cmd: No such file or directory".

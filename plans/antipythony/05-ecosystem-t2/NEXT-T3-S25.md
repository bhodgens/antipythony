# NEXT-T3-S2.5 — Plan 05 leaf: type.__instancecheck__ wall (issue #20)

Date: 2026-11-26. Worktree: `~/.git/worktrees/leaf-t3chk` (branch
`tier1-t3chk`, off main `74febf1`). Target dir: `target-t3chk`. Logs +
probes under `logs-t3s2/` and `t3chat/` (probe-only, uncommitted by
convention except where pathspec'd).

## VERDICT: DONE — the wall is FIXED and the pydantic chain goes THROUGH
stmt 33 to a WORKING `from pydantic import BaseModel` + model
instantiation/field round-trip (D stage). Commit chain ff138ad → 37495a5
→ 4a270b5.

## Root cause (wall as recorded in NEXT-T3-S2.md item d)

The failure was NOT inside the @dataclass_transform application itself.
The chain reached typing_extensions' `_ProtocolMeta.__instancecheck__`
(typing_extensions.py __instancecheck__, the `cls is Protocol` arm) which
calls `type.__instancecheck__(cls, instance)` EXPLICITLY — CPython exposes
`__instancecheck__`/`__subclasscheck__` as slot wrappers ON `type` (in
type.__dict__); our interned `type` type-class dict lacked both names ->
AttributeError "'type' object has no attribute '__instancecheck__'".
Minimal repro: logs-t3s2/probe/chk01_type_attr.py (fails on trunk base,
green after).

## Fixes (general type-protocol mechanisms, no pydantic special-casing)

1. **type slot wrappers seeded** (classes.rs type_class): the `type` dict
   carries `type.__instancecheck__`/`type.__subclasscheck__` BuiltinFn
   markers (mirrors the existing type.__new__ entry); interp.rs
   call_value_inner_impl dispatches them to the native structural answer
   (value_isinstance/value_issubclass) — CPython slot semantics: the BASE
   answer user hooks delegate INTO, no metaclass-hook re-entry (would
   recurse).
2. **W112 consult-loop guard** (classes.rs value_isinstance +
   builtins.rs check_isinstance): the hook-resolution sites now skip the
   base slot markers (they ARE the structural fallback those arms
   implement); calling them loops (marker -> value_isinstance -> same arm).
3. **slots-vs-class-var ordering** (classes.rs make_class step 5 +
   builtins.rs builtin_type_new): CPython validates __slots__ conflicts in
   type.__new__ — AFTER the metaclass __new__ (ModelMetaclass pops the
   instance-slot entries in its no-bases branch). Pre-metaclass checking
   rejected BaseModel's body. Deferred to builtin_type_new when a user
   metaclass __new__ drives creation (conflict STILL raised when the meta
   __new__ does not pop — probe_slots3.py).
4. **dict_keys views are sized set-likes** (interp.rs): set &/|/- over
   dict_keys views both directions + view&view (pydantic _config
   `V2_REMOVED_KEYS & config_dict.keys()` /
   `V2_RENAMED_KEYS.keys() & ...`); dict-view truthiness sizes from the
   live payload (`if serializers:` over an EMPTY values() view must be
   False — _apply_model_serializers gates on it; truthy-empty view took
   list(empty)[-1] -> IndexError).
5. **__isabstractmethod__ default on set/frozenset** (interp.rs, both
   dunder-surface arms): abc's abstract collection runs
   getattr(value, "__isabstractmethod__", False) over every model
   namespace entry; the truthy virtual marker marked plain sets (and the
   frozenset __abstractmethods__ frozenset itself) abstract ->
   "Can't instantiate abstract class ChatMessage without an
   implementation for abstract method '__class_vars__'".
6. **object.__getattribute__ does not re-enter __getattr__** (interp.rs
   getattribute marker call arm + classes.rs instance_attr step 4):
   BaseModel.__getattr__ probes object.__getattribute__(self, ...) inside
   __getattr__; the slot arm fell back to the full walk incl. __getattr__
   -> RecursionError. Thread-local reentrancy counter guards both
   __getattr__ steps while a getattribute slot call is in flight.
7. **vendored ForwardRef.__owner__** (vendored_stdlib/annotationlib.py,
   separate commit 4a270b5 — vendored-owned surface, flagged): v0
   ForwardRef.__init__ never assigned the __owner__ slot; typing.py
   evaluate_forward_ref/_make_forward_ref read it mid annotation
   evaluation (pydantic _typing_extra.get_model_type_hints).

## Probe staging (reproduction of the prior leaf's uncommitted deps_py)

The prior leaf's t3chat/deps_py/ pure-py pydantic_core shim was
uncommitted and its worktree is gone — rebuilt as `t3chat/deps/
pydantic_core.py` (real types: SchemaValidator/SchemaSerializer/
ValidationError/PydanticUndefined/__version__="2.50.1" passing the
version gate; core_schema members are real CLASSES because pydantic
builds runtime type-unions over them in _core_utils.py; Handler members
are typing.Protocol subclasses because pydantic subclasses them as
`class X(core_schema.Y, Protocol)` and typing._ProtocolMeta rejects
non-protocol bases). Staged into the cache site-packages as a PACKAGE
(pydantic_core/__init__.py + core_shim.py + core_schema.py +
_pydantic_core.py; the file-finder needs __init__.py for a package dir).
The native _pydantic_core surface pydantic touches (_schema_gather with
MissingDefinitionError attribute, gather_schemas_for_cleaning) is
shimmed honestly (empty collection — shim schemas carry no
definition-ref members).

Driver: t3chat/probe_ic.py — stages A (import pydantic_core), B (import
pydantic._internal._model_construction = the wall), C (from pydantic
import BaseModel), D (instantiate ChatMessage(role=.., content=..) and
read fields back). ALL FOUR GREEN: `D OK user hi`.

## Gates (executed)

- `cargo test -p ap-cli --test stdlib_modules_differential`: **48 passed /
  0 failed** (after all commits).
- `cargo test -p ap-run --test classes_differential`: **29 passed /
  0 failed**.
- `cargo test -p ap-run --test method_surface_differential`: **34 passed /
  0 failed**.
- `scripts/unsafe_report.sh --diff`: zero new unsafe (no changed .rs in
  the ratchet view; zero new unsafe blocks added by hand).
- stdlib-only ABCMeta/isinstance regression probe
  logs-t3s2/probe/chk02e.py: green (assert matrix in the earlier
  chk02 variant that failed pre-fix passes: isinstance through ABC
  hierarchy, slot calls, issubclass through ABCMeta, bool<int).

## Honest scope

- The D stage passes through the PURE-PY pydantic_core shim (validation
  is store-kwargs; to_python is vars() dump). This is the recorded S2.5
  exercise posture — the REAL pydantic_core extension surface is Plan
  10.04, unchanged.
- ModelMetaclass.__signature__ walk: the complete_model_class chain now
  proceeds through generate_pydantic_signature WITHOUT the
  ModelMetaclass __signature__ recursion the NEXT-STRUCT1 record filed
  (the object.__getattribute__ no-re-entry fix removed the loop the
  walk died in). The signature is generated; not byte-compared against
  CPython here.
- pydantic_full deeper surfaces (model_config deprecation walk
  ConfigWrapper effective_config, serialization round-trips) are
  exercised only as far as the D-stage probe reaches.
- vendored_stdlib edit (annotationlib) is outside my primary file
  ownership; committed separately with the exact consumer pinned. If a
  vendored-owned leaf reverts it, the pydantic chain regresses at
  _typing_extra.get_model_type_hints.

## Machine policy

One cargo lane; CARGO_TARGET_DIR=$PWD/target-t3chk; logs + probes under
the worktree. No push, no merge — branch tier1-t3chk head 4a270b5.

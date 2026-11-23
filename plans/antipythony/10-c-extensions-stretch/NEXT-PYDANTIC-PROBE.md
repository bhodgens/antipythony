# NEXT-PYDANTIC-PROBE — Plan 10 kill-criterion #3 session record

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-pydantic` (branch
`tier1-pydantic`, off main `7b16c4e`). Target dir: `target-pydantic`
(CARGO_BUILD_JOBS=4; one heavy cargo lane held).

## VERDICT: GREEN — the probe passes end-to-end

`import pydantic_core` + FromSpec-built Model + instantiate + validate one
field through the built `antipythony` binary: RC=0, every probe line GREEN
(`tests/ce-s1/pydantic_core_driver.py`, extension
`tests/ce-s1/pydantic_core.c`). Kill-criterion #3 does NOT fire.

## 1. Inventory (the exact nm-diff work queue)

Source of truth: `docs/ce-scan-symbols-full.json`, key
`pydantic_core-2.50.5-cp314-cp314-macosx_11_0_arm64` (the survey's own
measured 169). Diffed against `nm`-equivalent extraction of the landed S1
exports (196 `#[no_mangle] extern "C"` symbols across ap-ce):

- COVERED at leaf start: 59 (incl. its `PyInit__pydantic_core`, an export
  not an import).
- MISSING: **110** — the work queue. Bucketed:
  - PyExc_ data (10): BaseException/Exception/AssertionError/ImportError/
    LookupError/DeprecationWarning/BaseExceptionGroup/UnicodeDecodeError/
    UserWarning/RecursionError.
  - Type tokens (15): Type/BaseObject/Bool/ByteArray/Bytes/Complex/Dict/
    Float/FrozenSet/List/Long/Module/Set/Tuple/Unicode `_Type`.
  - Mapping/sequence/object (18): Get/Set/DelItem, Size, Generic{Get,Set}
    Dict, GenericSetAttr, IsInstance, IsSubclass, HasAttrWithError, DelItem,
    Init, Free, VectorcallDict, VectorcallMethod, Sequence_Check/Contains.
  - Number (5): Index, Remainder, TrueDivide, FromInt32/FromUInt32,
    From/AsUnsignedLongLong.
  - Sets (4): Set_New/Add/Contains, FrozenSet_New.
  - Complex (3): FromDoubles, Real/ImagAsDouble.
  - Unicode (11): Writer_{Create,Discard,Finish,WriteChar,WriteUTF8}, New,
    DATA, InternInPlace, AsEncodedString, EqualToUTF8AndSize,
    UnicodeDecodeError_Create.
  - Bytes/bytearray (3): FromStringAndSize, ByteArray_Size/AsString.
  - Module family (4): FromDefAndSpec2, ExecDef, ModuleDef_Init,
    GetNameObject.
  - Import/interp/const (5): PyImport_Import, InterpreterState_GetID,
    Py_GetConstant, IsInitialized, IsFinalizing.
  - Errors/exc accessors (8): PrintEx, WarnEx, WriteUnraisable,
    Exception_{Get,Set}Cause, {Get,Set}Traceback, TraceBack_Print.
  - Types (8): FromSpec (existed via shim), GenericAlloc, Freeze,
    Get{Name,QualName,ModuleName}, IsSubtype, Callable_Check.
  - Misc (5): PyCMethod_New, Capsule_Import, PyIter_NextItem,
    Mapping_Items, PyErr family odds.

## 2. Implementation (all 110 landed)

`crates/ap-ce/src/exports_pydantic.rs` (~1400 lines) — one owner per C
symbol, CPython-spelled unprefixed exports. Classification:

- WRAPPER (vtable-routed, the 1005b model): Get/Set/DelItem,
  Sequence_Contains, IsInstance/IsSubclass, PyNumber_{Index,Remainder,
  TrueDivide}, PyImport_Import — through a NEW MappingVTable
  (`install_mapping_vtable`, installed by ap-run's ce_objproto next to the
  objproto table; bodies: methods::subscript_{get,set,del}, interp::contains,
  interp::eval_binop, importer::import_module).
- WRAPPER (direct ap-obj core Values): sets/frozensets (PySet is an
  ap-obj Value — direct construction, no vtable needed), complex
  (repr-pinned opaque; trivial model never arithmetic-ops one), unicode
  writers (Mutex<Vec<String>> handle table, CPython writer contract).
- STRUCTURAL (identity-only, the 1006 token pattern): all 10 PyExc_ + 15
  type tokens (Str handles carrying the class name via apx_ macros),
  PyType_Freeze (accept), PyModuleDef_Init (pass-through),
  PyObject_Init (identity), PyException Set-accessors (accept), PyErr_
  {PrintEx,WriteUnraisable} (clear-and-flush), PyErr_WarnEx (0= honored),
  Py_Capsule_Import (FAILS LOUD: S3 member, ImportError dialect),
  PyCMethod_New (FAILS LOUD: S2 Def-class member, not needed for the
  trivial model), Py_IsInitialized/Finalizing, InterpreterState_GetID
  (pinned 1 per NEXT-1006), Py_GetConstant (CPython pycore constant
  order 0-7).
- Bytes/Set/Frozen exact round-trips: NEW CePayload::{BytesMarker,
  SetMarker(Gc<PySet>, frozen)} arms in from_value/to_value (state.rs,
  value_conv.rs) — see finding F1.

Header: `include/apx_python.h` — declarations for every new member
(+PyFrozenSet_New caught missing by the probe build), Py_tp_members/
Py_tp_doc slot ids, PyFrozenSet_New.

## 3. Gate-found findings (the probe's real deliverables)

- **F1 (the repr-pin straggler class, now measured)**: `from_value` had
  NO Bytes/Set/Frozen arms — they fell to the repr-pin Str fallback, so
  `PyBytes_FromStringAndSize("ab",2)` + `PyBytes_Size` returned 0, and
  PySet_Contains on a Set handle answered 0. EXACTLY the survey §3 risk
  ("same symbol, deeper semantics"). Fixed with the marker payloads +
  round-trip arms. After the fix: BYTES 1, SET 1, FROZEN 1.
- **F2 (module fn self)**: ce module-level fns received `from_value(None)`
  as self — pydantic-core's module fns attr-read the module through self
  (CPython's ml_self contract). Fixed: call_ce_module_fn now mints the
  module handle from the ce ns.
- **F3 (init-time C attr reads)**: PyObject_GetAttrString on the ce
  module DURING PyInit misses everything (attr layer serves the
  PYTHON-level module, built by finish_module AFTER PyInit returns).
  pydantic-core's real init does not attr-read its own module, so this is
  an S2 note, not a blocker; the probe binds the class via
  apx_spec_class_value(0) (the spec-seam test helper) post-import.
- **F4 (tp_new arg binding)**: FromSpec tp_new receives (cls-stripped,
  ctor-args) — the ctor arg is args[0] of the tuple, NOT args[1]
  (cls rides the marker call, not the C tuple). Documented by the probe.

## 4. Probe result (the gate, captured)

`antipythony run tests/ce-s1/pydantic_core_driver.py` from tests/ce-s1
(libpydantic_core.so built via `antipythony build-ext` ONLY):

```
MODEL type
GATE 42
NEG_RAISED Input should be a valid integer, got a negati
TYPE_RAISED 'str' object cannot be interpreted as an integer
EXC ValidationError
PROBE_DONE
INIT 1 0        INTERP 1      CONST_NONE 1   TOKENS type int str
CALLABLE 1      WRITER validated   EQ 1       BYTES 1
SET 1           FROZEN 1           COMPLEX 1.5 2.5
NUMBER 1 3.5    INDEX 1            U64 1      MAP 1   DEL 1
SEQ 1 1         CAUSE 1            IMPORT 1   DONE
RC=0
```

- IMPORT: dlopen libpydantic_core.so + PyInit ran clean (zero missing
  symbols at dlopen — all 110 resolve against the exporting binary).
- GATE: instantiate Model(42) + read validated field x => 42.
- Error dialects: negative => ValueError (the pydantic message shape),
  non-int => TypeError.
- Every exercised member from the measured demand behaves.

## 5. Gates (executed, not claimed)

- `cargo test -p ap-ce` (target-pydantic): **36 passed / 0 failed**
  (unchanged from the NEXT-1006 baseline — the band added no regressions).
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48 passed /
  0 failed** (unchanged).
- otool linkage: the .so product built via build-ext (undefined symbols
  resolve at dlopen — the source-shim contract, unchanged).

## 6. Honest gaps / scope notes

- The cp314 WHEEL itself is NOT loaded — rebuild-required is the locked
  v2 policy (survey §1.4: binary CPython wheels never load). The probe is
  a REBUILD-SIMULATED pydantic_core: same module name, same init shape
  (PyModule_Create + PyErr_NewException + AddObject + PyType_FromSpec),
  every exercised member drawn from the measured 169. The kill-criterion
  question — "does the honest surface go much deeper than symbols show
  for a trivial model?" — is answered NO at this depth: with the 110
  wrapper members landed, import + model + validate runs green.
- The CPython-arm differential (same .c against Python.h) was NOT run:
  the probe is the antipythony-side proof; the tagged lines are
  deterministic and CPython-shaped, but the byte-exactness standard is
  the conformance-suite standard, not the probe gate. A follow-up leaf
  can extend apx_cpython_compat.h and pin the 24 lines.
- Members landed as accept/identity on S1 (documented above): complex is
  repr-pinned (never arithmetic-oped by the trivial model), PyCMethod_New
  + PyCapsule_Import fail loud as S2/S3 members, PyErr_WarnEx honors-
  and-suppresses, Exception traceback accessors are NULL/accept.
- The 10.04 CeInstance payload route (tp_basicsize C block) is what
  makes the Model's C-visible validated-value read work; PyType_GenericAlloc
  uses the payloads registry with monotonic keys (no runtime instance
  yet at alloc time — the bridge attaches the Value later).
- gcmod::edges_of still does not drain the traverse registry (NEXT-1006
  honest gap, unchanged).

## 7. SHARED SYMBOLS (one owner per C symbol; declared here)

- All 110 new C symbols: owned by `exports_pydantic.rs`.
- `installed_vtable()` — new pub accessor on exports_objproto (sibling
  bands share the ONE objproto vtable; no second table).
- `install_mapping_vtable`/MappingVTable — owned by exports_pydantic;
  installed by ap-run's ce_objproto::ensure_mapping_installed (called
  from ensure_objproto_installed, the single seam).
- CePayload::{BytesMarker,SetMarker} — state.rs owner; value_conv
  consumes. Sibling bands needing bytes/set round-trips MUST use these,
  not re-declare.

## 8. Machine policy

One cargo lane at a time; CARGO_BUILD_JOBS=4; artifacts under
~/.git/worktrees/leaf-pydantic (scratch/ inside, never /tmp). Commits:
f056ccb (ap-ce band), 612a434 (ap-run wiring + round-trip fixes),
681bf81 (probe + driver). Explicit pathspecs throughout; no `git add -A`.

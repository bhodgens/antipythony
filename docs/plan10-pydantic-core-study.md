# Plan 10 case study: pydantic-core (REQUIRED by master.md tier table)

Status: **research leaf 10.01 input — complete.** This is the "REQUIRED CASE STUDY"
named in `plans/antipythony/10-c-extensions-stretch/orchestrator.md` (leaf 10.01) and
the recurring T3/T4 blocker in `plans/antipythony/master.md` (tier table rows T3
hermes-agent, T4 fastapi). It also discharges the "Decide at 10.01" note in
`docs/path-to-targets.md` (lines 61–62).

Method: every claim below is grounded in artifacts actually inspected on this
machine — no guessing, no web-only claims. Artifacts:

- Installed wheel copy: `pydantic_core 2.50.1` (cp314, macOS arm64) at
  `~/.cache/uv/archive-v0/NHB9MzeXEz4BRj9h/lib/python3.14/site-packages/pydantic_core/`
  (uv cache; same content as the PyPI wheel).
- Type stub: `pydantic_core/_pydantic_core/__init__.pyi` (1,118 lines, 48 KB).
- Compiled extension: `_pydantic_core.cpython-314-darwin.so`, 4,336,192 bytes,
  `Mach-O 64-bit dynamically linked shared library arm64` (`file(1)` output).
- Symbol scan: `nm -u` on the `.so` (237 unique undefined symbols, 173 of them
  CPython `Py*` imports); `strings(1)` for build metadata.
- Live introspection under Homebrew CPython 3.14: `dir()`, `list_all_errors()`.
- Repo files read: `plans/antipythony/10-c-extensions-stretch/orchestrator.md`,
  `plans/antipythony/master.md`, `docs/lxml-shim.md`, `docs/path-to-targets.md`,
  `crates/ap-run/src/builtins_modules.rs` (registry), `crates/ap-run/src/lxml.rs`
  (3,002 lines), `crates/ap-obj/src/lib.rs` (object-model rationale).

---

## 1. Surface audit: what pydantic-core actually exposes to Python

### 1.1 The native module `_pydantic_core` (the C extension proper)

From the `.pyi` and confirmed against live `dir(_pydantic_core)` on CPython 3.14:

**23 module-level public members: 16 classes, 4 functions, 3 constants.**

| Kind | Members |
| --- | --- |
| Core engine classes (2) | `SchemaValidator`, `SchemaSerializer` |
| URL classes (2) | `Url`, `MultiHostUrl` |
| Exception classes (8) | `SchemaError(Exception)`, `ValidationError(ValueError)`, `PydanticCustomError(ValueError)`, `PydanticKnownError(ValueError)`, `PydanticOmit(Exception)`, `PydanticUseDefault(Exception)`, `PydanticSerializationError(ValueError)`, `PydanticSerializationUnexpectedValue(ValueError)` |
| Support classes (4) | `ArgsKwargs`, `Some(Generic[_T])`, `PydanticUndefinedType`, `TzInfo(datetime.tzinfo)` |
| Functions (4) | `to_json`, `from_json`, `to_jsonable_python`, `list_all_errors` |
| Constants (3) | `__version__`, `build_profile`, `build_info` |

Method surface per class (from the `.pyi`, cross-checked with live `dir()`):

| Class | Public methods/properties | Notes |
| --- | --- | --- |
| `SchemaValidator` | 7: `validate_python`, `validate_json`, `validate_strings`, `validate_assignment`, `isinstance_python`, `get_default_value`, `title` | ctor takes a `CoreSchema` dict tree + optional `CoreConfig` |
| `SchemaSerializer` | 2: `to_python`, `to_json` | ctor mirrors validator |
| `Url` | 12 + `__str__/__repr__/__deepcopy__`: `scheme`, `username`, `password`, `host`, `unicode_host`, `port`, `path`, `query`, `query_params`, `fragment`, `unicode_string`, `build` | backed by Mozilla's `url` Rust crate (docstring in `.pyi`) |
| `MultiHostUrl` | 9 + dunders | same shape, `hosts` list |
| `ValidationError` | 5: `from_exception_data` (static), `title`, `error_count`, `errors`, `json` | the error payload pydantic users see |
| `TzInfo` | 4: `tzname`, `utcoffset`, `dst`, `fromutc` | subclasses `datetime.tzinfo` |
| `ArgsKwargs` | 2: `args`, `kwargs` | arguments carrier for `__init__` validation |
| others | 0–4 each | `Some.value`, `PydanticUndefinedType.__repr__/__copy__`, etc. |

Total: **~80 public methods/properties across the 16 classes**, plus 23 properties
and 6 static/classmethods per `@property`/`@staticmethod` counts in the `.pyi`.
`list_all_errors()` returns **111 built-in error types** (measured live) — each with
a snake_case id (`missing`, `int_parsing`, `url_scheme`, …), a message template with
`{placeholders}`, and an example context. That table is the exact differential
target for error-message fidelity.

### 1.2 The pure-Python half: `core_schema.py`

Crucially, **most of the package is NOT native**. `pydantic_core/core_schema.py`
(175,822 bytes) and `pydantic_core/__init__.py` (5,861 bytes) are plain Python:

- `core_schema.py`: **229 public names** — 99 TypedDict/type exports and 65
  `*_schema()` builder functions (measured via `dir()` + suffix scan under
  CPython 3.14). This is the schema DSL pydantic v2 compiles model definitions
  into before handing a dict tree to `SchemaValidator.__init__`.
- `__init__.py`: re-exports the 23 native members; the module has a custom
  `__getattr__` raising `AttributeError` for unknown names (observed live).

Consequence for antipythony: `core_schema.py` needs **no Plan 10 work at all** — it
runs on any interpreter whose `typing`/`TypedDict` machinery is faithful. Only the
23 native members (and through them, the entire validation/serialization engine)
require either a C-API path or a shim.

### 1.3 Build facts of the shipped `.so`

- Built with **PyO3 0.29.2** (`strings` shows
  `/Users/runner/.cargo/registry/src/index.crates.io-…/pyo3-0.29.2/src/impl_/pyclass.rs`
  and friends baked into the panic-location table).
- **Version-specific ABI, not abi3**: the filename is
  `_pydantic_core.cpython-314-darwin.so`; no `abi3` marker appears in the binary.
  Every CPython minor version needs a separate wheel — and per Plan 10's locked
  v2 policy ("Binary wheels built for CPython NEVER load"), this wheel is
  unloadable by antipythony regardless.
- 237 undefined symbols total: 173 CPython `Py*`, the rest libc/libunwind/
  libdispatch (`Unwind_RaiseException`, `dispatch_semaphore_*`, …) — the Rust
  panic-unwinding and std machinery rides along.

---

## 2. CPython C-API features this surface requires

`nm -u` gives the exact import list — 173 `Py*` symbols. Grouped into **19 feature
rows**, each with what antipythony would have to provide for a *recompiled* PyO3
extension (option a). "Present today?" refers to antipythony's native runtime
(`crates/ap-run`, `crates/ap-obj`) as of this writing.

| # | C-API feature (evidence: imported symbols) | What antipythony would need |
| --- | --- | --- |
| 1 | **Multi-phase module init**: `PyModuleDef_Init`, `PyModule_ExecDef`, `PyModule_FromDefAndSpec2`, `PyModule_GetNameObject` | A C-callable module bootstrap: `PyModuleDef` struct layout, spec-based exec, our importer invoking it. Our registry (`builtins_modules.rs`) is Rust-internal; nothing C-facing exists. |
| 2 | **Heap type creation**: `PyType_FromSpec`, `PyType_GenericAlloc`, `PyType_Freeze`, plus `PyType_GetName/GetQualName/GetModuleName`, `PyType_IsSubtype` | Full `PyType_Slot` table semantics (tp_new/tp_dealloc/tp_getattro/tp_richcompare/tp_iter/traverse/clear flags), heap-type lifetime under a **tracing GC (MMTk)** — the object-layout conflict Plan 10's kill criteria explicitly name. |
| 3 | **Type-object & singleton globals**: `PyType_Type`, `PyBaseObject_Type`, `PyLong_Type`, `PyUnicode_Type`, `PyDict_Type`, `PyList_Type`, … (16 type globals), `Py_NoneStruct`, `Py_TrueStruct`, `Py_FalseStruct`, `Py_EllipsisObject`, `Py_NotImplementedStruct` | Exported **data symbols** at stable addresses matching CPython's `PyObject` header expectations (`ob_refcnt`/`ob_type`). ap-obj is an enum-of-8 tagged `Value` with `Gc<T>` handles (see `crates/ap-obj/src/lib.rs`) — there is no refcounted `PyObject` header to point at. This is the single hardest row. |
| 4 | **Object protocol calls**: `PyObject_GetAttr/SetAttr/SetAttrString/GetItem/SetItem/DelItem/Call/CallOneArg/CallNoArgs/RichCompare/Hash/Repr/Str/IsTrue/Size/GetIter/IsInstance/IsSubclass/Dir/LengthHint/HasAttrWithError` (~20 fns) | C entry points for every protocol our interpreter dispatches internally via `special_lookup`/`native_attr`. Mechanical but wide; each needs error-state conventions. |
| 5 | **Generic attribute machinery**: `PyObject_GenericGetDict`, `PyObject_GenericSetAttr`, `PyObject_GenericSetDict` | Instance `__dict__` slots on extension types — our class objects (02.03) hold attrs in Rust structs, not a PyObject dict offset. |
| 6 | **Vectorcall ABI**: `PyObject_Vectorcall`, `PyObject_VectorcallDict`, `PyObject_VectorcallMethod`, `PyCMethod_New` | The calling convention PyO3 0.29 uses for *all* calls. Must match CPython's `PyObject **args, size_t nargsf, PyObject *kwnames` layout exactly, including the `PY_VECTORCALL_ARGUMENTS_OFFSET` flag bit. |
| 7 | **Refcount/dealloc surface**: `Py_Dealloc`, `PyObject_Free`, `PyObject_Init` | Refcount semantics emulated over a tracing collector (bump-on-borrow or deferred reclamation). GraalPy spent years here; it is the row Plan 10's kill criterion #2 targets. |
| 8 | **GC integration**: `PyObject_GC_Del`, `PyObject_GC_UnTrack`; `tp_traverse`/`tp_clear` arrive as *slots* through `PyType_FromSpec` (no imported symbol), and PyO3's own strings confirm traversal (`assertion failed: self.has_traverse`) | Cycle-collection callbacks reconciled with MMTk: MMTk traces natively, so `tp_traverse` must be driven by our GC or faked. Doable only if extension objects live in a GC-visible heap — foreign C memory does not. |
| 9 | **Exception machinery**: `PyErr_SetString/SetObject/GetRaisedException/SetRaisedException/GivenExceptionMatches/NewExceptionWithDoc/PrintEx/WarnEx/WriteUnraisable`, `PyException_Get/SetCause/Get/SetTraceback`, 17 `PyExc_*` globals | Our `exc_hier.rs` exceptions re-exposed as C globals + a C-visible error-indicator thread state (we are no-GIL by design — per-interpreter error state must still behave per-thread). |
| 10 | **Unicode**: `PyUnicode_FromStringAndSize/AsUTF8AndSize/EqualToUTF8AndSize/New/InternInPlace/AsEncodedString`, `PyUnicodeDecodeError_Create`, and the **3.14-new writer API** `PyUnicodeWriter_Create/WriteUTF8/WriteChar/Finish/Discard` (+ inline `PyUnicode_DATA`) | Byte-level access to string internals. ap-obj strings are CompactAscii-or-Rust-`String` (UTF-8) — `PyUnicode_DATA`'s UCS2/UCS4 kind assumptions don't hold; we'd ship replacement headers, but PyO3's *compiled-in* inline expansions were built against CPython's headers. |
| 11 | **Int**: `PyLong_From*/As*` (8 fns), **3.13+ compact-int API** `PyLongWriter_Create/Finish`, `PyLong_Export/FreeExport/GetNativeLayout/AsNativeBytes/FromNativeBytes` | Our i61-smi + BigInt two-level representation (ap-obj `SMI_BITS`) bridged to CPython's digit-array export layout. `PyLong_Export` hands out raw digit pointers — direct conflict with the smi fast path. |
| 12 | **Float/complex**: `PyFloat_AsDouble/FromDouble`, `PyComplex_FromDoubles/RealAsDouble/ImagAsDouble` | Trivial (we have both types); complex needs a new ap-obj variant or shim. |
| 13 | **Dict**: `PyDict_New/Copy/Clear/Contains/GetItemRef/SetItem/DelItem/Next/Keys/Update` | Our compact dict exists; `PyDict_Next` exposes insertion-order iteration positions — semantics we already honor (C-contract), needs C-facing cursors. |
| 14 | **Containers**: `PyList_New/Append/AsTuple/GetSlice`, `PyTuple_New/GetItem`, `PySet_New/Add/Contains/Size`, `PyFrozenSet_New`, `PyBytes_FromStringAndSize/AsString/Size`, `PyByteArray_AsString/Size` | Mostly mechanical wrappers over existing ap-obj types. **Note: no buffer protocol** — `PyObject_GetBuffer` is *absent* from the import list; bytes access goes through `PyBytes_AsString` only. One classic hard feature we do NOT need. |
| 15 | **Iteration/number protocol**: `PyIter_Check`, `PyIter_NextItem` (3.14-new), `PyObject_GetIter`, `PyMapping_Items`, `PySequence_Check/Contains`, `PyNumber_Index/Remainder/TrueDivide` | Wiring our `iter.rs` protocol to C entry points. |
| 16 | **Thread/GIL state**: `PyGILState_Ensure/Release`, `PyEval_SaveThread/RestoreThread`, `PyInterpreterState_Get/GetID`, `Py_IsInitialized/IsFinalizing` | PyO3's `Python::attach`/`allow_threads` model assumes a GIL to release. antipythony is **no-GIL (locked decision, master.md)**: `PyGILState_Ensure` becomes a no-op and `SaveThread` a token — behaviorally fine for single-thread validation, but any PyO3 code path that infers thread-safety from the GIL is a latent hazard. |
| 17 | **Capsule & import**: `PyCapsule_Import`, `PyImport_Import` | Needed by row 18; general C-level import of Python modules from Rust. |
| 18 | **datetime C-API interop**: `PyCapsule_Import` + the PyO3 string `failed to import 'datetime' C API`; `TzInfo` subclasses `datetime.tzinfo`; validators construct `date`/`datetime`/`timedelta` objects | The `PyDateTime_*` capsule table (CPython's only sanctioned way for extensions to touch datetime) must exist and return objects that are `isinstance`-compatible with our stdlib `datetime` — i.e. C-created datetimes and Python-visible datetimes must be the *same* representation. |
| 19 | **Panic/FFI boundary**: libunwind symbols (`Unwind_RaiseException`, `Unwind_Resume`, …), PyO3 `PanicException` strings | Rust panics converted to Python exceptions across the boundary — for a *recompiled* extension this means our loader must link the extension's unwinder to ours and provide a `PanicException` type. |

**Feature count: 19 rows / 173 distinct imported CPython symbols.** Difficulty is
bimodal: rows 4, 6, 9, 12–15, 17 are mechanical wrappers (the 10.02 spike should
measure hours-per-function here); rows 2, 3, 5, 7, 8, 10, 11, 16, 18 touch the
**object layout / refcount / GC / no-GIL** core that Plan 10's kill criteria call
out ("Any API requires CPython object layout byte-compatibility we cannot provide
without abandoning the GC design").

One important positive: PyO3 0.29 emits **modern-API-only** code — everything goes
through `PyType_FromSpec`, vectorcall, `PyLongWriter`/`PyUnicodeWriter`,
`PyDict_GetItemRef`. No legacy `Py_InitModule`, no direct struct-field pokes appear
as symbols (the layout-sensitive parts are the *inline header expansions* in rows
3/10/11, which a replacement-headers strategy can address only if the extension is
recompiled against OUR headers — exactly the Plan 10 v2 "rebuild-required" policy).
PyO3 upstream already supports alternate runtimes (GraalPy, RustPython) behind
config gates, so a PyO3-built crate like pydantic-core is the *most* portable class
of C extension we could hope to face.

---

## 3. Strategic options

### Option (a): C-API compatibility layer sufficient for PyO3 output

Implement the 19 rows above as a C-callable shim (headers + `contrib/ce-compat/`
crate) so pydantic-core *recompiled from source* links against antipythony.

**Cost (honest):**
- The mechanical half (~150 of 173 symbols: rows 1, 4, 6, 9, 12–15, 17) is what
  the 10.02 spike exists to price. Even at an optimistic 2–4 h/API function that
  is 300–600 h before the hard rows.
- The hard half is existential, not incremental: rows 3/7 demand a refcounted,
  fixed-layout `PyObject` header at stable addresses; ap-obj is a 16-byte tagged
  `Value` enum behind MMTk `Gc<T>` handles with **movable** heap objects
  (`crates/ap-obj/src/lib.rs` — NaN-boxing and tagged pointers were *rejected*
  precisely because MMTk handles aren't pointers). Providing CPython-shaped
  `ob_refcnt`/`ob_type` headers means either (i) a pinned, non-moving arena for
  extension-visible objects with refcount↔MMTk bridging, or (ii) byte-layout
  compatibility we've already decided against. Row 8 (`tp_traverse` over foreign
  memory) and row 16 (GIL-token semantics under no-GIL) add design risk on top.
- Row 18 couples the datetime C-API to our stdlib datetime representation —
  a cross-crate contract that would outlive pydantic-core.
- Prior art calibration: GraalPy ≈ a decade of effort for a C-API subset;
  RustPython stalled (orchestrator.md prior-art list). PyO3-modern-only output
  shrinks the target but does not remove the layout/refcount core.
- Every win is general (unlocks torch/T5 someday), every delay is unbounded.

**Benefit:** the only option that reaches T5 (comfyui) and the long tail; makes
10.03/10.04 meaningful; pydantic-core would run *unmodified logic* (its own test
suite is the differential gate).

**Kill-criterion status:** rows 3/7/8 sit squarely inside Plan 10's kill criterion
#2. Option (a) cannot be honestly recommended *before* the 10.02 density spike
measures it; this study's symbol scan is the input that spike needs.

### Option (b): native `pydantic_core` compat module in Rust (the lxml-shim precedent)

Implement the 23-member native surface directly against ap-obj as a registered
native module — exactly how `lxml` was shimmed (`docs/lxml-shim.md`: 3,002-line
`crates/ap-run/src/lxml.rs`, registered in `builtins_modules.rs`, 24 differential
tests byte-compared against CPython + real lxml).

**Cost (honest):**
- We re-implement a *validation/serialization engine*, not a parser subset: the
  upstream crate is tens of thousands of lines of Rust (the strings dump shows
  ~50 validator families — typed-dict, dataclass, union discriminators,
  constrained numbers/strings, url, uuid, decimal, fraction, complex, timedelta,
  recursive JSON with partial-parse support). lxml was ~3k lines for a subset;
  a fastapi/hermes-agent-sufficient pydantic_core is plausibly **6–12k lines**.
- The differential target is unusually well-specified: 111 error types with exact
  message templates (`list_all_errors()`), the `.pyi` contract, and pydantic v2's
  own test corpus. That makes it *testable to byte-exactness* the way lxml was —
  but error-message and `repr` fidelity across 111 templates is grind work.
- License path is clean and already paved: pydantic-core is MIT, and master.md
  line 53 **already lists `pydantic/monty` (MIT, Rust) as an approved donor** —
  monty is pydantic-core's JSON serializer core. We can adapt monty (with
  attribution) for the `to_json`/`from_json` half rather than write a JSON engine.
  Upstream validator logic, however, is PyO3-saturated (`&Bound<PyAny>` inputs
  everywhere) — it ports as *design reference*, not drop-in code.
- Scope discipline bounds it: T3/T4 exercise a narrow slice — `SchemaValidator`
  (validate_python/validate_json), `SchemaSerializer` (to_python/to_json),
  `ValidationError(.errors()/.json())`, `Url`, `TzInfo`, the exception family,
  and the schema kinds pydantic v2 emits for model/dataclass/typed-dict fields.
  `core_schema.py` itself already runs unmodified (pure Python, §1.2).
- Risk: pydantic v2→v3 schema churn (v3 in development per upstream signals —
  `Some`, `Sentinel`, `PydanticMissing` strings in the 2.50.1 binary). Each
  pydantic release can move the contract the shim tracks. lxml had no such
  version-treadmill problem.
- Wasm: a pure-Rust shim works on the WasmGC target for free (master.md Wasm
  constraint: "pure-Rust implementations are REQUIRED" — this option satisfies
  it; option (a) explicitly does not, per orchestrator leaf 10.04 "far stretch").

**Benefit:** decouples T3/T4 from the hail-mary plan entirely (the isolation rule
in orchestrator.md says nothing in Plans 00–04 may depend on Plan 10 — a shim in
`crates/ap-run` respects that, option (a) as a T3/T4 prerequisite would violate
it). Bounded, testable, precedented, wasm-compatible.

**Downside vs (a):** zero generalization. Every future C extension needs its own
shim (that's the lxml precedent's known ceiling — see `docs/lxml-shim.md` "Known
gaps"), and T5 (torch) is unreachable by shimming.

### Option (c): declare C-extensions out of scope; pick different T3/T4 targets

Drop pydantic-dependent targets. Alternatives: T3 → a pydantic-free agent path
(master.md T3 row already says "pydantic OR pydantic-free path"); T4 → a
non-fastapi ASGI framework with no native deps (e.g. litestar-without-pydantic
still pulls msgspec — also a C extension; blacksheep — C extension; the honest
survey result is that *most* modern async web stacks have at least one native
accelerator). pydantic v1 is pure Python but fastapi ≥0.100 and hermes-agent's
stack target pydantic v2 semantics; `docs/path-to-targets.md` line 62 already
flags "pydantic v1 pure-python fallback (slower but pure)" as the alternative.

**Cost:** cheap in engineering, expensive in signal value — the tier table exists
to prove antipythony runs *real, popular* software. Retargeting T3/T4 to
exotic-dependency-free software weakens exactly the claim the tiers make, and
doesn't answer the T5 question at all.

**Benefit:** zero Plan 10 exposure; tiers remain independent per master.md rules.

---

## 4. Recommendation

**Recommendation: Option (b) as the T3/T4 path — build a native `pydantic_core`
compat module in `crates/ap-run` (lxml-shim pattern), reusing `pydantic/monty`
(already an approved MIT donor, master.md line 53) for the JSON core. Keep Plan 10
option (a) alive on its own isolated track for T5, fed by this study's 173-symbol
scan, but let nothing in T3/T4 wait on it.**

Rationale:

1. The isolation rule is binding: orchestrator.md — "nothing in Plans 00-04 may
   depend on any leaf here." Gating T3/T4 on option (a) violates the plan tree's
   own contract; option (b) is Plan-02-style work in the mainline.
2. The hard rows of the C-API surface (§2 rows 3/7/8) collide with two *locked*
   decisions (MMTk tracing GC, no GIL). Option (a) is a re-litigation risk;
   option (b) touches neither.
3. The differential-test methodology is proven: lxml landed 24 byte-compared
   tests against real lxml; pydantic_core's contract is even better specified
   (the `.pyi` + `list_all_errors()`'s 111 templates give an exhaustive oracle).
4. Scope is bounded by actual T3/T4 usage (a narrow slice of §1.1), and
   `core_schema.py` — the largest file in the package — already runs unmodified.

Sequencing: shim lands behind the native-module registry
(`builtins_modules.rs`, alongside `lxml`/`struct`); a differential suite runs
pydantic v2 + fastapi TestClient scenarios against CPython 3.14 + pydantic-core
2.50.1 (the exact wheel audited here). Plan 10 leaf 10.02 should still execute its
spike using the 19-row table as the API shopping list — its density measurement is
what keeps option (a) honest for T5.

### Decision criteria that would flip this recommendation

- **Flip to (a)** if the 10.02 spike measures the mechanical half at < ~2 h/API
  function AND a design emerges for rows 3/7 (PyObject header over MMTk) that
  does not require abandoning movable-object GC — i.e., the extrapolated
  PyO3-subset cost lands under Plan 10's 2-person-year kill threshold. Then (a)
  subsumes (b) long-term (one engine unlocks torch/T5 and the whole wheel
  ecosystem), and the shim becomes a stopgap.
- **Flip to (c)** if the shim's differential surface exceeds the T3/T4 slice —
  concretely, if pydantic v3 schema churn or fastapi's usage turns out to need
  most of the ~50 validator families at byte-exact fidelity (estimate blows past
  ~12k lines / the corpus stops converging). At that point the honest move is
  master.md's already-written escape hatch: T3 via the pydantic-free path, T4
  retargeted.
- **Hard stop on (a) regardless** if any spike finding confirms kill criterion #2
  literally (extension-visible objects cannot get CPython-layout headers without
  pinning against MMTk) — then Plan 10 remains research-only and (b)/(c) are the
  only live options for all tiers below T5.

---

## Appendix A: raw measurements (reproduce)

```
# artifact
~/.cache/uv/archive-v0/NHB9MzeXEz4BRj9h/lib/python3.14/site-packages/pydantic_core/
    _pydantic_core.cpython-314-darwin.so   4,336,192 B  Mach-O arm64
    _pydantic_core/__init__.pyi            1,118 lines
    core_schema.py                         175,822 B (pure Python)

# symbol scan
nm -u _pydantic_core.cpython-314-darwin.so
  → 237 unique undefined symbols; 173 CPython Py* imports
  → 0 × PyObject_GetBuffer (no buffer protocol)
  → PyO3 0.29.2 (cargo-registry paths in strings)
  → no abi3 marker; cpython-314 version-specific ABI

# live surface (CPython 3.14)
dir(_pydantic_core) public members      → 23
_pydantic_core.list_all_errors()        → 111 error types
core_schema public names                → 229 (99 types, 65 *_schema builders)
SchemaValidator public methods          → 7; SchemaSerializer → 2
```

## Appendix B: sources cited

- `plans/antipythony/10-c-extensions-stretch/orchestrator.md` — required case
  study, v2 rebuild-required policy, kill criteria, isolation rule, 10.02 spike.
- `plans/antipythony/master.md` — tier table (T3/T4 blockers), locked decisions
  (MMTk, no-GIL), donor list (monty line 53), Wasm pure-Rust constraint.
- `docs/lxml-shim.md` — shim precedent, architecture, differential-test pattern.
- `docs/path-to-targets.md` lines 61–62 — "Decide at 10.01".
- `crates/ap-obj/src/lib.rs` — Value representation rationale (enum-of-8, Gc<T>,
  smi i61) grounding §2 rows 3/7/11 difficulty.
- `crates/ap-run/src/builtins_modules.rs` — native-module registry (lines 768–773:
  `struct`, `lxml`) where a `pydantic_core` shim would register.
- pydantic_core 2.50.1 wheel artifacts as listed in Appendix A (`.pyi`, `nm`,
  `strings`, live introspection). PyO3 API claims cross-referenced against the
  symbol evidence in the binary itself rather than docs alone; where the PyO3
  docs are the natural citation (vectorcall convention, `PyType_FromSpec` slots,
  datetime capsule), the imported-symbols list is the stronger, local proof.

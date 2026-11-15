# C-Extension Compatibility Survey (Leaf 10.01)

Measured C-API surface of real PyPI native-extension wheels; feasibility verdict and
recommended strategy for Plan 10. Research-only leaf; nothing here blocks or gates
any other plan (isolation rule, orchestrator.md).

- **Date:** 2026-11-14
- **Method:** scripted symbol scan (not guesses) — see
  `docs/measure-ce-surface.py`; raw per-package data in `docs/wheel-scan-results.json`.
  32 packages downloaded from PyPI (manylinux x86_64, cp312 or abi3), every `*.so`
  extracted, `nm -D --undefined-only` / `--defined-only` run per object, `Py*`/`_Py*`
  undefined symbols collected, deduplicated, and cross-checked against the official
  stable-ABI member list (via `abi3info` 2026.9.25, which vendors CPython's
  `stable_abi.toml`).
- **Prior art:** componentize-py, GraalPy docs/changelog, RustPython issues/PRs,
  CPython PEP 384 / PEP 659 / PEP 703 (Section 5).

---

## 1. Measurement results (26 packages with native code)

| package | `.so` | Py* undefined | stable-ABI % | note |
|---|---|---|---|---|
| numpy 2.2.6 | 22 | 383 | 89% | deepest surface incl. `_PyUnicode_Is*`, TraceMalloc |
| lxml 6.1.3 | 7 | 316 | 88% | |
| uvloop 0.23.0 | 1 | 274 | 89% | heavy thread/GIL + frame APIs |
| grpcio 1.85.1 | 1 | 234 | 88% | |
| sqlalchemy 2.1.8 | 8 | 223 | 88% | C accelerators |
| aiohttp 3.14.5 | 4 | 220 | 90% | |
| msgpack 1.2.4 | 1 | 195 | 89% | |
| cffi 2.1.1 | 1 | 171 | 93% | ABI-mode dlopen; many `_Py*` internals |
| **pydantic-core 2.50.4** | 1 | **158** | **96%** | **case study, §4** |
| cryptography 50.0.2 | 1 | 148 | **100%** | abi3 wheel (`cp311-abi3`) |
| pyyaml 6.0.3 | 1 | 145 | 90% | |
| contourpy 1.3.2 | 1 | 138 | 93% | |
| multidict 7.2.1 | 2 | 124 | 86% | |
| httptools 0.6.4 | 2 | 119 | 88% | |
| pillow 12.2.0 | 26 | 106 | 95% | |
| regex 2026.11.9 | 1 | 95 | 96% | |
| greenlet 3.2.5 | 3 | 89 | 90% | + does raw stack switching (special case, §6) |
| rpds-py 2026.9.1 | 1 | 84 | 99% | |
| zstandard 0.25.0 | 2 | 80 | 99% | |
| tiktoken 0.11.0 | 1 | 74 | **100%** | Rust ext |
| bcrypt 5.0.0 | 1 | 67 | **100%** | Rust ext, abi3 |
| orjson 3.13.0 | 1 | 63 | 87% | Rust ext |
| kiwisolver 1.5.1 | 1 | 36 | 94% | |
| brotli 1.2.1 | 1 | 24 | 100% | |
| google-crc32c 1.9.0 | 2 | 5 | 100% | |
| markupsafe 3.0.4 | 1 | 2 | 50% | |

Pure-python controls (pydantic, pydantic-settings, protobuf, pyjwt, frozenlist,
pyzstd as downloaded) ship 0 native `.so` in the fetched wheels — their C code lives
in sibling wheels or was already moved out. **Total measured: 3,573 unique Py\* symbol
imports across 26 packages; union = 587 unique symbols.**

### 1.1 Union shape: 587 symbols, steep head, long tail

- **Core (imported by ≥10 of 26 packages): 136 symbols.** This is the de-facto
  minimum viable C-API. Contents: error set/occurred/matches family, `PyExc_*`
  (13 exception types), tuple/list/dict constructors+accessors, `PyObject_{Get,Set}
  Attr{,String}`, `PyObject_Call*` + vectorcall, `PyLong_From/As` basics,
  `PyUnicode_{New,From*,AsUTF8AndSize,Intern*}`, `PyType_{Ready,IsSubtype,Modified}`,
  `PyModuleDef_Init`, `PyMem_{Malloc,Free,Realloc}`, `PyObject_GC_{Track,UnTrack,Del}`,
  `PyGILState_{Ensure,Release}`, `PyEval_{Save,Restore}Thread`, `_Py_Dealloc`,
  `_Py_NoneStruct` + the singleton struct exports, `_PyType_Lookup`,
  `_PyThreadState_UncheckedGet`, buffer protocol, capsules.
- **Mid (3–9 packages): 220 symbols.**
- **Tail (<3 packages): 231 symbols.** NumPy's 47 unique symbols alone include
  `_PyUnicode_Is*` character-class internals, `PyTraceMalloc_*`,
  `PyNumber_MatrixMultiply`, contextvars.
- **Zero direct struct-field access** appears in undefined symbols (no
  `tp_*`/`ob_refcnt` symbol pokes) — layout cheating happens through *headers at
  compile time*, not through dynamic symbols (see §2, the hidden dependency).

### 1.2 Stable-ABI overlap: 91% of measured imports

3,258 of 3,573 measured symbol imports (and 484 of 587 union symbols) are members of
PEP 384's stable ABI. The 103 outside:

- **74 are `_Py*` private** (`_PyDict_NewPresized`, `_PyBytes_Resize`,
  `_PyUnicode_Is*`, `_PyLong_{As,From}ByteArray`, `_Py_Dealloc`, `_Py_NoneStruct`…).
  These are not stable but are *de-facto standard*: PyO3 emits `_Py_Dealloc` and the
  singleton structs for every non-abi3 build; `_PyLong_{As,From}ByteArray` are used by
  pydantic-core and others for 128-bit ints. **Must implement anyway.**
- **40 public-but-non-stable functions** (complex, contextvars, `PyConfig_*`,
  `PyErr_GetRaisedException`, `PyUnicode_New`…).
- **Frame/code (5):** `PyFrame_{New,GetBack}`, `PyCode_NewEmpty`,
  `PyGen_GetCode` — traceback plumbing (greenlet, Cython-generated code).
- Long-tail: `PyThreadState_{Enter,Leave}Tracing`, `PyLong_FromUnicodeObject`.

**Conclusion: the stable ABI is a good *floor*, not a sufficient target. The real
target is stable-ABI ∪ (PyO3 emission set) ∪ (a numpy/-ish tail) ≈ 587-symbol union.**

### 1.3 The hidden layout dependency (not visible in symbols)

Undefined symbols cannot show the biggest constraint: **extensions compiled against
CPython headers contain *statically defined* `PyTypeObject` structs** — C structs laid
out per CPython's memory layout, filled with slot function pointers, then passed to
`PyType_Ready`. Every hand-written C extension does this; PyO3 instead uses
`PyType_FromSpec` (pydantic-core: `PyType_FromSpec` + `PyType_GenericAlloc` + zero
static type structs — measurably the cleanest possible client).

A rebuild-required boundary (v2 policy) sidesteps this: we ship our *own*
`Python.h`-compatible headers where `PyTypeObject` is **our** layout. Source that
compiles against our headers cannot embed CPython layout. This is exactly GraalPy's
and PyPy's argument for refusing ABI parity ("just impossible without replicating all
CPython internals"). It means: **binary wheels never load — recompile is mandatory** —
already the locked v2 policy; this survey confirms it is not merely convenient but
load-bearing.

---

## 2. Feasibility subsets

| subset | contents | est. size | feasible? |
|---|---|---|---|
| S0 "hello extension" | module def, `PyArg_Parse*`, basic converters, `PyType_FromSpec`, refcount emu, exc set/raise | ~120 fns | **yes — 10.02 spike target** |
| S1 abi3+PyO3 floor | S0 + full stable ABI + `_Py_Dealloc`/singleton structs + vectorcall + `_PyLong_{As,From}ByteArray` | ~500 members | **yes, Rust exts (pydantic-core, tiktoken, bcrypt, orjson, cryptography) land here** |
| S2 core-C floor | S1 + static-`PyTypeObject` header emulation + buffer protocol + `tp_*` slot surface + weakrefs + `PyCapsule` | +~100 | **yes, covers ~2/3 of measured packages** |
| S3 numpy tier | S2 + `_PyUnicode_Is*`, TraceMalloc, in-place number protocol, contextvars, `PyInterpreterState_*`, `PyOS_*` | +~60 | **mostly yes; perf is the risk, not semantics** |
| S4 embedder tier | `PyConfig_*`, subinterpreters, `PyThreadState` introspection, frame walking (cffi ABI mode, greenlet) | +~50 | **partial — greenlet needs stack-switch hooks into our fibers; cffi ABI-mode needs a libpython-lookalike** |
| R refused | direct-layout binary wheels, `Py_LIMITED_MODULE` cheats, CPython-internals pointers (`PyFrameObject` field walks), immortal-refcount arithmetic relied upon | — | **no — declared boundary, same as GraalPy/PyPy** |

The GIL question, measured: every one of the 26 packages calls
`PyGILState_Ensure`/`PyEval_SaveThread` or nothing at all. None relies on the GIL for
correctness beyond reentrancy-into-Python (which our no-GIL runtime satisfies
trivially — reentrancy is safe when everything is thread-safe). We implement
`PyGILState_Ensure` as a no-op-ish accessor and PEP 703 APIs
(`Py_mod_gil`, critical sections) per the 02.09 flip. This is a *differentiator*:
GraalPy kept a GIL solely to satisfy C extensions; we don't have to.

---

## 3. Case study: pydantic-core (the T3/T4 blocker)

Measured (pydantic_core 2.50.4, one 12MB `.so`, Rust/PyO3):

- **158 Py\* imports; 151 (96%) are stable-ABI members.**
- The 7 outside: `PyFunction_Type`, `PyObject_CallOneArg`, `PyObject_LengthHint`,
  `PyObject_VectorcallDict`, `PyUnicode_New`, `_PyLong_AsByteArray`,
  `_PyLong_FromByteArray` — all small, all implementable, none layout-bound.
- Type creation goes entirely through `PyType_FromSpec`/`PyType_GenericAlloc` —
  **no static `PyTypeObject`, no hidden layout dependency** (PyO3's design).
- GC interaction is the polite subset: `PyObject_GC_Track/UnTrack/Del` only —
  compatible with a tracing GC that fakes refcounts (GraalPy's proven model:
  opaque handle + emulated refcount field over a real tracing collector).
- Threads: `PyGILState_Ensure/Release`, `PyEval_Save/RestoreThread` only.

**Verdict: pydantic-core is reachable on subset S1.** The single sharpest blocker for
T3 (hermes-agent) and T4 (fastapi) needs ~500 stable-ABI members plus ~7 extras.
Zero surprises in the measured surface. This is the strongest early signal that
T3/T4 are reachable via Plan 10.

---

## 4. Prior art (research summary)

- **componentize-py** (Apache-2.0): ships *real* CPython (libpython → wasm) +
  "pseudo-dynamic-linking" lookup tables instead of `dlopen`. 100% C-API by
  construction, ~35MB hello-world, keeps the GIL, tracks a patched CPython fork.
  Lesson: full fidelity is buyable only by *being* CPython — which forfeits our
  no-GIL/GC/i64-value design. Not our strategy; useful as a conformance oracle.
- **GraalPy**: source-compatible C-API over a foreign (JVM) object model;
  "the complete API header files are the same as for CPython, to be source
  compatible"; explicit refusal of ABI parity ("just impossible without replicating
  all CPython internals, same as PyPy"); rebuilt-wheels-only; keeps a GIL "because C
  extensions expect to do so"; opaque `GraalPyObject` (refcount + handle index) over
  the tracing GC; no published %-coverage number, ecosystem metric instead (93% of
  top-600 PyPI packages install, 65%+ tests pass). Lesson: **the emulation model
  works when given a decade of per-package grinding**; the refcount+handle trick is
  directly applicable to our `Gc<T>` + i64 values.
- **RustPython**: stalled for ~8 years at "hundreds of stubs" depth — issue #8156
  counts 436 stable-ABI functions still missing; PR #8435 C-slot trampolines
  "unfortunately does not work" (PyO3 needs C `tp_new` on all base types); core
  objection #1940: mimicking the C API requires simulating object memory layout,
  "the problem PyPy has". Lesson: thin-wrapper, no-layout-emulation, no-`tp_*`
  approaches die; the type-slot trampoline is where Rust extensions actually land.
- **CPython internals**: PEP 384 — "the primary source of ABI incompatibility are
  changes to the lay-out of in-memory structures" → opaque types +
  `PyType_FromSpec` is exactly the surface that avoids layout. PEP 659 —
  specialization is interpreter-internal with internal guards; no C-API constraint
  on our i64-tagged/typed-region design. PEP 703 — free-threaded builds already
  broke stable-ABI compat ("will not be ABI compatible"), biased refcounts,
  `Py_REFCNT` "do not rely on the returned value to be accurate" — **legitimizes
  refcount-as-emulation** for our no-GIL runtime and means we target the 3.14
  free-threaded C-API dialect as our primary dialect.

---

## 5. Recommended approach

**Proceed, scoped: a source-level C-API shim (our own headers + `ap-obj`-backed
symbol table), targeting the measured union (587 symbols), phased S0→S3, with a
declared refused set (R) and rebuild-required boundary.**

1. **New extension ABI, CPython-source-compatible headers.** We provide
   `Python.h`-shaped headers whose `PyTypeObject` etc. are *our* layout, and a
   Rust-side dispatcher (C shim crate under `contrib/ce-compat/`) that maps ~590
   C symbols onto `ap-obj`/`ap-run` values. Extensions recompile via
   `antipythony build-ext` (10.03). Binary CPython wheels fail loudly (locked v1
   behavior stays for them).
2. **Refcounts are emulation.** `Py_INCREF/DECREF` manipulate a per-object emulated
   count; real memory ownership stays with the MMTk tracing GC (GraalPy model).
   `Py_REFCNT` returns the emulation; no extension may rely on exactness (same rule
   PEP 703 states). Immortal singletons (`_Py_NoneStruct`…) are immortal.
3. **Type slots via trampolines.** Implement `tp_*` slot dispatch as generated
   thunks that call back into Rust — the exact point RustPython's PR #8435 failed;
   do it first, not last (PyO3/pydantic-core and all Cython output need it).
4. **Vectorcall-first call path.** `PyObject_Vectorcall*` + `PyVectorcall_Function`
   are the modern call ABI; `tp_call`/`PyArg_Parse*` reduce onto it.
5. **Target the free-threaded (PEP 703) 3.14 dialect.** `Py_BEGIN_CRITICAL_SECTION`
   no-ops into our lock-free maps; `Py_mod_gil` accepted-and-ignored; thread-state
   APIs (`PyGILState_*`, `PyEval_SaveThread`) implemented as cheap accessors to a
   per-thread interpreter handle (our 02.09 per-worker registries already model
   this).
6. **Priority order = ecosystem weight:** S0/S1 lands pydantic-core + tiktoken +
   bcrypt + cryptography + orjson (all Rust/PyO3, ~96-100% stable-ABI). S2 lands the
   C long tail. S3 lands numpy/lxml/uvloop/grpcio. greenlet (asyncio stack, used by
   sqlalchemy) is the hardest single package (raw stack switching) — schedule it
   against our fiber implementation explicitly, not as generic C-API work.

### Kill criteria (from orchestrator.md, confirmed executable)

- **Cost density:** 10.02's spike must measure hours/API-function; the S0+S1 surface
  (~620 members) times that density must be < ~2 person-years, else STOP.
  (Prior estimate from GraalPy history: this is plausibly 6-18 months for S0-S2;
  numpy-tier S3 is where cost explodes and where the criterion most likely bites.)
- **Layout byte-compat:** none of the measured union *requires* CPython object
  layout given rebuild-required + our-headers (pydantic-core provably zero;
  C extensions' static `PyTypeObject` is re-laid-out by our headers). Criterion
  clean — unless a numpy-tier package reveals a hard layout dependency at S3.
- **Conformance regression:** any C-API accommodation that perturbs Plans 02-03
  byte-exactness must go behind an opt-in import path (registry pattern as in
  02.09's per-interp rows), never the default object model.

### Risks (measured, not guessed)

- numpy's 47 unique symbols include `_PyUnicode_Is*` character classes and
  `PyTraceMalloc_*` — small but high-count; numpy's *real* depth is in its
  runtime capsule protocol (`PyCapsule_Import` of `numpy._core._multiarray_umath`),
  i.e. numpy extensions import numpy's own C-API through capsules — that must work
  for the numpy *ecosystem*, numpy alone is not the finish line.
- cffi ABI-mode (`dlopen` + libffi, no compilation) wants a `libpython`-lookalike
  process surface — keep cffi on API-mode (recompile) and declare ABI-mode refused
  in v2.
- greenlet stack switching interacts with our fibers (AP_GEN_STACK_MB) — its 89
  imports are trivial; the stack dance is not. Isolate as its own leaf if asyncio +
  sqlalchemy ever needs it.

---

## 6. Measurement artifacts

- `docs/measure-ce-surface.py` — download + `nm` scan + classification script
  (rerunnable; PyPI test-data use only, nothing redistributed).
- `docs/wheel-scan-results.json` — raw per-package results: wheel filename, per-`.so`
  counts, full `Py*` undefined-symbol lists, category buckets.

Caveats: cp312/abi3 manylinux x86_64 wheels only (a mac-arm rerun would likely match
within a few %); symbol *imports* measure surface demand, not runtime behavior depth
(buffer protocol subtleties, `tp_traverse` correctness, etc. show up as "same symbol,
deeper semantics"); 26 packages is a solid sample of the top-depended-on native tier
but not the full top-50 native set — the tail adds symbols, not new *categories*.

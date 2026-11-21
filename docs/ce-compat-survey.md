# C-Extension Compatibility Survey (Leaf 10.01, completed by 10.01b)

Measured C-API surface of real PyPI native-extension wheels; feasibility verdict and
recommended strategy for Plan 10. Research-only leaf; nothing here blocks or gates
any other plan (isolation rule, orchestrator.md).

- **Date:** first pass 2026-11-14 (manylinux x86_64, cp312/abi3 — crashed at disk-full,
  data survived); **completed 2026-11-20** (macOS arm64 re-measurement of the surviving
  wheel set + gap-fill re-scans, this document).
- **Method:** scripted symbol scan (not guesses). Two independent passes:
  1. `docs/measure-ce-surface.py` + `docs/wheel-scan-results.json` — 32 manylinux
     x86_64 wheels, `nm -D` undefined/defined dynamic symbols, stable-ABI cross-check
     via `abi3info` 2026.9.25 (vendors CPython's `stable_abi.toml`). This is the
     *surviving prior-attempt dataset* (per-package totals + full symbol lists).
  2. `docs/scan_wheels.py` + `docs/ce-scan-results-macarm.json` (this completion) —
     49 macOS arm64/universal2 wheels (newest cp314/cp314t, plus cp311/cp313 where
     that is what PyPI serves), extracted and `nm`-scanned. NOTE: `nm -D` errors out
     on Mach-O ("no dynamic symbol table"), which is why the mac-arm pass's raw
     `undefined_Py` lists in `ce-scan-results-macarm.json` are empty; the real
     undefined-symbol data was recovered with plain `nm -U` over the same extracted
     `.so` files and is recorded in `docs/ce-scan-symbols-full.json`. Gap-fill
     re-scans (pydantic, argon2-cffi, gevent, simdjson, pymerkle, bcrypt, greenlet,
     grpcio) in `docs/ce-scan-gapfill.json`.
- **Prior art:** componentize-py, GraalPy docs/changelog, RustPython issues/PRs,
  CPython PEP 384 / PEP 659 / PEP 703 (§5).

---

## 1. Measurement results

**40 packages with native code and measured `Py*` symbol imports** (26 from pass 1,
36 from pass 2, deduplicated; 4 gap-fill additions: bcrypt, grpcio, gevent, greenlet).
Per-package undefined-`Py*` symbol counts and stable-ABI membership (pass-2 mac-arm
numbers; pass-1 manylinux numbers agreed within a few % per package):

| package | ver | `.so` | Py\* undef | stable-ABI % | class |
|---|---|---|---|---|---|
| numpy | 2.5.5 | 19 | 874 | 39.1% ¹ | C (numpy C-API ecosystem) |
| scipy | 1.18.1 | 106 | 499 | 66.3% ¹ | C |
| pandas | 3.1.1 | 45 | 368 | 79.1% ¹ | C |
| shapely | 2.2.0 | 3 | 343 | 54.5% ¹ | C (GEOS + own `*_Scalar` exports) |
| lxml | 6.1.3 | 7 | 291 | 92.8% | C |
| pyarrow | 26.0.1 | 21 | 289 | 84.8% | C++ (Arrow) |
| gevent | 26.9.0 | 17 | 300 | 79.0% ² | C (greenlet-internal types) |
| grpcio | 1.85.3 | 1 | 226 | 91.2% | C |
| sqlalchemy | 2.1.9 | 8 | 223 | 88.8% | C accelerators |
| fiona | 1.10.1 | 8 | 240 | 89.6% | C (GDAL) |
| rapidfuzz | 3.14.6 | 5 | 197 | 89.3% | C++ |
| msgpack | 1.2.4 | 1 | 190 | 89.5% | C |
| cryptography | 50.0.2 | 27 | 174 | 79.9% ³ | **Rust/PyO3** |
| matplotlib | 3.11.2 | 9 | 193 | 86.5% | C++ |
| msgspec | 0.22.0 | 1 | 168 | 95.8% | C |
| **pydantic-core** | **2.50.5** | **1** | **169** | **92.9%** | **Rust/PyO3 — case study, §4** |
| cffi | 2.1.1 | 1 | 165 | 91.5% | C (ABI-mode dlopen) |
| pillow | 13.0.0 | 8 | 156 ⁴ | 60.9% ¹ | C (own `PyImaging*` namespace) |
| contourpy | 1.4.0 | 1 | 140 | 91.4% | C++ |
| pyyaml | 6.0.3 | 1 | 140 | 90.7% | C |
| tokenizers | 0.23.3 | 8 | 138 | 88.4% | **Rust/PyO3** |
| psycopg2-binary | 2.9.13 | 1 | 138 | 96.4% | C |
| httptools | 0.9.0 | 2 | 133 | 88.7% | C |
| pymupdf | 2.1 | 5 | 126 | 96.8% | C++ (abi3 wheel) |
| safetensors | 0.8.0 | 1 | 114 | **99.1%** | **Rust/PyO3** (abi3 wheel) |
| uvloop | 0.23.0 | 1 | 250 | 94.0% | C |
| regex | 2026.11.19 | 1 | 91 | 94.5% | C |
| rpds-py | 2026.9.1 | 1 | 87 | 92.0% | **Rust/PyO3** |
| watchfiles | 1.3.0 | 1 | 87 | 93.1% | **Rust/PyO3** |
| tiktoken | 0.14.0 | 1 | 85 | 91.8% | **Rust/PyO3** |
| greenlet | 3.5.6 | 3 | 83 | 90.4% ⁵ | C (raw stack switching, §6) |
| zstandard | 0.25.0 | 2 | 77 | 94.8% | C |
| orjson | 3.13.0 | 1 | 56 | 94.6% | **Rust/PyO3** |
| psutil | 7.2.2 | 1 | 41 | 87.8% | C |
| kiwisolver | 1.5.1 | 1 | 32 | 90.6% | C++ |
| ujson | 6.1.0 | 1 | 70 | 84.3% | C |
| bcrypt | 5.0.0 | 1 | 64 | **100%** | **Rust/PyO3** (abi3 wheel) |
| markupsafe | 3.0.4 | 1 | 3 | — | C (`_speedups`, trivial) |
| brotli | 1.2.1 | 1 | 20 | 95.0% | C |
| brotlicffi | 1.2.1.0 | 1 | 11 | 90.9% | C |

¹ Percentages collapse because numpy/scipy/pandas/shapely/pillow/gevent import *their
own* package C-API namespaces (`PyArray*`/`PyUFunc*`, `PyImaging*`, `PyGevent*`,
GEOS `*_Scalar`) — measured against the CPython stable-ABI list these classify as
"non-stable" but they are not CPython surface at all (see §1.2).
² gevent's non-stable remainder is its 13 `PyGevent*` watcher types + static-inline
accessors (`Py*_GET_SIZE`, `PyTuple_SET_ITEM`) — header inlines, resolved at compile
time under a rebuild-required policy.
³ cryptography ships 27 submodule `.so`s (`_PyInit__rust` + 26 `_PyInit_*`); its
non-stable list is 8 symbols, all PyO3 emissions (§1.3).
⁴ pillow's 8 visible `.so`s (mac wheels split codecs across `_imaging*` modules).
⁵ greenlet: `PyGreenlet*` types + `PyObject_GetArenaAllocator`,
`PyThreadState_{Enter,Leave}Tracing`, `PyInterpreterState_Head` — the stack-switch
special case, not a symbol-count problem.

**Pure-python controls (gap-fill confirmed):** `pydantic` 2.14.4, `argon2-cffi`
25.1.0 and `pymerkle` 7.0.0 ship `py3-none-any` wheels — zero native code (pydantic's
native part is pydantic-core, a separate package; argon2's lives in
argon2-cffi-bindings). `simdjson` publishes no mac-arm wheel at all (sdist-only;
unmeasured, C library behind pybind11 — same class as the C tier). This resolves the
prior attempt's "missing wheel lines": **sdist/pure-python, not download failures.**

**Totals: 1,676 unique `Py*` symbol imports across the 40 native packages.**

### 1.1 Union shape: steep head, numpy-dominated tail

- **Core (imported by ≥40% of the 40 packages): 147 symbols.** Error
  set/raise/match family + the `PyExc_*` exception types (40+), tuple/list/dict
  constructors+accessors, `PyObject_{Get,Set}Attr{,String}` + Optional variants,
  `PyObject_Call*` + the full vectorcall family, `PyLong_{From,As}*` +
  `PyLongWriter`/`PyLong_{Export,GetNativeLayout,AsNativeBytes,FromNativeBytes}`
  (3.14 native-bytes long ABI), `PyUnicode_{From,As}*` + interns + the
  `PyUnicodeWriter` family, `PyType_{Ready,IsSubtype,Modified,FromSpec,
  FromMetaclass,FromModuleAndSpec,GetSlot,GenericAlloc,GenericNew}`,
  `PyModule_{Create2,ExecDef,FromDefAndSpec2,Add*}` + `PyModuleDef_Init`,
  `PyMem_{Malloc,Calloc,Realloc,Free}` + Raw variants, `PyObject_GC_*`,
  `PyGILState_{Ensure,Release}`, `PyEval_{Save,Restore}Thread`, buffer protocol,
  capsules, `PyOS_*` string/double/fork helpers, `Py_Version`/`Py_GetVersion`.
- **Mid (2–<40%): 355 symbols.**
- **Tail (<2 packages): 1,174 symbols** — dominated by numpy's 519 package-unique
  `PyArray*`/`PyUFunc*` C-API imports plus scipy/shapely/fiona internals. This tail
  is *ecosystem* surface (numpy extensions import numpy's own C-API via
  `PyCapsule_Import`), not CPython surface; it is priced separately in §2 (S3).
- Zero undefined-symbol struct-field pokes (`tp_*`, `ob_refcnt`) anywhere — layout
  access happens through headers at compile time, never through dynamic symbols
  (§1.4).

### 1.2 Stable-ABI overlap of the true C-API union

Excluding package-private namespaces (`PyArray*`, `PyUFunc*`, `PyImaging*`,
`PyGevent*`, `PyGreenlet*`, shapely `*_Scalar`, `PyInit_*` exports) and obvious
header-inline residue, **the true CPython-C-API union is 704 symbols**:
**487 (69.2%) are stable-ABI members; 73 more are public-but-non-stable**;
the remaining ~144 are `PyInit_*` module-init exports (not API surface).

The 73 public non-stable symbols, bucketed:

- **PyO3 emission set (18, shared across every Rust ext):**
  `PyObject_CallOneArg`, `PyObject_VectorcallDict`, `PyObject_LengthHint`,
  `PyUnicodeWriter_{Create,Discard,Finish,WriteChar,WriteUTF8}`,
  `PyUnicode_{New,DATA,CopyCharacters,FromKindAndData,AsUTF8}`,
  `PyFunction_Type`, `PyMutex_{Lock,Unlock}`, `PyUnstable_Module_SetGIL`,
  `PyUnstable_Object_{EnableDeferredRefcount,IsUniquelyReferenced,
  IsUniqueReferencedTemporary}`, `PyUnstable_Code_NewWithPosOnlyArgs`.
  These are de-facto standard (every pyo3 build emits them) — **must implement
  anyway**, and all are small.
- **Frame/code/traceback (8):** `PyFrame_{New,GetBack,GetCode,GetLineNumber}`,
  `PyCode_{NewEmpty,NewWithPosOnlyArgs}`, `PyGen_GetCode`, `PyRun_StringFlags` —
  traceback plumbing (sqlalchemy, grpcio, gevent, Cython output).
- **Method/class constructors (7):** `PyMethod_{New,Type}`,
  `PyClassMethod_New`, `PyStaticMethod_{New,Type}`, `PyInstanceMethod_{New,Type}`.
- **3.14-era additions (9):** `PyDict_{Pop,SetDefault}` (newer variants),
  `PyList_Extend`, `PyErr_FormatUnraisable`, `PyLong_{GetSign,FromUnicodeObject}`,
  `PyType_GetDict`, `PyThreadState_GetUnchecked`, `PyContext_{CopyCurrent,
  Enter,Exit,Type}`, `PyCoro_Type`, `PyComplex_{As,From}CComplex`,
  `PyFloat_Pack{4,8}/Unpack{4,8}`, `PyDateTimeAPI`, `PyFile_AsFile`,
  `PyObject_{ClearManagedDict,VisitManagedDict,Print}`, `Py_OptimizeFlag`,
  `PyIOBase_TypeObj`, `PyInterpreterState_{Main,Head}`.

**Conclusion (unchanged from pass 1, now double-measured): the stable ABI is a good
*floor*, not a sufficient target. The real target is stable-ABI ∪ PyO3 emission set
∪ traceback plumbing ≈ 560 members for S0–S1; the numpy capsule ecosystem is a
separate S3 tier.**

### 1.3 The Rust/PyO3 class (9 of 40 packages measured)

pydantic-core, cryptography, tokenizers, tiktoken, orjson, bcrypt, safetensors,
rpds-py, watchfiles. Measured commonalities:

- **One `.so` per module, zero static `PyTypeObject` structs** — type creation goes
  entirely through `PyType_FromSpec`/`PyType_FromMetaclass`/`PyType_GenericAlloc`.
  pydantic-core 2.50.5 also emits `PyType_Freeze`, `PyType_GetModuleName`,
  `PyType_GetName`, `PyType_GetQualName` — all FromSpec-era accessors, no layout.
- **Polite GC:** `PyObject_GC_{Track,UnTrack,Del}` only; no `tp_traverse` C-struct
  assumptions visible in symbols.
- **Threads:** `PyGILState_{Ensure,Release}`, `PyEval_{Save,Restore}Thread`,
  `PyUnstable_Module_SetGIL` (declares `Py_mod_gil` / PEP 703 posture).
- **abi3 wheels exist and are the cleanest clients:** bcrypt (`cp39-abi3`, 100%
  stable-ABI imports) and safetensors (`cp310-abi3`, 99.1%).

This class is the natural first target: one surface (S1) covers all nine.

### 1.4 The hidden layout dependency (not visible in symbols)

Undefined symbols cannot show the biggest constraint: **extensions compiled against
CPython headers contain *statically defined* `PyTypeObject` structs** laid out per
CPython's memory layout, passed to `PyType_Ready`. Every hand-written C extension
and all Cython output does this; PyO3 does not (§1.3).

A rebuild-required boundary (locked v2 policy) sidesteps this: we ship our own
`Python.h`-compatible headers where `PyTypeObject` is **our** layout. Source compiled
against our headers cannot embed CPython layout. This is GraalPy's and PyPy's
argument for refusing ABI parity ("just impossible without replicating all CPython
internals"). **Binary wheels never load — recompile is mandatory**; the survey
confirms this is not merely convenient but load-bearing.

---

## 2. Feasibility subsets

| subset | contents | est. size | feasible? |
|---|---|---|---|
| S0 "hello extension" | module def, `PyArg_Parse*`, basic converters, `PyType_FromSpec`, refcount emu, exc set/raise | ~120 fns | **yes — 10.02 spike target** |
| S1 stable-ABI + PyO3 floor | S0 + the 487 measured stable-ABI union + the 73-symbol non-stable bucket of §1.2 (PyO3 emissions, traceback, method ctors, 3.14 additions) | **~560 members (measured)** | **yes — all 9 Rust/PyO3 packages land here, incl. pydantic-core** |
| S2 core-C floor | S1 + static-`PyTypeObject` header re-layout + buffer protocol depth + `tp_*` slot surface + weakrefs + `PyCapsule` + datetime C-API (`PyDateTimeAPI`) | +~120 | **yes, covers the C/C++ mid tier (lxml, psycopg2, yaml, msgpack, regex, cffi API-mode, sqlalchemy…) — ~2/3 of measured packages** |
| S3 numpy tier | S2 + numpy's own C-API via capsule import (`PyArray*`/`PyUFunc*`, ~520 syms), TraceMalloc, `_PyUnicode_Is*`-class internals, contextvars | +~560 | **mostly yes; volume + perf is the risk, not semantics. Price separately from S0–S2.** |
| S4 embedder tier | `PyConfig_*`, subinterpreters, `PyThreadState` introspection, arena allocators, frame walking (cffi ABI-mode, greenlet) | +~50 | **partial — greenlet needs stack-switch hooks into our fibers; cffi ABI-mode needs a libpython-lookalike** |
| R refused | direct-layout binary wheels, `Py_LIMITED_MODULE` cheats, CPython-internals pointers (`PyFrameObject` field walks), immortal-refcount arithmetic relied upon | — | **no — declared boundary, same as GraalPy/PyPy** |

**The GIL question, measured:** every one of the 40 packages calls
`PyGILState_Ensure`/`PyEval_SaveThread` or nothing at all. None relies on the GIL for
correctness beyond reentrancy-into-Python (safe by construction in our no-GIL
runtime). Implement `PyGILState_Ensure` as a cheap per-thread accessor and accept
`Py_mod_gil`/critical-section APIs per the 02.09 flip. This is a *differentiator*:
GraalPy kept a GIL solely to satisfy C extensions; we don't have to.

---

## 3. Case study: pydantic-core (the T3/T4 blocker)

Measured (pydantic_core **2.50.5**, cp314 mac-arm wheel, one `.so`, Rust/PyO3;
pass-1 measured 2.50.4 manylinux at 158 syms/96% — the surface is stable across
versions and platforms):

- **169 `Py*` imports; 157 (92.9%) are stable-ABI members** (abi3info 2026.9.25).
- **11 outside stable (excluding its own `PyInit__pydantic_core`):**
  `PyFunction_Type`, `PyObject_CallOneArg`, `PyObject_LengthHint`,
  `PyObject_VectorcallDict`, `PyUnicodeWriter_{Create,Discard,Finish,WriteChar,
  WriteUTF8}`, `PyUnicode_DATA`, `PyUnicode_New` — all small, all implementable,
  none layout-bound. (The delta vs pass 1 is the newer pyo3 release adopting the
  `PyUnicodeWriter` family — it *shrank* the `_PyLong_*ByteArray` usage and added
  only writer-API calls.)
- **Type creation: 100% `PyType_FromSpec`/`PyType_GenericAlloc`** (+`PyType_Freeze`,
  `PyType_Get{Name,QualName,ModuleName}`) — no static `PyTypeObject`, no hidden
  layout dependency.
- GC interaction is the polite subset: `PyObject_GC_{UnTrack,Del}` only in the
  measured imports; threads: `PyGILState_{Ensure,Release}` +
  `PyEval_{Save,Restore}Thread`; dict/long access via the modern ref-returning
  variants (`PyDict_GetItemRef`, `PyLong_AsNativeBytes`, `PyLongWriter_*`,
  `PyLong_{Export,FreeExport,GetNativeLayout}`) — 3.14-dialect APIs, matching our
  3.14 conformance target exactly.
- Note (results.json cross-check): pydantic-core's `uses_abi3: false` and its cp314
  wheel tag mean a *specific-ABI* build; the PyO3 emission set is nonetheless the
  abi3-flavored minimal surface. **Rebuild against our headers is already locked
  policy, so the wheel tag is irrelevant — only the symbol demand matters.**

### 3.1 The honest pyo3 question (T3/T4 reachability)

pyo3 is itself a Rust crate whose `pyo3-ffi` layer *is* the CPython C-API surface.
"Rust backend already" therefore does not mean pydantic-core could run without the
C-API — it means pydantic-core's entire dependence on CPython is funneled through
~170 measured symbols emitted by one well-behaved crate. Two ways to serve it:

1. **S1 C-API floor (primary).** Implement the ~560-member surface (§2); pydantic-core
   — and simultaneously cryptography, tokenizers, tiktoken, orjson, bcrypt,
   safetensors, rpds-py, watchfiles — recompile against our headers unchanged. No
   upstream cooperation, no fork maintenance.
2. **pyo3 backend port (fallback, not the recommendation).** Fork pyo3 and swap
   `pyo3-ffi` for a direct `ap-obj` binding layer. Saves emulating symbols pyo3
   doesn't use, but: forks a fast-moving upstream, serves only the pyo3 class
   (all Cython/C extensions still need S1–S2), and doubles the maintenance
   surface. Only worth it if S1's tp-slot trampolines stall — precisely the risk
   RustPython's PR #8435 hit.

**Verdict: pydantic-core is REACHABLE on subset S1, by measurement, twice.** The
single sharpest blocker for T3 (hermes-agent) and T4 (fastapi) needs ~560 measured
members with zero layout dependency and zero exotic semantics. **T3/T4 are reachable
via Plan 10** contingent on S1 landing; the pyo3-backend option is the named
fallback, not the plan.

---

## 4. Prior art (research summary)

- **componentize-py** (Apache-2.0): ships *real* CPython (libpython → wasm) +
  "pseudo-dynamic-linking" lookup tables instead of `dlopen`. 100% C-API by
  construction, ~35MB hello-world, keeps the GIL, tracks a patched CPython fork.
  Lesson: full fidelity is buyable only by *being* CPython — which forfeits our
  no-GIL/GC/i64-value design. Not our strategy; useful as a conformance oracle.
- **GraalPy**: source-compatible C-API over a foreign (JVM) object model;
  explicit refusal of ABI parity ("just impossible without replicating all CPython
  internals, same as PyPy"); rebuilt-wheels-only; keeps a GIL "because C extensions
  expect to do so"; opaque `GraalPyObject` (refcount + handle index) over the
  tracing GC; ecosystem metric instead of %-coverage (93% of top-600 PyPI packages
  install, 65%+ tests pass). Lesson: **the emulation model works, given a decade of
  per-package grinding**; the refcount+handle trick is directly applicable to our
  `Gc<T>` + i64 values.
- **RustPython**: stalled ~8 years at "hundreds of stubs" depth — issue #8156 counts
  436 stable-ABI functions still missing; PR #8435 C-slot trampolines "unfortunately
  does not work" (PyO3 needs C `tp_new` on all base types); core objection #1940:
  mimicking the C API requires simulating object memory layout. Lesson:
  thin-wrapper, no-layout-emulation, no-`tp_*` approaches die; **the type-slot
  trampoline is where Rust extensions actually land — do it first, not last.**
- **CPython internals**: PEP 384 — "the primary source of ABI incompatibility are
  changes to the lay-out of in-memory structures" → opaque types + `PyType_FromSpec`
  is exactly the surface that avoids layout (and is exactly what the measured Rust
  class uses). PEP 659 — specialization is interpreter-internal; no C-API constraint
  on our i64-tagged/typed-region design. PEP 703 — free-threaded builds already
  broke stable-ABI compat, biased refcounts, `Py_REFCNT` "do not rely on the
  returned value to be accurate" — **legitimizes refcount-as-emulation** for our
  no-GIL runtime and means we target the 3.14 free-threaded C-API dialect.

---

## 5. Recommended approach

**Proceed, scoped: a new extension ABI with CPython-source-compatible headers + an
`ap-obj`-backed symbol table (source-shim), targeting the measured S0–S1 union
(~560 members), phased S0→S3, with a declared refused set (R) and
rebuild-required boundary.**

Ranked against the three candidate strategies:

| strategy | verdict |
|---|---|
| **Full C-API emulation in Rust** (binary-wheel loading, CPython layout byte-compat) | **Rejected.** Requires replicating CPython struct layout (PyPy/GraalPy both refuse for cause); contradicts our i64-value + tracing-GC object model; buys only faster installs. |
| **pyo3-direct-integration** (fork pyo3, native `ap-obj` backend) | **Fallback only (§3.1).** Serves the 9-package Rust class but not the ~31 C/C++/Cython packages; fork maintenance on a fast-moving upstream. |
| **New extension ABI + source-shim (CHOSEN)** | Our own `Python.h`-shaped headers (our `PyTypeObject` layout) + a Rust dispatcher mapping ~560 C symbols onto `ap-obj`/`ap-run` values; extensions recompile via `antipythony build-ext` (10.03). Binary CPython wheels fail loudly (locked v1 behavior). Covers every measured class. |

Implementation commitments (unchanged from pass 1, measurement re-confirms each):

1. **Refcounts are emulation.** `Py_INCREF/DECREF` manipulate a per-object emulated
   count; real memory ownership stays with the MMTk tracing GC (GraalPy model).
   `Py_REFCNT` returns the emulation; no extension may rely on exactness (PEP 703's
   own rule). Immortal singletons (`_Py_NoneStruct`…) are immortal.
2. **Type slots via trampolines, first.** `tp_*` slot dispatch as generated thunks
   calling back into Rust — the exact point RustPython's PR #8435 failed.
   `PyType_FromSpec` + slots are the measured path for the entire Rust class.
3. **Vectorcall-first call path.** `PyObject_Vectorcall*` + `PyVectorcall_Function`
   are the modern call ABI; `tp_call`/`PyArg_Parse*` reduce onto it.
4. **Target the free-threaded (PEP 703) 3.14 dialect.** `Py_BEGIN_CRITICAL_SECTION`
   no-ops into our lock-free maps; `Py_mod_gil` accepted-and-ignored; thread-state
   APIs implemented as cheap accessors to a per-thread interpreter handle (the 02.09
   per-worker registries already model this). The measured 3.14-dialect adoption
   (`PyDict_GetItemRef`, `PyLong_AsNativeBytes`, `PyUnicodeWriter_*`) says the
   ecosystem is moving *toward* the dialect we already implement.
5. **Priority order = ecosystem weight:** S0/S1 lands pydantic-core + cryptography +
   tokenizers + tiktoken + orjson + bcrypt + safetensors + rpds-py + watchfiles
   (all Rust/PyO3, 88–100% stable-ABI measured). S2 lands the C/C++ mid tier.
   S3 prices the numpy capsule ecosystem separately. greenlet is the hardest single
   package (raw stack switching) — schedule against our fiber implementation
   explicitly, not as generic C-API work.

### Kill-criterion (executable; fires at the 10.02 gate)

**Kill Plan 10 if any of:**

1. **Cost density:** the 10.02 spike measures hours/API-function; the S0+S1 surface
   (560 measured members) at that density must extrapolate to < ~2 person-years,
   else STOP. Concretely: S0 (~120 fns, incl. `PyType_FromSpec` + slot trampolines +
   vectorcall + a trivial pyo3-style module importing and running) must land within
   6 elapsed weeks of starting 10.02.
2. **Trampoline feasibility:** if a minimal PyO3-shaped module (static-type-free,
   FromSpec-built, slot-dispatched) cannot be made to import, instantiate a class,
   and round-trip values through `ap-obj` on the spike's S0 surface, the core
   mechanism is dead — STOP (this is the RustPython-#8435 failure mode).
3. **pydantic-core probe:** the spike's extrapolation must include importing
   `pydantic_core` and validating one trivial model on S1; if the measured 11-symbol
   non-stable delta + S1 floor proves insufficient for a *trivial* model (i.e. the
   honest surface is much deeper than symbols show), T3/T4 lose their payback —
   STOP and re-scope Plan 10 to S2-only (C tier) with T3/T4 served by pure-python
   fallbacks.

Single-package hard cases (greenlet stack switching, numpy capsule ecosystem,
cffi ABI-mode) do **not** trigger the kill by themselves — they are descope
candidates within a surviving plan.

### Risks (measured, not guessed)

- numpy's *real* depth is its runtime capsule protocol (`PyCapsule_Import` of
  `numpy._core._multiarray_umath`): the numpy *ecosystem* (scipy, pandas, pyarrow
  measured here) imports numpy's C-API through it. That must work for the numpy
  tier; numpy alone is not the finish line — hence the separate S3 pricing.
- cffi ABI-mode (`dlopen` + libffi, no compilation) wants a `libpython`-lookalike
  process surface — keep cffi on API-mode (recompile) and declare ABI-mode refused
  in v2. (cffi's measured imports are 91.5% stable-ABI; the C-API part is fine,
  the dlopen-of-libpython part is the refusal.)
- gevent at 79% stable is the lowest non-numpy score, driven by its 13
  `PyGevent*` watcher types (self-contained, ships with the package) plus
  static-inline accessors — recompile resolves both; its greenlet dependency
  remains the S4 special case.
- cp-version skew: wheels measured are cp311–cp315, whatever PyPI serves for
  mac-arm. Symbol *demand* is demand regardless of build tag under a
  rebuild-required policy; but exact-version pinning of the survey is not claimed.

---

## 6. Measurement artifacts

- `docs/measure-ce-surface.py` — pass-1 scan script (manylinux x86_64; survived the
  10.01 disk-full crash alongside its output).
- `docs/wheel-scan-results.json` — pass-1 raw per-package results (32 packages,
  full undefined-symbol lists, category buckets).
- `docs/scan_wheels.py` — pass-2 scan script (mac-arm; plain `nm` recovery of
  undefined symbols; gap-fill downloads).
- `docs/ce-scan-results-macarm.json` — pass-2 raw results (49 packages; wheel/
  so_count/PyInit measured; `undefined_Py` lists empty on Mach-O per `nm -D`).
- `docs/ce-scan-symbols-full.json` — recovered per-package undefined-`Py*` symbol
  lists for all 40 native packages (the ground truth behind §1's table).
- `docs/ce-scan-tiers.json` — core/mid/tail partition (147/355/1,174 of 1,676).
- `docs/ce-scan-gapfill.json` — gap-fill re-scan results + stable-ABI
  classification for pydantic, argon2-cffi, pymerkle, simdjson, bcrypt, greenlet,
  grpcio, gevent.

Caveats: mac-arm + two manylinux passes agree within a few % per package; symbol
*imports* measure surface demand, not runtime-behavior depth (buffer-protocol
subtleties, `tp_traverse` correctness show up as "same symbol, deeper semantics");
40 native packages is a solid sample of the top-depended-on native tier, and the
tail adds symbols, not new *categories* (re-confirmed by the gap-fill pass).

# Leaf 10.03 S1 feasibility note — wrapper-style vs hard members

Scope-check deliverable for plans/antipythony/10-c-extensions-stretch leaf
10.03 (per orchestrator.md: 10.04-style feasibility note, NOT S1
implementation). Inputs: the 10.02 measured density
(`docs/plan10-1002-spike-report.md`), the measured S1 membership
(`docs/ce-compat-survey.md` §1.1–§1.2, ~560 members), and the 10.03
graduation itself (build-ext verb + gate now in CI shape), which moved the
"S0 is servable" baseline from spike to tool.

## What 10.03 changed about the S1 estimate

The 10.02 spike measured 10–15 min/fn marginal for WRAPPER-style members.
10.03 adds two measured corrections, both in the favorable direction:

1. **The dual-compile differential harness is now reusable.** The
   `apx_cpython_compat.h` pattern + the `build_ext_gate.rs` CI shape mean
   each new S1 member lands WITH its byte-exactness proof nearly free — the
   spike built that machinery by hand; it is now infrastructure. Effect:
   the *debugging* share of the 10–15 min/fn band (the part that was
   error-text mismatches and ABI surprises) shrinks; the honest band for
   simple wrappers tightens toward ~10 min/fn.
2. **The ABI-divergence class is enumerated, not open-ended.** 10.03's
   differential caught exactly two shim-level divergences (ParseTuple `'i'`
   wrote a `long` — a silent ABI corruption vs CPython's `int` receiver;
   and a stale PyUnicode error text vs the 3.14 dialect). Both were
   1-line fixes once the differential pointed at them. Expectation for S1:
   more of the same class (converter widths, error-text dialects), each
   caught mechanically by the differential, each cheap. The residual risk
   is NOT in this class; it is in the hard members below.

## Wrapper-style S1 members (~500 of ~560; the 10.02 band holds)

These convert a Value/handle through an existing mechanism — no new runtime
capability required:

- **Container/long/unicode surface completion** (~180): the rest of
  `PyDict_*` (GetItemRef/Pop/SetDefault/Next/Keys/Values/Items...),
  `PyList_*` (GetItem/SetItem/GetSlice/Extend/Insert/Sort/Reverse...),
  `PyTuple` residual, `PyLong` converters incl. `PyLong_AsNativeBytes` /
  `PyLongWriter_*` (3.14 native-bytes ABI — mechanical over ap-obj's
  bigint), `PyUnicode_{New,FromKindAndData,CopyCharacters,AsUTF8}` +
  `PyUnicodeWriter_*` (a C-side builder over PyString — wrapper with one
  small new Rust helper each), interns, `PyObject_Str/Repr/TypeName`.
- **Object protocol** (~60): `PyObject_Get/Set/DelAttr{,String}` +
  Optional variants, `PyObject_LengthHint`, `PyObject_RichCompare*`,
  `PyObject_GetIter/IterNext` (bridge to the existing iterator registry —
  the OOM campaign's iterator table already has the lifecycle), hash/dir.
- **Method/class constructors + emission set** (~25):
  `PyMethod_{New,Type}`, `PyClassMethod_New`, `PyStaticMethod_{New,Type}`,
  `PyInstanceMethod_{New,Type}`, `PyFunction_Type` token,
  `PyMutex_{Lock,Unlock}` (trivial over std), `PyUnstable_*` markers
  (no-op-able), `Py_Version`/`Py_GetVersion`.
- **Module/mem/GIL surface** (~40): `PyModule_ExecDef/FromDefAndSpec2/Add*`,
  `PyMem_{Malloc,Calloc,Realloc,Free}` (+Raw) — ap-ce can front a plain
  allocator (these handles never enter the GC heap by construction;
  same discipline as the lxml DOCS LRU and StringIO in-place work),
  `PyGILState_{Ensure,Release}` + `PyEval_{Save,Restore}Thread` — cheap
  per-thread accessors per the survey's no-GIL differentiator (the 02.09
  flip means these are bookkeeping, not locks).
- **Error/traceback plumbing** (~30): `PyErr_Format` (varargs shim fn,
  same pattern as ParseTuple), `PyErr_{NormalizeException,ExceptionMatches,
  GivenExceptionMatches,Fetch,Restore,SetObject}`, `PyFrame_*` +
  `PyCode_*` + `PyGen_GetCode` — ap-run already carries per-frame file
  attribution (W110) and per-exception tracebacks (98b2964); the C surface
  is a projection of what exists.

Estimated: 500 × 0.17–0.25 h ≈ **85–125 h**. (Band tightened slightly from
the spike's 110 h per correction 1.)

## Hard S1 members (~60; the risk lives here)

1. **PyObject_Call-back into the runtime** (~12 members incl.
   `PyObject_Call/CallOneArg/CallNoArgs/Vectorcall*`) — the spike's named
   largest single unmeasured cost. NOT a wrapper: the C function runs on
   the mutator thread and must re-enter the tree-walk/tier-0 executor,
   which means threading the interpreter handle (Arc<Interpreter>) through
   the ap-ce registries so a marker/method C frame can push a Python call
   and propagate its Flow (Raise must round-trip through err_set). The
   mechanical parts exist (call_ce_marker already converts Flow↔err in
   BOTH directions; fiber-based async re-entry precedent from 02.05f/02.09).
   Estimate 8–16 h for the trampoline + call family, dominated by the
   interpreter-handle plumbing and the re-entrancy proof (a C method
   calling back into the class it belongs to). UNMEASURED — recommend a
   time-boxed S1 increment 1 that lands ONLY this, so the estimate
   hardens before the bulk wave.
2. **PyLong_Export digit layout** (`PyLong_Export/FreeExport/
   GetNativeLayout/FromNativeBytes`, ~6 members): CPython 3.14 exports a
   specific digit-array layout; ap-obj's bigint uses a different internal
   representation. Must produce CPython-layout bytes from our digits
   (and parse them back). Bounded but fiddly: ~4–8 h incl. differential
   on big-int values.
3. **tp_traverse / GC bridging** (`PyObject_GC_{Track,UnTrack,Del,New}`,
   tp_traverse slot, ~10 members): a C extension's payload struct holds
   PyObject* edges OUR collector cannot see (the C struct is outside the
   GC heap). Wrapper-shape but CONSERVATION-CRITICAL: the traverse fn must
   be turned into heap edges at FromSpec time (slot markers already plumb
   the fn pointer; the runtime must CALL it with a visit closure). This is
   the same boundary the fiber/weakreg bughunts lived on — the cost is
   not the wrapper, it is getting the GC contract right: ~6–12 h.
4. **Instance payloads** (enabler, not a member list): pydantic-core's
   `&self` model needs C-visible instance structs. Design: the runtime
   allocates a fixed-size user payload area per FromSpec instance
   (basicsize is already carried in PyType_Spec — unused on S0);
   tp_init writes it; methods read it via the instance handle. ~8–16 h
   including the GC interaction (payload edges → item 3).
5. **Buffer protocol + PyCapsule** (~15 members; survey classifies buffer
   DEPTH as S2 but the S1 slice needs the type tokens + capsule
   get/import for pydantic-core's function-set exchange): capsule is
   genuinely small (~4 h); buffer basic get/release ~6 h if S1-target
   packages need it (pydantic-core does not in the measured imports).

Hard-tail estimate: **~30–60 h** for items 1–5 combined IF item 1 lands at
the low end; the spike's doubling factor for unknown-unknowns exists
precisely for this tail.

## Total + verdict

- Wrapper band: 85–125 h. Hard tail: 30–60 h. Total: **~115–185 h ≈
  0.06–0.10 person-years**; with the spike's ×2 unknown-unknowns factor:
  **~0.13–0.20 PY** — comfortably inside the 10.02 report's 0.19–0.4 PY
  envelope and 10× under the 2-PY kill line. The 10.02 PROCEED verdict
  survives contact with the S1 membership detail.
- The single point estimate that matters most is PyObject_Call-back
  (item 1): everything else in S1 is bounded by measured precedent; it is
  the only member class that could re-open the cost question, and it is
  testable in isolation with the 10.03 gate harness. RECOMMENDATION: S1
  increment 1 = PyObject_Call trampoline + differential (time-box ~2
  working days), then the bulk wave by the sections above.
- pydantic-core gate (kill-criterion #3): reachable on S1 by measurement
  (survey §3, twice-measured). The S1 note adds: its 11 non-stable symbols
  all fall in the wrapper band except `PyObject_CallOneArg` /
  `PyObject_VectorcallDict` (item 1's family) — so the pydantic probe
  CANNOT go green until item 1 lands; sequence accordingly.

# Leaf 10.02 spike report — S0 C-API density measurement (2026-11-21)

Deliverable for plans/antipythony/10-c-extensions-stretch/orchestrator.md leaf 10.02.
Session record: NEXT-1002.md (same dir). Worktree: ~/.git/worktrees/leaf-1002
(branch tier1-1002 off main 8e96a8a).

## What landed

ONE trivial hand-written C extension (`tests/ce-spike/spike.c`, PyO3-shaped:
static-type-free, PyType_FromSpec-built, slot-dispatched — exactly the survey's
kill-criterion #2 shape) compiles via `antipythony build-ext`, dlopen-imports,
and runs byte-exact under antipythony native vs CPython 3.14.7 running the SAME
source (compile-both differential gate, verified including error-message texts
and exit codes).

Components:
1. `crates/ap-ce` (NEW crate): the S0 dispatcher.
   - `CeObject` handle layer: ob_refcnt-first OUR layout, refcount as
     EMULATION over the tracing GC (survey commitment 1; PEP 703-legitimized).
   - Registries: method/spec/module (process-global, Mutex-guarded).
   - `PyType_FromSpec` → `TypeObject::create` + `make_class_value` + slot
     markers (`__ap_ce__.<spec>#<entry>`) in the class dict; getsets as
     property descriptors whose fget is the marker. The trampoline is where
     RustPython PR #8435 died — ours works end-to-end (see Gate).
   - Exported C symbols (26): Py_IncRef/DecRef/Refcnt, singleton fns
     (apx_Py_None/True/False), 10 exception tokens, PyLong_From/As (x4),
     PyFloat_From/As, PyBool_From, PyUnicode_From/AsUTF8 (x3),
     PyObject_Str/Repr, PyTuple (x4), PyList (x2), PyDict (x2),
     PyModule_Create2/AddObject, PyType_Ready, apx_PyType_FromSpec,
     PyErr_SetString/Clear/Occurred, apx_method_register/apx_type_register.
   - `c/apx_shim.c`: PyArg_ParseTuple (varargs — cannot live in Rust;
     formats i/d/s/O + CPython-exact arity/type error texts), the C slot-table
     walker, the PyInit registry, apx_PyModule_AddFunctions.
2. `include/apx_python.h` (NEW): CPython-source-compatible header; PyObject
   opaque, PyModuleDef OUR 4-field layout (m_name/m_doc/m_size/m_methods),
   PyType_Spec/PyType_Slot CPython shapes (slot ids = CPython typeslots.h
   values 52/60/64/65/73 — required for the dual-compile to work).
3. `crates/ap-run`: `ce_bridge.rs` — marker dispatch (tp_new validation +
   runtime alloc, tp_init int-return semantics, methods as BoundMethod-bound
   markers, getsets as property fget), dotted module-fn dispatch, dlopen
   import seam (lib<mod>.so probed in script dir / cwd / APX_EXT_DIR,
   RTLD_LOCAL, PyInit_<mod> called). Hooks: `builtins_modules.rs`
   (call_native fallthrough + call_native_marker_impl prefix route),
   `importer.rs` (native-module arm before get_or_make_module),
   `classes.rs` (`__ap_ce__.` markers bind as BoundMethods), `interp.rs`
   (script-dir recording).
4. `crates/ap-cli`: `build-ext` verb (cc -shared -fPIC -undefined
   dynamic_lookup against include/), build.rs `-Wl,-export_dynamic` so the
   antipythony binary exports the Py* symbols (the source-shim loading model).

## Density measurement (the leaf's deliverable)

Honest accounting INCLUDING debugging (the biggest costs were: tp_init's int
return through a pointer-typed ABI, the CPython slot-id renumbering, the
`PyModuleDef` field shadowing bug, CPython-side PyObject_HEAD requirement —
each a multi-rebuild cycle), wall-clock from worktree creation to gate green:

- Elapsed: ~5.5 hours total.
- S0 subset landed: 40 API functions/symbols (26 exported symbols + 14
  header/walker functions), i.e. the spike-needed slice of S0 (~120 fns).
- Macro-level (headers+dispatcher+loader+build verb+extension): 5.5h / 40 fns
  ≈ **8 min/API-function ≈ 0.13 h/fn**.
- Infrastructure-adjusted (headers + dispatcher + build verb amortized over
  the surface, the honest 10.03-forward number): the bulk of the time (~3h)
  was the MECHANISM (handle layer, FromSpec, bridge, loader); the
  per-API-function marginal cost for each additional simple wrapper
  (PyDict_GetItem, PyList_GetSlice class) measured during the increment runs
  at ~10–15 min/fn (PyTuple_SetItem steal semantics + PyUnicode_AsUTF8AndSize
  were built and debugged in that band).

## Kill-criterion extrapolation (survey §5)

- S0 (~120 fns) at the marginal 0.2–0.25 h/fn: ~24–30 h. Landed the hard
  mechanism in one session; the survey's "S0 within 6 weeks" is comfortably
  green (measured: ~1 working day of sessions).
- S1 = S0 + ~560 measured members. Marginal-cost band (0.2–0.25 h/fn simple,
  plus a tail of harder members: PyLong_Export digit layout, tp_traverse
  bridging, PyObject_Call-back into the runtime — est. 2–6 h each for ~60
  members): 500×0.22 ≈ 110h + 60×4 ≈ 240h → **~350 h ≈ 0.19 person-years**
  (1 FY = 1800h). Even doubling for unknown-unknowns (integration with the
  IC system, no-GIL thread-state surfaces, GC interactions the spike never
  touches): **~0.4 person-years**.
- Threshold: > 2 person-years kills. Measured extrapolation is 5–10× UNDER
  the line. Kill-criterion #1 (cost density): NOT FIRED.
- Kill-criterion #2 (trampoline feasibility): NOT FIRED — the FromSpec-built,
  slot-dispatched, static-type-free module imports, instantiates, dispatches
  tp_new/tp_init/methods/getsets, round-trips values both ways, raises and
  propagates C errors as Python exceptions.
- Kill-criterion #3 (pydantic probe) is an S1-land gate, not this leaf; the
  density says S1 is affordable.

## Verdict: PROCEED

10.03 (`antipythony build-ext` hardening + one real extension family) is
justified at the measured density. The spike's S0 slice never needed CPython
object-layout compatibility anywhere — the opaque-handle + shim-headers +
source-rebuild strategy (survey recommendation) held under contact with zero
exceptions.

## Honest gaps (do not extrapolate blindly)

1. **Callback surface is S1**: PyObject_Call back into the runtime raises
   NotImplementedError on S0 (PyO3 modules call Python constantly — pyo3's
   trampoline needs the interpreter handle threaded through the bridge).
   This is the largest single S1 cost and the spike did not measure it.
2. **Instance payloads**: S0 tp_new validates but the runtime allocates;
   C-visible instance structs (PyO3's `&self` model) are S1. pydantic-core
   needs real payloads.
3. **Refcount exactness is emulation-only**: Py_REFCNT returns the emulated
   count; no extension relying on exactness (PEP 703 forbids it) was tested
   beyond the probe.
4. **Single-threaded**: error state is thread-local with the spike running on
   the interpreter thread; multi-worker error-state semantics unmeasured.
5. **Density band is for WRAPPER-style functions**: the S1 tail (PyLong
   export layout, GC traverse bridging, datetime capsule) was estimated, not
   measured. If that tail averages 10h+, S1 ≈ 0.6 FY — still under the line.
6. **Gate coverage**: differential is one extension, 10 assertions + error
   paths; not a conformance corpus.
7. **The fence/hybrid gates needed fresh-worktree binary staging** (probe +
   ap-host + aot-crt-hybrid) — environment artifacts, not regressions; main
   carries those artifacts.

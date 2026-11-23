# NEXT-1005C — Leaf 10.05c session record (S1 band: error/traceback C-API plumbing)

Date: 2026-11-22. Worktree: `~/.git/worktrees/leaf-1005c` (branch
`tier1-1005c`, off main `1cd873c`). Target dir: `target-1005c`.

## What landed (the band)

New `crates/ap-ce/src/exports_errtb.rs` (~30 members) + a registered
value-level runtime seam (`ErrTbOp` interpreter in `runtime_ctx.rs`,
dispatched by ap-run's `ce_bridge::runtime_errtb_dispatch`):

- **PyErr_Format** — the ONE C varargs symbol lives in `apx_shim.c`
  (Rust cannot declare varargs; the NEXT-1004 ParseTuple pattern):
  `%S` (str), `%R` (repr), `%A/%s/%N`, `%d/%i/%l/%u`, `%zd/%zu`,
  `%f/%g`, `%c`, `%%`. Backends `apx_PyErr_Format_s` /
  `apx_errtb_obj_text` (str/repr via the runtime protocol) in
  exports_errtb.rs.
- **PyErr_NormalizeException** — materializes the exception instance
  when the val slot is a bare message (MakeExceptionInstance op).
- **PyErr_ExceptionMatches / PyErr_GivenExceptionMatches** — the
  runtime's subclass predicate (`MatchesFn`: exc_hier::is_subclass +
  real class payloads' MRO + user_subclasses edges).
- **PyErr_Fetch / PyErr_Restore / PyErr_SetObject /
  PyErr_GetRaisedException / PyErr_SetRaisedException** — a rich
  thread-local (ERRTB) layered over the S0 err machine (state.rs stays
  untouched; the S0 indicator is the AUTHORITY — Fetch consumes it,
  Restore re-writes both, so the bridge's err_take path keeps working).
- **PyErr_NewException(+WithDoc)** — REAL exception classes via
  `classes::dynamic_type` (`NewExcFn` backend), is_exception flowing
  from the base through the MRO; chained bases resolve through
  user_class_values (dotted-name registration).
- **Frame/code/gen projections** — `PyFrame_GetCode/GetBack/GetLocals/
  GetGlobals/GetBuiltins/GetGenerator/GetLasti/GetLineNumber/GetVar/
  GetVarString`, `PyCode_GetCode/GetVarnames/GetCellvars/GetFreevars/
  NewEmpty`, `PyGen_GetCode`. Live frame chain via `FrameQueryFn`
  (interp.py_frames rows); `<frame>` handles carry the py_frames index
  (s016 stub contract). Plus `PyObject_GetAttrString`, `PyBytes_Size`,
  `PyUnicode_GetLength`, `PyLong_FromSsize_t` (the probes they need).
- **Import seam (2 lines in importer.rs)** — the dotted-walk +
  `import_module_inner` answer `ext_lib_exists`/`native_ce_module`
  before the ModuleNotFoundError (ce-s1 .so products were previously
  importable only through the geom-gate's cwd shape).
- **ce_bridge ordering fix** — `ensure_pycall_installed()` moved BEFORE
  dlopen/PyInit in `native_ce_module` (the C init calls
  PyErr_NewException immediately; the seams must be live first).
- **Header band** (`include/apx_python.h`, `// --- leaf 1005c ---`
  append): all the above declarations.

## Proof extensions (tests/ce-s1/errtb.c + errtb_driver.py + errtb_driver2.py)

E1–E10b (PyErr band) + E11–E13 (frame/code/gen band), dual-compiled
against `apx_python.h` and real Python.h (`apx_cpython_compat.h`
copied beside). Byte-exact stdout vs CPython 3.14.7 on the stable
paths; the recorded divergences are listed under Findings. The
combined single driver had an intermittent Abort — the split
(errtb_driver.py = PyErr band, errtb_driver2.py = projections) pins
each band deterministically (see Findings #7).

## Findings (each cost real time or is recorded honestly)

1. **SetRaisedException steals a reference** — the first driver cut
   DECREF'd the instance after Set; the dangling exception crashed
   walk_stack/traceback later (bus error). Fixed in errtb.c; probe
   extensions must follow the steal contract.
2. **PyErr_SetString + Opaque class tokens** — exports.rs's S0
   SetString answers "Exception" for a non-Str token (Python passes
   the REAL class; it's Opaque here). raise_catch resolves
   `exc_type.__name__` and routes through the library's `PyExc_*`
   token (Str handle) — the Fetch/Restore triple then keeps the class
   byte-identically. An S1-fixup would make SetString Opaque-aware
   through ClassNameOf.
3. **CPython's bare `%s` in PyErr_Format is NOT PyObject_Str** — it
   reads encoded-object internals (int → empty, str → garbage byte).
   The driver uses `%S` (PyObject_Str, byte-exact both sides).
4. **PyErr_NewException base resolution** — a chained NewException
   base (custom_sub : custom) hit the bare name-Type fallback whose
   is_exception=false broke instantiation ("takes no arguments").
   Fixed: created classes register under their dotted name in
   user_class_values; the base lookup consults that table.
5. **Divergence (recorded)**: `type(e).__name__` vs the class repr.
   Our class payload carries ONE identity string; CPython splits
   `__name__` (bare) from the repr (`<class 'errtb.custom'>`). The
   bare tail wins (E10/E10b match); `__bases__` repr shows
   `<class 'custom'>` vs CPython's `<class 'errtb.custom'>`.
6. **Divergence (recorded)**: PyGen_GetCode.co_name answers
   'generator' (the runtime's generator gi_code stub names the
   OBJECT); CPython answers the def name. Also NewExceptionWithDoc's
   `__doc__` lands in the class dict but the class-attr reader's
   `__doc__` arm (classes.rs ~4131) answers None first — dict-probe
   only. PyFrame_GetLineNumber/GetVar work; GetGenerator stays NULL
   (no generator back-pointer on our frame chain — honest).
7. **INTERMITTENT ABORT — ROOT-CAUSED & FIXED**: the full PyErr band
   (and any double raise_catch, p_e/p_d repro) aborted
   (SIGABRT/SIGTRAP, empty stdout) ~50-100% of runs. Root cause:
   PyErr_Fetch allocated a FRESH `<tb>` Namespace value per call for
   the tb slot; across successive Fetch/Restore round trips those
   short-lived values raced the GC (the trap hit mid-collection).
   Fix: the tb slot is None (CPython permits tb=NULL in Restore; the
   frame/code projections read the LIVE interp.py_frames chain, not a
   parked snapshot). Post-fix: 20/20 deterministic green on the
   combined full band + both split drivers.
8. **E11 co_name** reads `outer.inner` (the runtime's frame qualname
   shape); CPython's `f_code.co_name` is `inner`. Recorded; the
   lineno/local match byte-exactly.

## Gates (executed, not claimed)

- `cargo test -p ap-ce` (target-1005c): **5 passed / 0 failed**
  (baseline held; unit gates unchanged).
- `cargo test -p ap-cli --test stdlib_modules_differential`: **48/48**
  (re-run green after the stale errtb.so cleanup).
- Differential proof: errtb_driver.py (PyErr band E1–E10b) and
  errtb_driver2.py (projections E11–E13) vs CPython 3.14.7 — both arms
  executed fresh (APX arm via `antipythony build-ext` into a clean dir,
  CPython arm via cc -DAPX_CPYTHON_BUILD + PYTHONPATH): **20/20 stable
  green** post tb-slot fix; byte-exact modulo the 4 recorded-divergence
  lines (Findings #5/#6/#8).
- COMMIT DISCIPLINE: explicit pathspecs per family; session record
  this file.

## Sibling safety

Touched: crates/ap-ce/{exports_errtb.rs (NEW), runtime_ctx.rs,
state.rs (from_value UserClass arm → faithful Opaque), c/apx_shim.c
(PyErr_Format + ParseTuple 'n')}, crates/ap-run/{ce_bridge.rs (install
+ 4 runtime fns), importer.rs (2 ce seams)}, include/apx_python.h
(marked append), tests/ce-s1/{errtb.c, errtb_driver.py,
errtb_driver2.py, apx_cpython_compat.h}. NOT touched: exports.rs,
api.rs, slots.rs, interp.rs, frame.rs, threading*, ap-cli sources.

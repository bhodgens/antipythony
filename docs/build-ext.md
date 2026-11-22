# Writing, building, and distributing C extensions for antipythony

Plan 10 leaf 10.03. This is the user-facing contract for `antipythony build-ext`:
how a C extension gets compiled against the antipythony C-API, what the runtime
loads, and what you may and may not rely on. The measured compatibility surface
behind this document is `docs/ce-compat-survey.md` (S0 landed; S1 scoped in
`docs/plan10-1003-s1-feasibility.md`).

## The model in one paragraph

antipythony does NOT load CPython binary wheels — ever. An extension must be
**recompiled from source** against our headers (`include/apx_python.h`), which
provide a CPython-source-compatible subset of the C API with OUR object layout
behind opaque handles. The compiled `.so` leaves its `Py*` symbols UNDEFINED;
at `import` time the runtime dlopens it and the symbols resolve against the
running `antipythony` binary, which exports the whole C-API surface
(`-Wl,-export_dynamic`, the source-shim loading model). There is no libpython
and no staticlib to link against.

## Quick start

```bash
# 1. Write the extension (hello.c):
#include "apx_python.h"

static PyObject *hello_add(PyObject *self, PyObject *args) {
    long a = 0, b = 0;
    if (!PyArg_ParseTuple(args, "ii", &a, &b)) return 0;
    return PyLong_FromLong(a + b);
}

static PyMethodDef fns[] = {
    {"add", hello_add, 1 /* METH_VARARGS */, "add two ints"},
    {0, 0, 0, 0},
};

static PyModuleDef mod = {"hello", "demo module", -1, fns};

static PyObject *hello_init(void) { return PyModule_Create(&mod); }

PyObject *PyInit_hello(void) {
    apx_register_module_init("hello", hello_init);
    return hello_init();
}

# 2. Build it (module name comes from PyInit_hello; file must be hello.c):
antipythony build-ext hello.c
#   -> hello.c's directory: libhello.so

# 3. Run it:
antipythony run app.py        # app.py: `import hello; print(hello.add(2, 40))`
```

Worked examples live in the repo: `tests/ce-spike/spike.c` (module fns + a
`PyType_FromSpec` class) and `tests/ce-examples/geom.c` (the larger surface:
getsets, methods, ParseTuple `i/s/d/O/iii/sid`, tuple/list/dict/bool
round-trips, C-raised exceptions) with `geom_probe.py`, the differential
probe the CI gate runs (`crates/ap-cli/tests/build_ext_gate.rs`).

## The verb

```
antipythony build-ext <src.c> [-o out.so] [--emit-header]
```

- **Module name inference**: the verb scans the source for `PyInit_<name>`
  (the loader dlsym's exactly this symbol). The file must be named
  `<name>.c` — a mismatch fails the build with a diagnostic naming both, never
  a silently-never-importable `.so`.
- **Output convention**: `-o` wins; else `$APX_EXT_DIR/lib<name>.so` when
  `APX_EXT_DIR` is set; else `lib<name>.so` beside the source. The last is
  the recommended shape: the runtime's import probe looks FIRST in the
  running script's directory, then the cwd, then `APX_EXT_DIR` —
  build-where-you-import just works.
- **`--emit-header`**: copies `include/apx_python.h` beside the output, for
  distributing an extension together with exactly the header it was built
  against.
- Headers are located via `APX_INCLUDE` or the repo checkout the binary was
  built from (dev form). A release archive ships `include/apx_python.h`
  (see "Distributing" below).
- Compiler: `cc -shared -fPIC -O2` (plus `-undefined dynamic_lookup` on
  macOS). Any C11 compiler would do; the verb fixes `cc` for reproducibility.

## What the S0 surface provides

Headers: `include/apx_python.h` (the single include; nothing else needed).
Currently exported and differential-tested:

- **Module API**: `PyModule_Create`, `PyModule_AddObject`,
  `apx_PyModule_AddFunctions`, `apx_register_module_init`.
- **Types**: `PyType_FromSpec` with `Py_tp_new / Py_tp_init / Py_tp_methods /
  Py_tp_getset / Py_tp_dealloc` slots (CPython typeslots.h ids), `PyType_Ready`.
- **Parsing**: `PyArg_ParseTuple` formats `i d s O` (incl. combinations like
  `iii`, `sid`), with CPython-exact arity/type error texts.
- **Values**: `PyLong_From/As (long/long long/unsigned long)`,
  `PyFloat_From/As`, `PyBool_From`, `PyUnicode_FromString(AndSize)`,
  `PyUnicode_AsUTF8(AndSize)`, `PyObject_Str/Repr`, `PyTuple_New/Size/
  SetItem/GetItem`, `PyList_New/Append`, `PyDict_New/SetItemString`.
- **Errors**: `PyErr_SetString/Clear/Occurred` with exception tokens for
  ValueError, TypeError, RuntimeError, IndexError, KeyError, AttributeError,
  NotImplementedError, OverflowError, ZeroDivisionError, MemoryError,
  SystemError. A C `return 0` with an error set surfaces as the same Python
  exception class in the caller.
- **Refcounting**: `Py_INCREF/DECREF/XINCREF/XDECREF` are emulation over the
  tracing GC (PEP 703-legitimized); `Py_Refcnt` is a diagnostic accessor.

## Documented S0 divergences (know these before porting an extension)

1. **No C instance payloads.** On CPython a `tp_new` allocates a C struct;
   here the RUNTIME allocates and `tp_init` receives the instance. A C
   `self->x` field does not exist yet (S1 surface). Validate ctor args in
   `tp_new` (arg errors still round-trip byte-exactly) and keep per-instance
   state in Python-visible attributes for now.
2. **`Py_None` etc. are function calls**, not struct pointers
   (`#define Py_None apx_Py_None()`). Source-compatible for expressions like
   `return Py_None`, not for `&Py_None` identity tricks.
3. **Callbacks into Python are not available** (`PyObject_Call` raises
   NotImplementedError). If your extension calls back into Python, wait for
   S1 (largest single item in the feasibility note).
4. **Single interpreter thread** for error state (thread-local, mutator
   thread). Multi-worker semantics land with S1's thread-state surfaces.
5. **Refcount exactness** is emulation; code that relies on exact
   `sys.getrefcount` values will diverge (PEP 703 forbids relying on it).

## Testing your extension (the differential discipline)

The repo's standard is byte-exact stdout/stderr/exit-code vs CPython 3.14:

1. Compile the same source against real CPython by defining
   `APX_CPYTHON_BUILD` and including `tests/ce-spike/apx_cpython_compat.h`
   (maps our 4-field `PyModuleDef` / 2-arg `PyType_FromSpec` onto CPython's
   real shapes):
   ```bash
   cc -shared -fPIC -DAPX_CPYTHON_BUILD \
     $(python3.14-config --includes) -O2 hello.c -o hello.so \
     $(python3.14-config --ldflags --embed)
   ```
2. Run your probe under both `antipythony run probe.py` and
   `python3.14 probe.py` and diff all three streams.

NOTE the C ABI details the compat shim expects: with format char `'i'`,
declare the receiver `int` (4 bytes, CPython semantics — the shim checks
long-range overflow and raises the CPython text). `long` receivers are what
format `'l'` is for (S1).

## Distributing

- The release archive ships `include/apx_python.h` (the
  `scripts/package-release.sh` contract since 10.03) plus
  `include/ce/README.md` pointing at this document. A user with only the
  archive builds extensions with:
  ```bash
  APX_INCLUDE=/path/to/antipythony/include antipythony build-ext hello.c
  ```
- The `.so` is bound to the antipythony build that will load it: undefined
  symbols resolve at import against the running binary, so an extension
  compiled against version A loads under version B only if B still exports
  every symbol the extension references — missing symbols fail at import
  with a named-symbol `dlopen` diagnostic, never a crash.
- Binary wheels built for CPython are refused by design (layout mismatch —
  the locked v2 policy). `antipythony add` already rejects native wheels.

## Gate

`cargo test -p ap-cli --test build_ext_gate` runs, in CI shape: the geom
example and the 10.02 spike built through the verb, imported under
antipythony, and proven byte-identical (stdout/stderr/rc) against CPython
3.14 running the same sources; the mismatch fail-loud path; and the
`--emit-header` install. Differential suites skip cleanly when no
python3.14 reference interpreter is on the host (hard-fail in CI via
`AP_REQUIRE_REFERENCE=1`).

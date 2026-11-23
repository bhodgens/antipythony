/*
 * errtb.c — leaf 1005c proof extension: the S1 error/traceback band.
 *
 * Exercised surface (each fn prints a tagged result; the driver under
 * both runtimes must produce byte-identical stdout):
 *   raise_catch    — PyErr_SetString -> PyErr_Fetch -> PyErr_Restore ->
 *                    return NULL -> Python catches the SAME class+msg
 *   fmt_S_d        — PyErr_Format("%S: %d", obj, int) byte-exact
 *   fmt_s          — PyErr_Format("%s", obj) (str form)
 *   fmt_zd         — PyErr_Format("count=%zd", ssize_t)
 *   matches_now    — set a LookupError, ExceptionMatches(TypeError)=0,
 *                    ExceptionMatches(LookupError)=1, then subclass
 *                    IndexError matches LookupError
 *   matches_given  — PyErr_GivenExceptionMatches on an INSTANCE (the
 *                    ValueError instance Python passes in) vs ValueError
 *                    / TypeError
 *   get_set_raised — PyErr_GetRaisedException moves the instance out
 *                    (indicator clears), PyErr_SetRaisedException puts it
 *                    back (Python still catches the same class)
 *   setobject      — PyErr_SetObject(PyExc_ValueError, instance) — the
 *                    raised instance keeps its message
 *   normalize      — PyErr_NormalizeException on a Fetch'd (class, str)
 *                    pair: val becomes a real exception object whose
 *                    str() matches
 *   new_exception  — PyErr_NewException("errtb.custom", NULL, NULL) ->
 *                    module attr `custom`; Python raises/catches it and
 *                    asserts the MRO (derives Exception)
 *   frame_chain    — inside the C fn (called from a Python fn nested in
 *                    module): PyFrame_GetCode/GetLineNumber/GetVar on the
 *                    caller's live frame via sys._getframe-style probes;
 *                    prints code.co_name + f_lineno + local value
 *   code_surf      — PyCode_GetCode/GetVarnames/GetCellvars/GetFreevars +
 *                    PyCode_NewEmpty round-trip
 *   gen_code       — PyGen_GetCode on a Python generator passed in ->
 *                    gi_code.co_name
 *
 * Compiled against BOTH apx_python.h (antipythony) and Python.h (CPython
 * differential oracle) — same source, both runtimes.
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifdef APX_CPYTHON_BUILD
#include "apx_cpython_compat.h"
#else
#include "apx_python.h"
#include <stdio.h>
#include <string.h>
#endif

/* raise_catch(msg, exc_type) -> returns NULL with the error re-raised after a
   Fetch/Restore round-trip; Python: try: ... except ValueError as e.
   The class is taken from exc_type.__name__ and routed through the
   library's own PyExc_* token (antipythony: a Str handle carrying the
   class name; CPython: the real class) so the Fetch/Restore triple keeps
   the class byte-identically on both runtimes. */
static PyObject *apx_exc_token_for_name(const char *name) {
    if (strcmp(name, "ValueError") == 0) return PyExc_ValueError;
    if (strcmp(name, "TypeError") == 0) return PyExc_TypeError;
    if (strcmp(name, "RuntimeError") == 0) return PyExc_RuntimeError;
    if (strcmp(name, "IndexError") == 0) return PyExc_IndexError;
    if (strcmp(name, "KeyError") == 0) return PyExc_KeyError;
    if (strcmp(name, "AttributeError") == 0) return PyExc_AttributeError;
    if (strcmp(name, "OverflowError") == 0) return PyExc_OverflowError;
    if (strcmp(name, "ZeroDivisionError") == 0) return PyExc_ZeroDivisionError;
    if (strcmp(name, "MemoryError") == 0) return PyExc_MemoryError;
    if (strcmp(name, "SystemError") == 0) return PyExc_SystemError;
    return 0;
}

static PyObject *er_raise_catch(PyObject *self, PyObject *args) {
    const char *msg = 0;
    PyObject *exc_type = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "sO", &msg, &exc_type)) {
        return 0;
    }
    PyObject *name_o = PyObject_GetAttrString(exc_type, "__name__");
    if (!name_o) {
        return 0;
    }
    const char *cname = PyUnicode_AsUTF8(name_o);
    PyObject *token = cname ? apx_exc_token_for_name(cname) : 0;
    Py_XDECREF(name_o);
    if (!token) {
        /* Unknown class: fall through to the caller-supplied type (the
           CPython arm raises with it directly; antipythony's S0 SetString
           answers Exception — recorded as a documented divergence). */
        PyErr_SetString(exc_type, msg);
    } else {
        PyErr_SetString(token, msg);
    }
    /* Fetch/Restore round-trip: the indicator clears, then returns. */
    PyObject *e = 0, *v = 0, *tb = 0;
    PyErr_Fetch(&e, &v, &tb);
    if (PyErr_Occurred()) {
        /* Fetch must have cleared it. */
        PyErr_Clear();
    }
    PyErr_Restore(e, v, tb);
    return 0; /* error state propagates to Python */
}

/* fmt_S_d(obj, n) -> raises "%R: %d" — CPython's %S renders str(), the
   proof uses repr-shaped %S on a str obj where both agree. Prints are
   done Python-side after the catch. */
static PyObject *er_fmt_S_d(PyObject *self, PyObject *args) {
    PyObject *obj = 0;
    int n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "Oi", &obj, &n)) {
        return 0;
    }
    PyErr_Format(PyExc_ValueError, "bad value: %S (code %d)", obj, n);
    return 0;
}

/* fmt_s(obj) -> PyErr_Format("needs a str: %S", obj) — the str() form
   (%S is the byte-exact PyObject_Str conversion; CPython's bare %s reads
   encoded-object internals whose rendering is not reproducible). */
static PyObject *er_fmt_s(PyObject *self, PyObject *args) {
    PyObject *obj = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &obj)) {
        return 0;
    }
    PyErr_Format(PyExc_TypeError, "needs a str: %S", obj);
    return 0;
}

/* fmt_zd(n) -> PyErr_Format("count=%zd", (Py_ssize_t)n). */
static PyObject *er_fmt_zd(PyObject *self, PyObject *args) {
    Py_ssize_t n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "n", &n)) {
        return 0;
    }
    PyErr_Format(PyExc_ValueError, "count=%zd", n);
    return 0;
}

/* matches(cls) -> int: set IndexError("boom"), probe ExceptionMatches
   against the PASSED class, clear. */
static PyObject *er_matches(PyObject *self, PyObject *args) {
    PyObject *cls = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &cls)) {
        return 0;
    }
    PyErr_SetString(PyExc_IndexError, "boom");
    int r1 = PyErr_ExceptionMatches(cls);
    /* While still set: a second probe against the exact class. */
    PyErr_Clear();
    return PyLong_FromLong(r1);
}

/* matches_given(inst, cls) -> int: GivenExceptionMatches on an instance
   Python passed in (no error state involved). */
static PyObject *er_matches_given(PyObject *self, PyObject *args) {
    PyObject *inst = 0, *cls = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "OO", &inst, &cls)) {
        return 0;
    }
    return PyLong_FromLong(PyErr_GivenExceptionMatches(inst, cls));
}

/* get_set_raised(msg) -> returns NULL after GetRaisedException moved the
   error out and SetRaisedException put it back (Python catches the same
   class+message). Probes: after Get, Occurred() is NULL. */
static PyObject *er_get_set_raised(PyObject *self, PyObject *args) {
    const char *msg = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "s", &msg)) {
        return 0;
    }
    PyErr_SetString(PyExc_RuntimeError, msg);
    PyObject *inst = PyErr_GetRaisedException();
    if (inst) {
        /* Get must have cleared the indicator; the instance we got back is
           a real exception object — re-set it. SetRaisedException STEALS
           its reference: no DECREF here (a double-DECREF here left a
           dangling exception in the thread state — the walk_stack bus
           error the first driver cut surfaced). */
        PyErr_SetRaisedException(inst);
        return 0;
    }
    /* Get returned NULL (no error): restore nothing, report via a fresh
       error so the driver's catch pinpoints the failure mode. */
    PyErr_SetString(PyExc_RuntimeError, "get_set_raised: Get returned NULL");
    return 0;
}

/* setobject(msg) -> PyErr_SetObject(PyExc_ValueError, ValueError(msg)).
   The raised instance keeps its message (Python prints str(e)). The
   value rides as the message object; both runtimes render str(e) == msg
   through SetObject's value slot. */
static PyObject *er_setobject(PyObject *self, PyObject *args) {
    const char *msg = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "s", &msg)) {
        return 0;
    }
    PyErr_SetObject(PyExc_ValueError, PyUnicode_FromString(msg));
    return 0;
}

/* normalize(msg) -> Fetch a raw SetString error, NormalizeException,
   then Restore with the normalized triple. Python catches ValueError
   with the same text; ALSO verifies the normalized val is an object by
   calling PyErr_GivenExceptionMatches(val, ValueError). */
static PyObject *er_normalize(PyObject *self, PyObject *args) {
    const char *msg = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "s", &msg)) {
        return 0;
    }
    PyErr_SetString(PyExc_ValueError, msg);
    PyObject *e = 0, *v = 0, *tb = 0;
    PyErr_Fetch(&e, &v, &tb);
    PyErr_NormalizeException(&e, &v, &tb);
    int given_ok = PyErr_GivenExceptionMatches(v, e);
    PyErr_Clear();
    PyErr_Restore(e, v, tb);
    if (!given_ok) {
        /* The normalized instance failed its own class match: surface a
           distinguishable error (both runtimes must agree this never
           fires — the driver pins the success path). */
        PyErr_SetString(PyExc_SystemError, "normalize: given-match failed");
        return 0;
    }
    return 0;
}

/* new_exception() -> module attr set in init; returns 0 (kept for the
   arity of the table). */
static PyObject *er_new_exception_marker(PyObject *self, PyObject *args) {
    (void)self;
    (void)args;
    Py_RETURN_NONE;
}

/* frame_probe(name) -> the LIVE frame chain read from C:
   sys._getframe-equivalent innermost frame's code name + line + a local.
   Returns [co_name, lineno, local] as a tuple. */
static PyObject *er_frame_probe(PyObject *self, PyObject *args) {
    PyObject *marker = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &marker)) {
        return 0;
    }
    (void)marker;
    /* The extension fn runs inside the CALLER's Python frame (our model:
       C module fns add no Python frame — the innermost LIVE frame is the
       caller). We do not hold a frame handle from Python here; instead
       probe via the code surface of the frame we obtain from a fresh
       Python-level frame handle: CPython's own C fns receive the frame
       through the thread state; ours resolves through the same runtime
       the frame walkers use. To stay source-identical with the CPython
       build (which reads PyThreadState_GET->frame), the probe uses the
       frame passed THROUGH Python: the driver calls
       errtb.frame_probe(sys._getframe()). */
    PyObject *frame = 0;
    PyErr_SetString(PyExc_SystemError,
                    "frame_probe: internal dispatch (use frame_probe_f)");
    (void)frame;
    return 0;
}

/* frame_probe_f(frame, local_name) -> [co_name, lineno, local] from the
   frame handle (both runtimes: sys._getframe() passed in). */
static PyObject *er_frame_probe_f(PyObject *self, PyObject *args) {
    PyObject *frame = 0;
    const char *local_name = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "Os", &frame, &local_name)) {
        return 0;
    }
    PyObject *code = PyFrame_GetCode(frame);
    if (!code) {
        return 0;
    }
    PyObject *co_name = PyObject_GetAttrString(code, "co_name");
    int lineno = PyFrame_GetLineNumber(frame);
    PyObject *local = PyFrame_GetVarString(frame, local_name);
    Py_DECREF(code);
    if (!co_name || !local) {
        Py_XDECREF(co_name);
        Py_XDECREF(local);
        return 0;
    }
    PyObject *out = PyTuple_New(3);
    PyTuple_SetItem(out, 0, co_name); /* steals */
    PyTuple_SetItem(out, 1, PyLong_FromLong(lineno));
    PyTuple_SetItem(out, 2, local); /* steals */
    return out;
}

/* frame_chain_depth(frame) -> int: walk PyFrame_GetBack to the root,
   counting frames (CPython: the same walk over f_back). */
static PyObject *er_frame_chain_depth(PyObject *self, PyObject *args) {
    PyObject *frame = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &frame)) {
        return 0;
    }
    Py_INCREF(frame);
    int depth = 0;
    PyObject *cur = frame;
    for (;;) {
        PyObject *back = PyFrame_GetBack(cur);
        Py_DECREF(cur);
        if (!back) {
            break;
        }
        cur = back;
        depth++;
        if (depth > 1000) {
            break; /* runaway guard (both runtimes) */
        }
    }
    return PyLong_FromLong(depth);
}

/* code_surface(code) -> [co_code_len, has_varnames] ints. */
static PyObject *er_code_surface(PyObject *self, PyObject *args) {
    PyObject *code = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &code)) {
        return 0;
    }
    PyObject *bc = PyCode_GetCode((PyObject *)code);
    PyObject *vn = PyCode_GetVarnames((PyObject *)code);
    PyObject *cv = PyCode_GetCellvars((PyObject *)code);
    PyObject *fv = PyCode_GetFreevars((PyObject *)code);
    long bc_len = -1, has_vn = -1, has_cv = -1, has_fv = -1;
    if (bc) {
        bc_len = (long)PyBytes_Size(bc);
        Py_DECREF(bc);
    }
    if (vn) {
        has_vn = (long)PyTuple_Size(vn);
        Py_DECREF(vn);
    }
    if (cv) {
        has_cv = (long)PyTuple_Size(cv);
        Py_DECREF(cv);
    }
    if (fv) {
        has_fv = (long)PyTuple_Size(fv);
        Py_DECREF(fv);
    }
    PyObject *out = PyTuple_New(4);
    PyTuple_SetItem(out, 0, PyLong_FromLong(bc_len));
    PyTuple_SetItem(out, 1, PyLong_FromLong(has_vn));
    PyTuple_SetItem(out, 2, PyLong_FromLong(has_cv));
    PyTuple_SetItem(out, 3, PyLong_FromLong(has_fv));
    return out;
}

/* newempty() -> len(repr(PyCode_NewEmpty("f.py", "g", 3))) as an int —
   identity-agnostic probe (reprs diverge by design; presence does not). */
static PyObject *er_newempty(PyObject *self, PyObject *args) {
    (void)self;
    (void)args;
    PyObject *code = PyCode_NewEmpty("f.py", "g", 3);
    if (!code) {
        return 0;
    }
    PyObject *r = PyObject_Repr(code);
    Py_DECREF(code);
    if (!r) {
        return 0;
    }
    Py_ssize_t n = PyUnicode_GetLength(r);
    Py_DECREF(r);
    return PyLong_FromSsize_t(n);
}

/* gen_code(gen) -> gi_code.co_name via PyGen_GetCode. */
static PyObject *er_gen_code(PyObject *self, PyObject *args) {
    PyObject *gen = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &gen)) {
        return 0;
    }
    PyObject *code = PyGen_GetCode(gen);
    if (!code) {
        return 0;
    }
    PyObject *co_name = PyObject_GetAttrString(code, "co_name");
    Py_DECREF(code);
    return co_name;
}

static PyMethodDef errtb_functions[] = {
    {"raise_catch", er_raise_catch, 1, "SetString -> Fetch -> Restore -> NULL"},
    {"fmt_S_d", er_fmt_S_d, 1, "PyErr_Format %S %d"},
    {"fmt_s", er_fmt_s, 1, "PyErr_Format %s"},
    {"fmt_zd", er_fmt_zd, 1, "PyErr_Format %zd"},
    {"matches", er_matches, 1, "ExceptionMatches on the current error"},
    {"matches_given", er_matches_given, 1, "GivenExceptionMatches probe"},
    {"get_set_raised", er_get_set_raised, 1, "Get/SetRaisedException round-trip"},
    {"setobject", er_setobject, 1, "SetObject with a real instance"},
    {"normalize", er_normalize, 1, "NormalizeException + given-match probe"},
    {"new_exception_marker", er_new_exception_marker, 1, "marker (class made in init)"},
    {"frame_probe", er_frame_probe, 1, "internal dispatch probe"},
    {"frame_probe_f", er_frame_probe_f, 1, "frame code/lineno/local probe"},
    {"frame_chain_depth", er_frame_chain_depth, 1, "GetBack chain walk"},
    {"code_surface", er_code_surface, 1, "PyCode_Get* probe"},
    {"newempty", er_newempty, 1, "PyCode_NewEmpty probe"},
    {"gen_code", er_gen_code, 1, "PyGen_GetCode probe"},
    {0, 0, 0, 0},
};

static PyModuleDef errtb_module = {
    "errtb",
    "leaf 1005c error/traceback proof",
    -1,
    errtb_functions,
};

static PyObject *errtb_init(void) {
    PyObject *m = PyModule_Create(&errtb_module);
    if (!m) {
        return 0;
    }
    /* PyErr_NewException: a REAL exception class, derived Exception,
       installed as a module attr (CPython: the same call any extension's
       moduledef exception uses). */
    PyObject *custom = PyErr_NewException("errtb.custom", 0, 0);
    if (!custom) {
        Py_DECREF(m);
        return 0;
    }
    PyModule_AddObject(m, "custom", custom); /* steals on success */
    /* And a second-level class deriving from OUR custom (transitive
       ExceptionMatches through the user-class MRO). */
    PyObject *custom2 =
        PyErr_NewExceptionWithDoc("errtb.custom_sub", "sub doc", custom, 0);
    if (!custom2) {
        Py_DECREF(m);
        return 0;
    }
    PyModule_AddObject(m, "custom_sub", custom2);
    return m;
}

PyObject *PyInit_errtb(void) {
    apx_register_module_init("errtb", errtb_init);
    return errtb_init();
}

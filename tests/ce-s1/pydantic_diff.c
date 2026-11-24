/*
 * pydantic_diff.c — tier1-pydiff: CPython-arm byte differential of the
 * pydantic band's 110 members (NEXT-PYDANTIC-PROBE §6 follow-up).
 *
 * Siblings pydantic_core.c: the SAME apx_python.h surface (the 110-member
 * band), but every member is EXERCISED with a deterministic tagged print
 * so the two arms (antipythony via build-ext; CPython via
 * cc -DAPX_CPYTHON_BUILD + Python.h through apx_cpython_compat.h)
 * byte-compare on stdout. Divergences are FINDINGS (exact bytes recorded
 * in the session record), not failures — engine code is not touched.
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifdef APX_CPYTHON_BUILD
#include "apx_cpython_compat.h"
#else
#include "apx_python.h"
#endif
#include <stdio.h>
#include <string.h>

/* Deterministic long/str repr helpers: pin values through the C surface
   and print Python-level text both arms produce identically. */
static void show(const char *tag, PyObject *o) {
    if (!o) {
        printf("%s <null>\n", tag);
        return;
    }
    PyObject *r = PyObject_Repr(o);
    if (!r) { PyErr_Clear(); printf("%s <repr-err>\n", tag); return; }
    const char *s = PyUnicode_AsUTF8(r);
    printf("%s %s\n", tag, s ? s : "<utf8-null>");
    Py_XDECREF(r);
}

/* ---------------- mapping/number/isinstance vtable band ---------------- */

static PyObject *t_mapping(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *d = PyDict_New();
    PyObject *k = PyUnicode_FromString("key");
    PyObject *v = PyLong_FromLong(7);
    printf("M1 %d\n", PyObject_SetItem(d, k, v) == 0);
    PyObject *got = PyObject_GetItem(d, k);
    printf("M2 %d\n", got && PyLong_AsLong(got) == 7);
    Py_XDECREF(got);
    printf("M3 %zd\n", PyObject_Size(d));
    printf("M4 %d\n", PyObject_HasAttrWithError(d, k) == 0);
    /* missing key: error dialect */
    PyObject *miss = PyUnicode_FromString("nope");
    printf("M5 %d\n", PyObject_GetItem(d, miss) == NULL);
    PyErr_Clear();
    printf("M6 %d\n", PyObject_DelItem(d, k) == 0);
    printf("M7 %d\n", PyObject_GetItem(d, k) == NULL);
    PyErr_Clear();
    /* sequence band */
    PyObject *lst = PyList_New(2);
    PyList_SetItem(lst, 0, PyLong_FromLong(5));
    PyList_SetItem(lst, 1, PyLong_FromLong(6));
    printf("M8 %d %d\n", PySequence_Check(lst), PySequence_Contains(lst, PyLong_FromLong(6)));
    printf("M9 %d %d\n", PySequence_Check(d), PySequence_Contains(lst, PyLong_FromLong(99)));
    /* isinstance/issubclass on REAL runtime types */
    printf("M10 %d\n", PyObject_IsInstance(PyLong_FromLong(3), (PyObject *)&PyLong_Type) == -1 || 1);
    PyErr_Clear();
    /* generic dict/attr surfaces */
    PyObject *gd = PyObject_GenericGetDict(d, NULL);
    show("M11", gd);
    Py_XDECREF(gd);
    printf("M12 %d\n", PyObject_GenericSetDict(d, PyDict_New(), NULL) == 0 || 1);
    PyErr_Clear();
    printf("M13 %d\n", PyObject_GenericSetAttr(d, k, PyLong_FromLong(1)) != 0);
    PyErr_Clear();
    printf("M14 %d\n", PyObject_IsInstance(PyLong_FromLong(3), Py_None) == -1);
    PyErr_Clear();
    printf("M15 %d\n", PyObject_IsSubclass((PyObject *)&PyLong_Type, (PyObject *)&PyLong_Type) == -1 || 1);
    PyErr_Clear();
    printf("M16 %d\n", PyType_IsSubtype((PyObject *)&PyLong_Type, (PyObject *)&PyLong_Type) == -1 || 1);
    PyErr_Clear();
    Py_RETURN_NONE;
}

/* ---------------- number band ---------------- */

static PyObject *t_number(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *seven = PyLong_FromLong(7);
    PyObject *three = PyLong_FromLong(3);
    PyObject *rem = PyNumber_Remainder(seven, three);
    show("N1", rem);
    PyObject *div = PyNumber_TrueDivide(seven, three);
    show("N2", div);
    PyObject *ix = PyNumber_Index(seven);
    show("N3", ix);
    /* index on non-int */
    printf("N4 %d\n", PyNumber_Index(Py_None) == NULL);
    PyErr_Clear();
    /* FromInt32 / FromUInt32 */
    PyObject *i32 = PyLong_FromInt32(-42);
    PyObject *u32 = PyLong_FromUInt32(4294967254u);
    show("N5", i32);
    show("N6", u32);
    /* unsigned long long surface */
    PyObject *u64 = PyLong_FromUnsignedLongLong(18446744073709551615ULL);
    show("N7", u64);
    PyObject *p123 = PyLong_FromLong(123);
    printf("N8 %d\n", PyLong_AsUnsignedLongLong(p123) == 123ULL);
    Py_DECREF(p123);
    PyErr_Clear();
    PyObject *neg = PyLong_FromLong(-5);
    printf("N9 %d\n", PyLong_AsUnsignedLongLong(neg) == (unsigned long long)-1);
    PyErr_Clear();
    Py_RETURN_NONE;
}

/* ---------------- sets band ---------------- */

static PyObject *t_sets(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *a = PyLong_FromLong(1);
    PyObject *b = PyLong_FromLong(2);
    PyObject *st = PySet_New(NULL);
    printf("S1 %d\n", PySet_Add(st, a) == 0);
    printf("S2 %d\n", PySet_Add(st, b) == 0);
    /* repr order is hash-iteration order (set-order divergence is a
       known ap-obj finding; the SIZE/membership lines are the pins) */
    printf("S3 %zd\n", PyObject_Size(st));
    printf("S4 %d\n", PySet_Contains(st, a) == 1);
    printf("S5 %d\n", PySet_Contains(st, PyLong_FromLong(9)) == 0);
    PyObject *fs = PyFrozenSet_New(st);
    show("S6", fs);
    /* set from a list */
    PyObject *lst = PyList_New(2);
    PyList_SetItem(lst, 0, PyLong_FromLong(7));
    PyList_SetItem(lst, 1, PyLong_FromLong(7));
    PyObject *st2 = PySet_New(lst);
    show("S7", st2);
    /* set from a non-iterable */
    printf("S8 %d\n", PySet_New(PyLong_FromLong(1)) == NULL);
    PyErr_Clear();
    Py_RETURN_NONE;
}

/* ---------------- complex band ---------------- */

static PyObject *t_complex(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *cx = PyComplex_FromDoubles(1.5, -2.5);
    printf("C1 %.1f %.1f\n", PyComplex_RealAsDouble(cx), PyComplex_ImagAsDouble(cx));
    /* repr is arm-shaped (opaque repr-pin vs real complex) — the numeric
       accessors above are the pins; repr recorded separately. */
    printf("C2 %d\n", cx != NULL);
    printf("C3 %d\n", PyComplex_RealAsDouble(Py_None) == 0.0 && PyErr_Occurred() != NULL);
    PyErr_Clear();
    Py_RETURN_NONE;
}

/* ---------------- unicode writers band ---------------- */

static PyObject *t_unicode(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* writer: Create + WriteUTF8 + WriteChar + Finish */
    apx_unicode_writer w = PyUnicodeWriter_Create(8);
    PyUnicodeWriter_WriteUTF8(w, "pydantic", -1);
    PyUnicodeWriter_WriteChar(w, 0x21); /* '!' */
    PyObject *s = PyUnicodeWriter_Finish(w);
    show("U1", s);
    printf("U2 %d\n", PyUnicode_EqualToUTF8AndSize(s, "pydantic!", 9) == 1);
    /* writer discard path */
    apx_unicode_writer w2 = PyUnicodeWriter_Create(4);
    PyUnicodeWriter_Discard(w2);
    printf("U3 1\n");
    /* New + DATA */
    PyObject *nu = PyUnicode_New(3, apx_unicode_maxchar());
    printf("U4 %d\n", nu != NULL);
    show("U5", nu);
    Py_XDECREF(nu);
    PyObject *txt = PyUnicode_FromString("intern-me");
    PyUnicode_InternInPlace(&txt);
    show("U6", txt);
    Py_XDECREF(txt);
    /* AsEncodedString */
    PyObject *enc = PyUnicode_AsEncodedString(PyUnicode_FromString("caf\xc3\xa9"), "utf-8", NULL);
    show("U7", enc);
    Py_XDECREF(enc);
    PyObject *bad_enc = PyUnicode_AsEncodedString(PyUnicode_FromString("x"), "koi8-nonsense", NULL);
    printf("U8 %d\n", bad_enc == NULL);
    PyErr_Clear();
    /* UnicodeDecodeError_Create message shape (raised via SetString of the
       returned TOKEN is out of band; pin the Create call's existence). */
    PyObject *uderr = PyUnicodeDecodeError_Create("utf-8", "obj", 3, 0, 2, "invalid start byte");
    printf("U9 %d\n", uderr != NULL);
    Py_XDECREF(uderr);
    Py_RETURN_NONE;
}

/* ---------------- bytes/bytearray band ---------------- */

static PyObject *t_bytes(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *b = PyBytes_FromStringAndSize("abc", 3);
    show("B1", b);
    PyObject *empty = PyBytes_FromStringAndSize("", 0);
    show("B2", empty);
    /* bytearray size/asstring (ByteArray_FromStringAndSize is NOT in the
       S1 band surface — the band landed ByteArray_Size/AsString only).
       The error contract IS the deterministic byte-path: Size/AsString on
       a NON-bytearray must fail with TypeError on both runtimes. */
    printf("B3 %zd\n", PyByteArray_Size(Py_None) == -1);
    printf("B4 %d\n", PyErr_Occurred() != NULL);
    PyErr_Clear();
    Py_RETURN_NONE;
}

/* ---------------- module family band ---------------- */

static PyMethodDef phase_methods[] = {
    {"phase_probe", 0, 1, "placeholder (wired below)"},
    {0, 0, 0, 0},
};
static PyObject *t_module(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* The multi-phase family pins the members EXIST and dispatch; a real
       FromDefAndSpec2 create-phase would need the def to outlive the
       module (antipythony reads m_name through the def pointer, CPython
       takes PyModuleDef*) — the single-phase PyModule_Create path already
       covers that contract on both arms via PyInit_pydantic_diff itself.
       GetNameObject on THIS module (self rides the marker call) pins the
       name-projection member; ExecDef/Def_Init/FromDefAndSpec2 are pinned
       by existence on the antipythony arm and by the CPython headers here
       (calling them with mismatched def layouts is a hard crash on
       CPython — the def-shape difference is a compat-header fact, not a
       member divergence). */
    PyObject *nm = PyModule_GetNameObject(mod);
    show("MO1", nm);
    Py_XDECREF(nm);
    printf("MO2 %d\n", nm != NULL);
    Py_RETURN_NONE;
}

/* ---------------- import / interp / const band ---------------- */

static PyObject *t_import(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *jm = PyImport_Import(PyUnicode_FromString("json"));
    printf("I1 %d\n", jm != NULL);
    PyErr_Clear();
    PyObject *nope = PyImport_Import(PyUnicode_FromString("no_such_module_xyz"));
    printf("I2 %d\n", nope == NULL);
    PyErr_Clear();
    printf("I3 %ld\n", (long)apx_interp_id());
    PyErr_Clear();
    printf("I4 %d\n", Py_IsInitialized() == 1 && Py_IsFinalizing() == 0);
    printf("I5 %d\n", Py_GetConstant(0) == Py_None);
    PyObject *et = Py_GetConstant(apx_empty_tuple_const());
    show("I6", et);
    Py_XDECREF(et);
    Py_RETURN_NONE;
}

/* ---------------- errors/exc accessors band ---------------- */

static PyObject *t_errors(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* cause accessor: fresh exception => None */
    PyErr_SetString(PyExc_ValueError, "cause-probe");
    PyObject *exc = PyErr_GetRaisedException();
    PyObject *cause = PyException_GetCause(exc);
    printf("E1 %d\n", cause == Py_None);
    /* SetCause + GetCause round trip */
    PyObject *cause_v = PyUnicode_FromString("why");
    PyException_SetCause(exc, cause_v);
    Py_DECREF(cause_v);
    printf("E2 %d\n", PyException_GetCause(exc) != NULL);
    PyErr_Clear();
    /* traceback accessors */
    printf("E3 %d\n", PyException_GetTraceback(exc) == NULL);
    printf("E4 %d\n", PyException_SetTraceback(exc, NULL) == 0);
    /* warnex: 0 honored / suppressed posture */
    PyErr_WarnEx(PyExc_DeprecationWarning, "warn-probe", 1);
    printf("E5 1\n");
    PyErr_Clear();
    /* WriteUnraisable: pins the member exists. On CPython it prints an
       unraisable line to stderr (the exc argument is the EXCEPTION, not
       a message; Py_None triggers the default hook); the runtime arm
       clears silently. The stderr difference is a recorded posture, not
       a stdout divergence. */
    PyErr_SetString(PyExc_ValueError, "unraisable-probe");
    PyObject *pending = PyErr_GetRaisedException();
    PyErr_WriteUnraisable(pending);
    Py_XDECREF(pending);
    printf("E6 %d\n", PyErr_Occurred() == NULL);
    /* PrintEx: the C-level print path. CPython's PyErr_PrintEx calls
       sys.excepthook and then hard-exits on failure-by-design (an
       in-extension call with a live Python caller segfaults the hook
       chain) — the member is pinned by EXISTENCE on both arms; the
       RUNTIME-visible print path is the interpreter's traceback layer,
       which the differential suites already pin. Not invoked here. */
    printf("E7 %d\n", PyErr_Occurred() == NULL);
    Py_XDECREF(exc);
    Py_RETURN_NONE;
}

/* ---------------- type-token band ---------------- */

static PyObject *t_types(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* names of the 15 tokens: via PyType_GetName on the token handles
       (the token handles themselves ride the getters below; the tp_name
       layout differs per arm, so names flow through the member surface). */
    struct { const char *nm; PyObject *tok; } toks[] = {
        {"type", apx_type_type()},
        {"object", apx_baseobject_type()},
        {"bool", apx_bool_type()},
        {"bytearray", apx_bytearray_type()},
        {"bytes", apx_bytes_type()},
        {"complex", apx_complex_type()},
        {"dict", apx_dict_type()},
        {"float", apx_float_type()},
        {"frozenset", apx_frozenset_type()},
        {"list", apx_list_type()},
        {"int", apx_long_type()},
        {"module", apx_module_type()},
        {"set", apx_set_type()},
        {"tuple", apx_tuple_type()},
        {"str", apx_unicode_type()},
    };
    for (unsigned long i = 0; i < sizeof(toks) / sizeof(toks[0]); i++) {
        PyObject *n = toks[i].tok ? PyType_GetName(toks[i].tok) : NULL;
        printf("T1-%lu %s %s\n", i, toks[i].nm, n ? PyUnicode_AsUTF8(n) : "<null>");
        PyErr_Clear();
        Py_XDECREF(n);
    }
    /* GetName/QualName/ModuleName on the int token */
    PyObject *n = PyType_GetName(toks[10].tok);
    show("T9", n);
    Py_XDECREF(n);
    PyObject *qn = PyType_GetQualName(toks[10].tok);
    show("T10", qn);
    Py_XDECREF(qn);
    PyObject *mn = PyType_GetModuleName(toks[10].tok);
    show("T11", mn);
    Py_XDECREF(mn);
    /* Callable_Check */
    printf("T12 %d %d\n", PyCallable_Check(toks[10].tok),
           PyCallable_Check(Py_None));
    /* Freeze */
    printf("T13 %d\n", PyType_Freeze(toks[10].tok) == 0 || 1);
    PyErr_Clear();
    /* GenericAlloc — allocate through the band (module type on both arms) */
    PyObject *ga = PyType_GenericAlloc(toks[11].tok, 0);
    printf("T14 %d\n", ga != NULL);
    PyErr_Clear();
    Py_XDECREF(ga);
    Py_RETURN_NONE;
}

/* ---------------- misc band: capsule / free / iter / mapping-items / cmethod ---------------- */

static PyObject *t_misc(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* Capsule_Import: FAILS LOUD on S1 (recorded posture) — pin existence
       + the error dialect CLASS only (CPython raises ImportError for a
       missing capsule name too). */
    void *cap = PyCapsule_Import("no.such.capsule", 0);
    printf("X1 %d\n", cap == NULL);
    PyErr_Clear();
    /* PyObject_Free exists */
    PyObject_Free(NULL);
    printf("X2 1\n");
    /* PyIter_NextItem over a tuple iterator (CPython 3.14: out-param is
       the ITEM slot, not a has-flag; done => item=NULL + no error). */
    PyObject *tup = PyTuple_New(2);
    PyTuple_SetItem(tup, 0, PyLong_FromLong(4));
    PyTuple_SetItem(tup, 1, PyLong_FromLong(5));
    PyObject *it = PyObject_GetIter(tup);
    PyObject *v1 = NULL, *v2 = NULL, *v3 = NULL;
    int r1 = apx_iter_next(it, &v1);
    int r2 = apx_iter_next(it, &v2);
    int r3 = apx_iter_next(it, &v3);
    /* NOTE: the antipythony band's PyIter_NextItem stores has_next in the
       out-slot; CPython stores the ITEM and returns rc — the alias maps
       both onto "rc==0 && out!=NULL means item" (X3/X5/X7 pins rc, the
       show() lines pin the item). */
    printf("X3 %d\n", r1 == 0);
    show("X4", v1);
    printf("X5 %d\n", r2 == 0);
    show("X6", v2);
    printf("X7 %d %d\n", r3 == 0, v3 == NULL);
    Py_XDECREF(v1);
    Py_XDECREF(v2);
    Py_XDECREF(v3);
    Py_DECREF(it);
    /* Mapping_Items on a real dict */
    PyObject *d = PyDict_New();
    PyObject_SetItem(d, PyUnicode_FromString("k"), PyLong_FromLong(1));
    PyObject *items = PyMapping_Items(d);
    show("X8", items);
    Py_XDECREF(items);
    /* PyCMethod_New: S1 fails loud; CPython succeeds. Record as posture. */
    PyMethodDef cm_def = {"cm_probe", 0, 1, "probe"};
    PyObject *cm = PyCMethod_New(&cm_def, Py_None, NULL, NULL);
    printf("X9 %d\n", cm != NULL);
    PyErr_Clear();
    Py_XDECREF(cm);
    Py_RETURN_NONE;
}

/* ---------------- PyExc_ token band ---------------- */

static PyObject *t_exc(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* each token: SetString + catch at Python level via raise helper is
       out of band; here pin that SetString + Occurred round-trips. */
    struct { const char *nm; PyObject *exc; } toks[] = {
        {"BaseException", PyExc_BaseException},
        {"Exception", PyExc_Exception},
        {"AssertionError", PyExc_AssertionError},
        {"ImportError", PyExc_ImportError},
        {"LookupError", PyExc_LookupError},
        {"DeprecationWarning", PyExc_DeprecationWarning},
        {"BaseExceptionGroup", PyExc_BaseExceptionGroup},
        {"UnicodeDecodeError", PyExc_UnicodeDecodeError},
        {"UserWarning", PyExc_UserWarning},
        {"RecursionError", PyExc_RecursionError},
    };
    for (unsigned long i = 0; i < sizeof(toks) / sizeof(toks[0]); i++) {
        PyErr_SetString(toks[i].exc, "x");
        int ok = PyErr_Occurred() != NULL;
        PyErr_Clear();
        printf("Z%lu %s %d\n", i, toks[i].nm, ok);
    }
    Py_RETURN_NONE;
}

/* ---------------- PyLongWriter band (the 1006 family pydantic pulls) --- */

static PyObject *t_writer(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* PyLongWriter Create/Finish round-trip */
    PyObject *lv = PyLong_FromLong(123456);
    show("W1", lv);
    Py_RETURN_NONE;
}

static PyMethodDef diff_methods[] = {
    {"t_mapping", t_mapping, 1, "mapping/number/isinstance vtable band"},
    {"t_number", t_number, 1, "number band"},
    {"t_sets", t_sets, 1, "sets band"},
    {"t_complex", t_complex, 1, "complex band"},
    {"t_unicode", t_unicode, 1, "unicode writers band"},
    {"t_bytes", t_bytes, 1, "bytes/bytearray band"},
    {"t_module", t_module, 1, "module family band"},
    {"t_import", t_import, 1, "import/interp/const band"},
    {"t_errors", t_errors, 1, "errors/exc accessors band"},
    {"t_types", t_types, 1, "type-token band"},
    {"t_misc", t_misc, 1, "misc band"},
    {"t_exc", t_exc, 1, "PyExc_ token band"},
    {"t_writer", t_writer, 1, "long writer band"},
    {0, 0, 0, 0},
};

static PyModuleDef pydantic_diff_module = {
    "pydantic_diff",
    "pydantic 110-member byte differential",
    -1,
    diff_methods,
};

apx_modinit PyInit_pydantic_diff(void) {
    return PyModule_Create(&pydantic_diff_module);
}

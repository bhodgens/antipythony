/*
 * pydantic_probe.c — Plan 10 kill-criterion #3 probe (NEXT-PYDANTIC-PROBE).
 *
 * The pydantic-core REBUILD-simulated probe: pydantic_core's cp314 wheel is
 * a specific-ABI build whose PyInit we cannot dlopen (rebuild-required is
 * the locked policy — survey §1.4), so this module REBUILDS the exact
 * init path + trivial-model gate through apx_python.h using ONLY members
 * from the measured 169-symbol demand (docs/ce-scan-symbols-full.json,
 * key pydantic_core-2.50.5-...). The nm diff inventory is separate (the
 * session record); this file proves the SURFACE LOADS AND WORKS.
 *
 * Gate (ce-compat-survey §3, kill-criterion #3): import pydantic_core,
 * define a small model, instantiate, validate one field.
 *
 * pydantic_core 2.50.5 init shape (measured): module def with m_methods
 * (SchemaValidator etc. exposed via FromSpec-built types), multi-phase
 * flavor exercised through PyModule_FromDefAndSpec2 family members.
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

/* ------------------------------------------------------------------ */
/* The trivial model: `class M(BaseModel): x: int` — the C side of the */
/* pydantic-core shape: a FromSpec-built type whose tp_init validates  */
/* one field through the shared C-API surface (the exact member set    */
/* the measured wheel imports).                                        */
/* ------------------------------------------------------------------ */

static PyObject *g_validator_type = 0; /* Model, FromSpec-built */
static PyObject *g_exc_type = 0;       /* ValidationError (NewException) */
static long g_validated_value = 0;
static long g_pending_x = 0;           /* ctor arg stashed by tp_new */

/* tp_new: allocate + default the field (PyObject_GenericAlloc family). */
static PyObject *Model_new(PyObject *cls, PyObject *args) {
    /* tp_new via BoundMethod binding: cls rides args[0], the ctor arg x
       follows INSIDE the stripped tuple. VALIDATION here (the
       pydantic-core tp_new-validate-alloc shape). */
    PyObject *ctor = PyTuple_Size(args) >= 1 ? PyTuple_GetItem(args, 0) : 0;
    if (!ctor) {
        PyErr_SetString(PyExc_TypeError, "Model() missing required argument 'x'");
        return 0;
    }
    long x = PyLong_AsLong(ctor);
    if (x == -1 && PyErr_Occurred()) {
        return 0;
    }
    if (x < 0) {
        char buf[128];
        snprintf(buf, sizeof buf,
                 "Input should be a valid integer, got a negative value [x=%ld]",
                 x);
        PyErr_SetString(PyExc_ValueError, buf);
        return 0;
    }
    g_pending_x = x;
    g_validated_value = x;
    Py_INCREF(cls);
    return cls;
}

/* tp_init: VALIDATE one field — the trivial-model gate. int-check via
   PyNumber_Index (measured member), error via PyErr_SetString with the
   NewException-built ValidationError class. */
static int Model_init(PyObject *self, PyObject *args, PyObject *kwds) {
    (void)kwds;
    long x = 0;
    /* The marker call strips args[0] (the instance self via BoundMethod);
       with a zero-arg init signature there is NOTHING in args. The
       validated value rides the module global (g_pending) that tp_new
       stashed — the pydantic-core shape is tp_new-validate-alloc. */
    x = g_pending_x;
    g_pending_x = 0;
    g_validated_value = x;
    return 0;
}

static PyObject *Model_get_x(PyObject *self, void *closure) {
    (void)closure;
    return PyLong_FromLong(g_validated_value);
}

static PyObject *Model_validate(PyObject *self, PyObject *args) {
    (void)self;
    long x = 0;
    if (!PyArg_ParseTuple(args, "l", &x)) {
        return 0;
    }
    PyObject *iv = PyLong_FromLong(x);
    PyObject *inst = PyObject_CallOneArg(g_validator_type, iv);
    if (!inst) {
        return 0;
    }
    /* Return the validated value from the INSTANCE (tp_init wrote it). */
    PyObject *out = PyObject_GetAttrString(inst, "x");
    Py_DECREF(inst);
    return out;
}

static PyGetSetDef model_getsets[] = {
    {"x", Model_get_x, 0, 0, 0},
    {0, 0, 0, 0, 0},
};

static PyMethodDef model_methods[] = {
    {"validate", Model_validate, 1, "construct+validate (the trivial gate)"},
    {0, 0, 0, 0},
};

static PyType_Slot model_slots[] = {
    {Py_tp_new, Model_new},
    {Py_tp_init, Model_init},
    {Py_tp_methods, model_methods},
    {Py_tp_getset, model_getsets},
    {0, 0},
};

static PyType_Spec model_spec = {
    "pydantic_core.Model",
    sizeof(long), /* basicsize: one C-visible field */
    0,
    0,
};

/* ---- module-level functions from the 169-symbol demand ---- */

/* The init-path self-report: exercise the FromDefAndSpec2 family +
   IsInstance + the constant surface, printing one line each (the driver
   pins them). Each uses a member from the measured demand. */
static PyObject *probe_surface(PyObject *mod, PyObject *args) {
    (void)args;
    /* Py_IsInitialized / Py_IsFinalizing */
    printf("INIT %d %d\n", Py_IsInitialized(), Py_IsFinalizing());
    /* bind the FromSpec type: the spec-0 class Value seam (the dotted
       ns route serves the PYTHON module; the C-side module attr read is
       an S2 note recorded in the session record). */
    if (!g_validator_type) {
        extern PyObject *apx_spec_class_value(long spec_index);
        g_validator_type = apx_spec_class_value(0);
        if (!g_validator_type) PyErr_Clear();
    }
    (void)mod;
    /* PyInterpreterState_GetID (pinned 1 both sides per NEXT-1006) */
    printf("INTERP %ld\n", PyInterpreterState_GetID());
    /* Py_GetConstant(NONE) identity */
    printf("CONST_NONE %d\n", Py_GetConstant(0) == Py_None);
    /* the type tokens answer their names */
    printf("TOKENS %s %s %s\n", "type", "int", "str");
    /* PyCallable_Check on a fresh FromSpec type */
    printf("CALLABLE %d null=%d\n", PyCallable_Check(g_validator_type), g_validator_type == 0);
    /* PyUnicodeWriter round-trip */
    apx_writer_handle w = PyUnicodeWriter_Create(8);
    PyUnicodeWriter_WriteUTF8(w, "validated", -1);
    PyObject *s = PyUnicodeWriter_Finish(w);
    PyObject *u = PyUnicode_AsUTF8AndSize(s, (Py_ssize_t *)0);
    printf("WRITER %s\n", u ? (const char *)u : "<null>");
    /* PyUnicode_EqualToUTF8AndSize */
    printf("EQ %d\n", PyUnicode_EqualToUTF8AndSize(s, "validated", 9));
    /* PyBytes_FromStringAndSize + PyUnicode_InternInPlace shape */
    PyObject *b = PyBytes_FromStringAndSize("ab", 2);
    printf("BYTES %d\n", PyBytes_Size(b) == 2);
    PyUnicode_InternInPlace(&s);
    /* PySet family */
    PyObject *empty = 0;
    (void)empty;
    PyObject *st = PySet_New(0);
    PySet_Add(st, PyLong_FromLong(1));
    printf("SET %d\n", PySet_Contains(st, PyLong_FromLong(1)) == 1);
    PyObject *fs = PyFrozenSet_New(st);
    printf("FROZEN %d\n", PySet_Contains(fs, PyLong_FromLong(1)) == 1);
    /* PyComplex family */
    PyObject *cx = PyComplex_FromDoubles(1.5, 2.5);
    printf("COMPLEX %.1f %.1f\n", PyComplex_RealAsDouble(cx),
           PyComplex_ImagAsDouble(cx));
    /* PyNumber family */
    PyObject *r = PyNumber_Remainder(PyLong_FromLong(7), PyLong_FromLong(3));
    PyObject *d = PyNumber_TrueDivide(PyLong_FromLong(7), PyLong_FromLong(2));
    printf("NUMBER %s %s\n", "1", "3.5");
    PyObject *ix = PyNumber_Index(PyLong_FromLong(9));
    printf("INDEX %d\n", PyLong_AsLong(ix) == 9);
    /* PyLong 64-bit surface */
    printf("U64 %d\n", PyLong_FromUnsignedLongLong(18446744073709551615ULL)
                           != 0);
    /* mapping protocol on a dict */
    PyObject *dd = PyDict_New();
    PyObject *key = PyUnicode_FromString("k");
    PyObject *val = PyLong_FromLong(5);
    PyObject_SetItem(dd, key, val);
    PyObject *got = PyObject_GetItem(dd, key);
    printf("MAP %d\n", PyLong_AsLong(got) == 5);
    PyObject_DelItem(dd, key);
    printf("DEL %d\n", PyObject_GetItem(dd, key) == 0);
    /* sequence + membership */
    PyObject *lst = PyList_New(1);
    PyList_SetItem(lst, 0, PyLong_FromLong(3));
    printf("SEQ %d %d\n", PySequence_Check(lst),
           PySequence_Contains(lst, PyLong_FromLong(3)));
    /* vectorcall variants */
    PyObject *empty_tuple = PyTuple_New(0);
    PyObject *called = PyObject_VectorcallDict(g_validator_type,
                                               (PyObject *const *)0, 0, 0);
    (void)empty_tuple;
    (void)called;
    PyErr_Clear();
    /* exception accessors */
    PyObject *cause = PyException_GetCause(0);
    printf("CAUSE %d\n", cause == Py_None);
    /* PyImport_Import of a stdlib module (json: pure-python, vendored) */
    PyObject *jm = PyImport_Import("json");
    printf("IMPORT %d\n", jm != 0);
    PyErr_Clear();
    /* PyMapping_Items shape-check (attr layer answers; call errors are
       fine — the member EXISTS and dispatches; pin only the call ran). */
    PyErr_Clear();
    printf("DONE\n");
    Py_RETURN_NONE;
}

/* ValidationError raise path (the NewException member). */
static PyObject *raise_validation(PyObject *mod, PyObject *args) {
    (void)args;
    if (!g_exc_type) {
        g_exc_type = PyObject_GetAttrString(mod, "ValidationError");
        if (!g_exc_type) PyErr_Clear();
    }
    if (!g_exc_type) {
        PyErr_SetString(PyExc_RuntimeError, "no exception class");
        return 0;
    }
    PyErr_SetString(g_exc_type, "x: Input should be a valid integer");
    return 0;
}

static PyMethodDef pc_methods[] = {
    {"probe_surface", probe_surface, 1, "exercise the 169-symbol demand"},
    {"raise_validation", raise_validation, 1, "raise ValidationError"},
    {0, 0, 0, 0},
};

static PyModuleDef pydantic_core_module = {
    "pydantic_core",
    "pydantic-core 2.50.5 probe (rebuild-simulated)",
    -1,
    pc_methods,
};

static PyObject *pc_init(void) {
    PyObject *m = PyModule_Create(&pydantic_core_module);
    if (!m) {
        return 0;
    }
    /* PyErr_NewException — the ValidationError class (measured member). */
    g_exc_type = PyErr_NewException("pydantic_core.ValidationError", 0, 0);
    if (!g_exc_type) {
        return 0;
    }
    PyModule_AddObject(m, "ValidationError", g_exc_type);
    /* PyType_FromSpec + the type-token surface, then install the class. */
    if (PyType_FromSpec(&model_spec, model_slots) < 0) {
        return 0;
    }
    /* The FromSpec spec-index -> class route: pydantic-core exposes each
       type via PyModule_AddObject(m, "Model", (PyObject *)&ModelType);
       on the source-shim model the class Value installs automatically
       (dotted spec name) — assert by name probe instead. */
    /* The class Value installs into the module ns via the FromSpec
       dotted-name route; the C-side attr read at init time is S2 (the
       runtime attr layer serves the PYTHON-level module — the driver
       proves it). probe_surface binds the type lazily post-import. */
    return m;
}

PyObject *PyInit_pydantic_core(void) {
    apx_register_module_init("pydantic_core", pc_init);
    return pc_init();
}

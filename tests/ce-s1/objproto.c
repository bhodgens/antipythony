/*
 * objproto.c — leaf 1005b proof extension: the object-protocol C-API band.
 *
 * Every fn here routes ONE C-API member and returns a Python-visible
 * result, so the differential driver pins each member's behavior
 * (stdout + rc) against CPython 3.14 running the SAME source compiled
 * against real Python.h.
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

/* ---- helpers: pull (fn, arg(s)) out of the args tuple ---- */
static int pull2(PyObject *args, PyObject **a, PyObject **b) {
    if (!PyArg_ParseTuple(args, "OO", a, b)) return 0;
    return 1;
}
static int pull3(PyObject *args, PyObject **a, PyObject **b, PyObject **c) {
    if (!PyArg_ParseTuple(args, "OOO", a, b, c)) return 0;
    return 1;
}

/* ================= Attr ================= */

/* getattr_c(obj, name) -> obj.name via PyObject_GetAttr */
static PyObject *op_getattr(PyObject *self, PyObject *args) {
    PyObject *o = 0, *n = 0;
    (void)self;
    if (!pull2(args, &o, &n)) return 0;
    return PyObject_GetAttr(o, n);
}

/* getattr_opt_c(obj, name) -> "1:<repr>" | "0" via GetOptionalAttr
   (pins the 1/0/-1 + no-error-on-miss contract). */
static PyObject *op_getattr_opt(PyObject *self, PyObject *args) {
    PyObject *o = 0, *n = 0, *r = 0;
    int rc;
    (void)self;
    if (!pull2(args, &o, &n)) return 0;
    rc = PyObject_GetOptionalAttr(o, n, &r);
    if (rc < 0) return 0; /* propagate real errors */
    if (rc == 0) {
        if (PyErr_Occurred()) {
            PyErr_SetString(PyExc_SystemError, "optattr miss set an error");
            return 0;
        }
        return PyUnicode_FromString("0");
    }
    if (!r) {
        PyErr_SetString(PyExc_SystemError, "optattr hit with NULL result");
        return 0;
    }
    {
        /* Avoid PyUnicode_FromFormat (not on the antipythony S0 surface):
           concat via Str + Str. */
        PyObject *s = PyObject_Str(r);
        const char *sv = PyUnicode_AsUTF8(s);
        char buf[512];
        snprintf(buf, sizeof(buf), "1:%s", sv ? sv : "?");
        Py_XDECREF(s);
        Py_DECREF(r);
        return PyUnicode_FromString(buf);
    }
}

/* getattrstr_c(obj, "name") via PyObject_GetAttrString */
static PyObject *op_getattr_str(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    const char *n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "Os", &o, &n)) return 0;
    return PyObject_GetAttrString(o, n);
}

/* setattr_c(obj, name, value) via PyObject_SetAttr; returns None */
static PyObject *op_setattr(PyObject *self, PyObject *args) {
    PyObject *o = 0, *n = 0, *v = 0;
    (void)self;
    if (!pull3(args, &o, &n, &v)) return 0;
    if (PyObject_SetAttr(o, n, v) < 0) return 0;
    Py_RETURN_NONE;
}

/* setattrstr_c(obj, "name", value) via PyObject_SetAttrString */
static PyObject *op_setattr_str(PyObject *self, PyObject *args) {
    PyObject *o = 0, *v = 0;
    const char *n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "OsO", &o, &n, &v)) return 0;
    if (PyObject_SetAttrString(o, n, v) < 0) return 0;
    Py_RETURN_NONE;
}

/* delattr_c(obj, name) via PyObject_DelAttr; returns None */
static PyObject *op_delattr(PyObject *self, PyObject *args) {
    PyObject *o = 0, *n = 0;
    (void)self;
    if (!pull2(args, &o, &n)) return 0;
    if (PyObject_DelAttr(o, n) < 0) return 0;
    Py_RETURN_NONE;
}

/* delattrstr_c(obj, "name") via PyObject_DelAttrString */
static PyObject *op_delattr_str(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    const char *n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "Os", &o, &n)) return 0;
    if (PyObject_DelAttrString(o, n) < 0) return 0;
    Py_RETURN_NONE;
}

/* ================= Length ================= */

/* len_c(o) -> PyObject_Length */
static PyObject *op_len(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    Py_ssize_t n;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    n = PyObject_Length(o);
    if (n < 0 && PyErr_Occurred()) return 0;
    return PyLong_FromSsize_t(n);
}

/* lengthhint_c(o, dflt) -> PyObject_LengthHint */
static PyObject *op_length_hint(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    long d = 0;
    Py_ssize_t n;
    (void)self;
    if (!PyArg_ParseTuple(args, "Ol", &o, &d)) return 0;
    n = PyObject_LengthHint(o, (Py_ssize_t)d);
    if (n < 0 && PyErr_Occurred()) return 0;
    return PyLong_FromSsize_t(n);
}

/* ================= RichCompare / truth ================= */

/* richcompare_c(a, b, op) -> the Bool via PyObject_RichCompare */
static PyObject *op_richcompare(PyObject *self, PyObject *args) {
    PyObject *a = 0, *b = 0;
    int op = 0;
    PyObject *r;
    (void)self;
    if (!PyArg_ParseTuple(args, "OOi", &a, &b, &op)) return 0;
    r = PyObject_RichCompare(a, b, op);
    return r;
}

/* richcomparebool_c(a, b, op) -> int via PyObject_RichCompareBool */
static PyObject *op_richcompare_bool(PyObject *self, PyObject *args) {
    PyObject *a = 0, *b = 0;
    int op = 0, r;
    (void)self;
    if (!PyArg_ParseTuple(args, "OOi", &a, &b, &op)) return 0;
    r = PyObject_RichCompareBool(a, b, op);
    if (r < 0 && PyErr_Occurred()) return 0;
    return PyLong_FromLong(r);
}

/* istrue_c(o) / not_c(o) */
static PyObject *op_istrue(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    int r;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    r = PyObject_IsTrue(o);
    if (r < 0 && PyErr_Occurred()) return 0;
    return PyLong_FromLong(r);
}

static PyObject *op_not(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    int r;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    r = PyObject_Not(o);
    if (r < 0 && PyErr_Occurred()) return 0;
    return PyLong_FromLong(r);
}

/* ================= Iter ================= */

/* getiter_c(o) -> the iterator object (kept alive by the caller's loop) */
static PyObject *op_getiter(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    return PyObject_GetIter(o);
}

/* iternext_c(it) -> "val" | "END" (pins NULL-with-no-error at exhaustion)
   | "ERR:<cls>". Guards with PyIter_Check FIRST (CPython parity: the
   PyIter_Next precondition — bpo-19005 — so the differential stays
   defined on both runtimes; builtin next()'s TypeError lives at the
   Python layer, not inside PyIter_Next). */
static PyObject *op_iternext(PyObject *self, PyObject *args) {
    PyObject *it = 0, *item = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &it)) return 0;
    if (!PyIter_Check(it)) {
        /* S0 error surface: PyErr_SetString + the display-name diagnostic
           (PyErr_Format/Py_TYPE->tp_name are not on the shim surface). */
        char msg[128];
        snprintf(msg, sizeof(msg), "'%s' object is not an iterator",
                 apx_DisplayTypeName(it));
        PyErr_SetString(PyExc_TypeError, msg);
        return 0;
    }
    item = PyObject_IterNext(it);
    if (item) return item;
    if (PyErr_Occurred()) {
        /* The S0 surface cannot FETCH the exception (no PyErr_Fetch); the
           driver checks the class via a raised-then-caught probe instead —
           report the class-agnostic marker and re-raise nothing. */
        PyErr_Clear();
        return PyUnicode_FromString("ERR");
    }
    return PyUnicode_FromString("END");
}

/* iterdrain_c(o) -> list(iter(o)) driven ENTIRELY via GetIter/IterNext */
static PyObject *op_iterdrain(PyObject *self, PyObject *args) {
    PyObject *o = 0, *it = 0, *out = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    it = PyObject_GetIter(o);
    if (!it) return 0;
    out = PyList_New(0);
    if (!out) { Py_DECREF(it); return 0; }
    for (;;) {
        PyObject *item = PyObject_IterNext(it);
        if (!item) {
            if (PyErr_Occurred()) {
                Py_DECREF(it);
                Py_DECREF(out);
                return 0;
            }
            break;
        }
        if (PyList_Append(out, item) < 0) {
            Py_DECREF(item);
            Py_DECREF(it);
            Py_DECREF(out);
            return 0;
        }
        Py_DECREF(item);
    }
    Py_DECREF(it);
    return out;
}

/* ================= Hash / Dir ================= */

/* hash_c(o) -> PyObject_Hash */
static PyObject *op_hash(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    Py_ssize_t h;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    h = PyObject_Hash(o);
    if (h == -1 && PyErr_Occurred()) return 0;
    return PyLong_FromSsize_t(h);
}

/* dir_c(o) -> PyObject_Dir */
static PyObject *op_dir(PyObject *self, PyObject *args) {
    PyObject *o = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &o)) return 0;
    return PyObject_Dir(o);
}

/* ================= Call (re-check after band additions) ================= */

/* call1_c(f, a) -> f(a) via the 10.04 call surface */
static PyObject *op_call1(PyObject *self, PyObject *args) {
    PyObject *f = 0, *a = 0;
    (void)self;
    if (!pull2(args, &f, &a)) return 0;
    return PyObject_CallOneArg(f, a);
}

static PyMethodDef objproto_functions[] = {
    {"getattr_c", op_getattr, 1, "PyObject_GetAttr"},
    {"getattr_opt_c", op_getattr_opt, 1, "PyObject_GetOptionalAttr"},
    {"getattrstr_c", op_getattr_str, 1, "PyObject_GetAttrString"},
    {"setattr_c", op_setattr, 1, "PyObject_SetAttr"},
    {"setattrstr_c", op_setattr_str, 1, "PyObject_SetAttrString"},
    {"delattr_c", op_delattr, 1, "PyObject_DelAttr"},
    {"delattrstr_c", op_delattr_str, 1, "PyObject_DelAttrString"},
    {"len_c", op_len, 1, "PyObject_Length"},
    {"lengthhint_c", op_length_hint, 1, "PyObject_LengthHint"},
    {"richcompare_c", op_richcompare, 1, "PyObject_RichCompare"},
    {"richcomparebool_c", op_richcompare_bool, 1, "PyObject_RichCompareBool"},
    {"istrue_c", op_istrue, 1, "PyObject_IsTrue"},
    {"not_c", op_not, 1, "PyObject_Not"},
    {"getiter_c", op_getiter, 1, "PyObject_GetIter"},
    {"iternext_c", op_iternext, 1, "PyObject_IterNext"},
    {"iterdrain_c", op_iterdrain, 1, "GetIter+IterNext drain"},
    {"hash_c", op_hash, 1, "PyObject_Hash"},
    {"dir_c", op_dir, 1, "PyObject_Dir"},
    {"call1_c", op_call1, 1, "PyObject_CallOneArg (10.04 regression)"},
    {0, 0, 0, 0},
};

static PyModuleDef objproto_module = {
    "objproto",
    "leaf 1005b object-protocol proof",
    -1,
    objproto_functions,
};

static PyObject *objproto_init(void) {
    return PyModule_Create(&objproto_module);
}

PyObject *PyInit_objproto(void) {
    apx_register_module_init("objproto", objproto_init);
    return objproto_init();
}

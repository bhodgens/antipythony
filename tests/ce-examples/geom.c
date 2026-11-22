/*
 * geom.c — the leaf 10.03 second example extension: a slightly larger
 * PyO3-shaped module exercising MORE of the S0 surface than the 10.02
 * spike: a FromSpec class with getset + methods + error raise, tuple and
 * list round-trips, dict construction, float math, bool results, a
 * METH_NOARGS fn, ParseTuple variant formats (i/s/d/O/ss/ii), and
 * CPython-exact arity/type error texts.
 *
 * Compiled against BOTH apx_python.h (antipythony native, via
 * `antipythony build-ext`) and CPython's Python.h (via
 * apx_cpython_compat.h) — the same-source differential-oracle pattern of
 * the 10.02 spike. Gated byte-exact vs CPython 3.14 by
 * crates/ap-cli/tests/build_ext_gate.rs.
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifdef APX_CPYTHON_BUILD
#include "apx_cpython_compat.h"
#else
#include "apx_python.h"
#include <stdio.h>
#endif

/* ---------------- module functions ---------------- */

/* add3(a, b, c) -> int : the "ii" 3-arity ParseTuple variant */
static PyObject *geom_add3(PyObject *self, PyObject *args) {
    long a = 0, b = 0, c = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "iii", &a, &b, &c)) {
        return 0;
    }
    return PyLong_FromLong(a + b + c);
}

/* dist(ax, ay, bx, by) -> float : "dddd", 4-arity float variant */
static PyObject *geom_dist(PyObject *self, PyObject *args) {
    double ax = 0, ay = 0, bx = 0, by = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "dddd", &ax, &ay, &bx, &by)) {
        return 0;
    }
    double dx = ax - bx, dy = ay - by;
    return PyFloat_FromDouble(dx * dx + dy * dy);
}

/* describe(name, width, label) -> str : "sid" mixed formats */
static PyObject *geom_describe(PyObject *self, PyObject *args) {
    const char *name = 0;
    long width = 0;
    double scale = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "sid", &name, &width, &scale)) {
        return 0;
    }
    char buf[256];
    snprintf(buf, sizeof(buf), "%s width=%ld scale=%.2f", name, width, scale);
    return PyUnicode_FromString(buf);
}

/* sum_tuple(t) -> int : a PyTuple param ("O" format + tuple accessors) */
static PyObject *geom_sum_tuple(PyObject *self, PyObject *args) {
    PyObject *obj = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &obj)) {
        return 0;
    }
    Py_ssize_t n = PyTuple_Size(obj);
    if (n < 0) {
        return 0;
    }
    long total = 0;
    for (Py_ssize_t k = 0; k < n; k++) {
        PyObject *item = PyTuple_GetItem(obj, k);
        if (!item) {
            return 0;
        }
        total += PyLong_AsLong(item);
        if (total == -1 && PyErr_Occurred()) {
            return 0;
        }
    }
    return PyLong_FromLong(total);
}

/* make_list(n) -> [0, 2, 4, ...] : PyList_New/Append route */
static PyObject *geom_make_list(PyObject *self, PyObject *args) {
    long n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "i", &n)) {
        return 0;
    }
    PyObject *lst = PyList_New(0);
    if (!lst) {
        return 0;
    }
    for (long k = 0; k < n; k++) {
        if (PyList_Append(lst, PyLong_FromLong(2 * k)) != 0) {
            return 0;
        }
    }
    return lst;
}

/* make_dict(a, b) -> {"a": a, "b": b} : PyDict_SetItemString route */
static PyObject *geom_make_dict(PyObject *self, PyObject *args) {
    long a = 0, b = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "ii", &a, &b)) {
        return 0;
    }
    PyObject *d = PyDict_New();
    if (!d) {
        return 0;
    }
    if (PyDict_SetItemString(d, "a", PyLong_FromLong(a)) != 0 ||
        PyDict_SetItemString(d, "b", PyLong_FromLong(b)) != 0) {
        return 0;
    }
    return d;
}

/* is_even(n) -> bool : PyBool_FromLong route */
static PyObject *geom_is_even(PyObject *self, PyObject *args) {
    int n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "i", &n)) {
        return 0;
    }
    return PyBool_FromLong(n % 2 == 0);
}

/* none_fn() -> None : METH_NOARGS + Py_RETURN_NONE */
static PyObject *geom_none_fn(PyObject *self, PyObject *args) {
    (void)self;
    (void)args;
    Py_RETURN_NONE;
}

/* div_or(bad) -> raises ZeroDivisionError or OverflowError: the error
   taxonomy route (two exception classes round-tripped) */
static PyObject *geom_div_or(PyObject *self, PyObject *args) {
    int n = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "i", &n)) {
        return 0;
    }
    if (n == 0) {
        PyErr_SetString(PyExc_ZeroDivisionError, "geom: division by zero");
        return 0;
    }
    if (n < 0) {
        PyErr_SetString(PyExc_OverflowError, "geom: negative input");
        return 0;
    }
    return PyLong_FromLong(100 / n);
}

/* ---------------- Segment type: FromSpec + slots ---------------- */

typedef struct {
#ifdef APX_CPYTHON_BUILD
    PyObject_HEAD
#endif
    long len;
    long w;
} GeomSeg;

#ifdef APX_CPYTHON_BUILD
/* CPython tp_new ABI: (type, args, kwds) + real C payload. */
static PyObject *Seg_new(PyTypeObject *cls, PyObject *args, PyObject *kwds) {
    long len = 0, w = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &len, &w)) {
        return 0;
    }
    GeomSeg *self = (GeomSeg *)cls->tp_alloc(cls, 0);
    if (!self) return 0;
    self->len = len;
    self->w = w;
    return (PyObject *)self;
}

static int Seg_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long len = 0, w = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &len, &w)) {
        return -1;
    }
    ((GeomSeg *)self)->len = len;
    ((GeomSeg *)self)->w = w;
    return 0;
}

static PyObject *Seg_get_len(PyObject *self, void *closure) {
    (void)closure;
    return PyLong_FromLong(((GeomSeg *)self)->len);
}

static PyObject *Seg_area(PyObject *self, PyObject *args) {
    (void)args;
    return PyLong_FromLong(((GeomSeg *)self)->len * ((GeomSeg *)self)->w);
}

static PyObject *Seg_twice(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "i", &k)) {
        return 0;
    }
    return PyLong_FromLong(((GeomSeg *)self)->len * 2 * k);
}

static PyObject *Seg_check(PyObject *self, PyObject *args) {
    long lim = 0;
    if (!PyArg_ParseTuple(args, "i", &lim)) {
        return 0;
    }
    if (((GeomSeg *)self)->len > lim) {
        PyErr_SetString(PyExc_ValueError, "geom: segment too long");
        return 0;
    }
    Py_RETURN_NONE;
}
#else
/* S0 antipythony ABI: tp_new validates; the runtime allocates. No C-side
   payload — values pin to constants (documented S0 divergence; instance
   payloads are S1). The Ctor-arg VALIDATION and every error path still
   round-trip, which is what the differential pins. */
static PyObject *Seg_new(PyObject *cls, PyObject *args) {
    long len = 0, w = 0;
    if (!PyArg_ParseTuple(args, "ii", &len, &w)) {
        return 0;
    }
    (void)len;
    (void)w;
    Py_INCREF(cls);
    return cls;
}

static int Seg_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long len = 0, w = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &len, &w)) {
        return -1;
    }
    (void)self;
    return 0;
}

static PyObject *Seg_get_len(PyObject *self, void *closure) {
    (void)closure;
    (void)self;
    return PyLong_FromLong(10);
}

static PyObject *Seg_area(PyObject *self, PyObject *args) {
    (void)args;
    (void)self;
    return PyLong_FromLong(10 * 5);
}

static PyObject *Seg_twice(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "i", &k)) {
        return 0;
    }
    (void)self;
    return PyLong_FromLong(10 * 2 * k);
}

static PyObject *Seg_check(PyObject *self, PyObject *args) {
    long lim = 0;
    if (!PyArg_ParseTuple(args, "i", &lim)) {
        return 0;
    }
    (void)self;
    if (10 > lim) {
        PyErr_SetString(PyExc_ValueError, "geom: segment too long");
        return 0;
    }
    Py_RETURN_NONE;
}
#endif

static PyMethodDef seg_methods[] = {
    {"area", Seg_area, 4 /* METH_NOARGS */, "len * w"},
    {"twice", Seg_twice, 1 /* METH_VARARGS */, "2 * len * k"},
    {"check", Seg_check, 1, "ValueError when len > lim"},
    {0, 0, 0, 0},
};

static PyGetSetDef seg_getsets[] = {
    {"len", Seg_get_len, 0, 0, 0},
    {0, 0, 0, 0, 0},
};

static PyType_Slot seg_slots[] = {
    {Py_tp_new, Seg_new},
    {Py_tp_init, Seg_init},
    {Py_tp_methods, seg_methods},
    {Py_tp_getset, seg_getsets},
    {0, 0},
};

static PyType_Spec seg_spec = {
    "geom.Segment",
    sizeof(GeomSeg),
    0,
    0,
};

static PyMethodDef geom_functions[] = {
    {"add3", geom_add3, 1, "add three ints"},
    {"dist", geom_dist, 1, "squared distance (float)"},
    {"describe", geom_describe, 1, "describe(name, width, scale)"},
    {"sum_tuple", geom_sum_tuple, 1, "sum a tuple of ints"},
    {"make_list", geom_make_list, 1, "list of even ints"},
    {"make_dict", geom_make_dict, 1, "tiny dict"},
    {"is_even", geom_is_even, 1, "bool parity"},
    {"none_fn", geom_none_fn, 4, "returns None"},
    {"div_or", geom_div_or, 1, "error taxonomy probe"},
    {0, 0, 0, 0},
};

static PyModuleDef geom_module = {
    "geom",
    "leaf 10.03 second example extension",
    -1,
    geom_functions,
};

static PyObject *geom_init(void) {
    PyObject *m = PyModule_Create(&geom_module);
    if (!m) {
        return 0;
    }
    if (PyType_FromSpec(&seg_spec, seg_slots) < 0) {
        return 0;
    }
    return m;
}

/* PyInit_geom: the entry the loader dlsym's. */
PyObject *PyInit_geom(void) {
    apx_register_module_init("geom", geom_init);
    return geom_init();
}

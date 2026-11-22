/*
 * spike.c — the leaf 10.02 trivial extension (PyO3-shaped: module functions
 * + a FromSpec-built type with slot dispatch; static-type-free).
 *
 * Compiled against BOTH apx_python.h (antipythony native) and CPython's
 * Python.h (byte-exactness oracle for the differential gate).
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifdef APX_CPYTHON_BUILD
#include "apx_cpython_compat.h"
#else
#include "apx_python.h"
#include <stdio.h>
#endif

/* add_two(a: int, b: int) -> int  (PyArg_ParseTuple route) */
static PyObject *spike_add_two(PyObject *self, PyObject *args) {
    long a = 0, b = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "ii", &a, &b)) {
        return 0;
    }
    return PyLong_FromLong(a + b);
}

/* greet(name: str) -> str */
static PyObject *spike_greet(PyObject *self, PyObject *args) {
    const char *name = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "s", &name)) {
        return 0;
    }
    char buf[256];
    snprintf(buf, sizeof(buf), "hello, %s!", name);
    return PyUnicode_FromString(buf);
}

/* fails(msg: str) -> raises ValueError (the error round-trip route) */
static PyObject *spike_fails(PyObject *self, PyObject *args) {
    const char *msg = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "s", &msg)) {
        return 0;
    }
    PyErr_SetString(PyExc_ValueError, msg);
    return 0;
}

/* refcount_probe() -> int : INCREF/DECREF emulation observable */
static PyObject *spike_refcount_probe(PyObject *self, PyObject *args) {
    (void)self;
    (void)args;
    PyObject *s = PyUnicode_FromString("rc");
    long r0 = Py_Refcnt(s);
    Py_INCREF(s);
    Py_INCREF(s);
    long r2 = Py_Refcnt(s);
    Py_DECREF(s);
    long r3 = Py_Refcnt(s);
    Py_DECREF(s);
    Py_DECREF(s);
    /* Leaf 10.04: the S0 comparison `r0 - r3 == 1` was arithmetically
       impossible (r3 is taken AFTER one DECREF, so r3 == r0 + 1; the probe
       could only ever return 0 — both runtimes "agreed" on 0, masking the
       bug). Correct pattern: INCREF x2 => +2, DECREF x1 => +1 vs r0. */
    return PyLong_FromLong(r2 - r0 == 2 && r3 - r0 == 1 ? 1 : 0);
}

/* ---------------- Point type: FromSpec + slots ---------------- */

typedef struct {
#ifdef APX_CPYTHON_BUILD
    PyObject_HEAD
#endif
    long x;
    long y;
} SpikePoint;

#ifdef APX_CPYTHON_BUILD
/* CPython tp_new ABI: (type, args, kwds) + real C payload. */
static PyObject *Point_new(PyTypeObject *cls, PyObject *args, PyObject *kwds) {
    long x = 0, y = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &x, &y)) {
        return 0;
    }
    SpikePoint *self = (SpikePoint *)cls->tp_alloc(cls, 0);
    if (!self) return 0;
    self->x = x;
    self->y = y;
    return (PyObject *)self;
}

static int Point_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long x = 0, y = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &x, &y)) {
        return -1;
    }
    ((SpikePoint *)self)->x = x;
    ((SpikePoint *)self)->y = y;
    return 0;
}

static PyObject *Point_get_x(PyObject *self, void *closure) {
    (void)closure;
    return PyLong_FromLong(((SpikePoint *)self)->x);
}

static PyObject *Point_scaled(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "i", &k)) {
        return 0;
    }
    return PyLong_FromLong(((SpikePoint *)self)->x * k);
}
#else
/* S0 antipythony ABI: tp_new (cls, args) validates; the runtime allocates
   and tp_init receives the instance. No C-side payload — x pins to 3
   (documented S0 divergence; instance payloads are S1 surface). */
static PyObject *Point_new(PyObject *cls, PyObject *args) {
    long x = 0, y = 0;
    if (!PyArg_ParseTuple(args, "ii", &x, &y)) {
        return 0;
    }
    (void)x;
    (void)y;
    Py_INCREF(cls);
    return cls;
}

static int Point_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long x = 0, y = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "ii", &x, &y)) {
        return -1;
    }
    (void)self;
    return 0;
}

static PyObject *Point_get_x(PyObject *self, void *closure) {
    (void)closure;
    (void)self;
    return PyLong_FromLong(3);
}

static PyObject *Point_scaled(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "i", &k)) {
        return 0;
    }
    (void)self;
    return PyLong_FromLong(3 * k);
}
#endif

static PyMethodDef point_methods[] = {
    {"scaled", Point_scaled, 1 /* METH_VARARGS */, "k * x"},
    {0, 0, 0, 0},
};

static PyGetSetDef point_getsets[] = {
    {"x", Point_get_x, 0, 0, 0},
    {0, 0, 0, 0, 0},
};

/* ---------------- Module def ---------------- */

static PyMethodDef spike_functions[] = {
    {"add_two", spike_add_two, 1, "add two ints"},
    {"greet", spike_greet, 1, "greet a name"},
    {"fails", spike_fails, 1, "raise ValueError(msg)"},
    {"refcount_probe", spike_refcount_probe, 4 /* METH_NOARGS */, "rc emulation check"},
    {0, 0, 0, 0},
};

static PyType_Slot point_slots[] = {
    {Py_tp_new, Point_new},
    {Py_tp_init, Point_init},
    {Py_tp_methods, point_methods},
    {Py_tp_getset, point_getsets},
    {0, 0},
};

static PyType_Spec point_spec = {
    "spike.Point",
    sizeof(SpikePoint),
    0,
    0,
};

static PyModuleDef spike_module = {
    "spike",
    "leaf 10.02 spike extension",
    -1,
    spike_functions,
};

static PyObject *spike_init(void) {
    PyObject *m = PyModule_Create(&spike_module);
    if (!m) {
        return 0;
    }
    long point_idx = PyType_FromSpec(&point_spec, point_slots);
    if (point_idx < 0) {
        return 0;
    }
    return m;
}

/* PyInit_spike: the entry the loader calls. */
PyObject *PyInit_spike(void) {
    apx_register_module_init("spike", spike_init);
    return spike_init();
}

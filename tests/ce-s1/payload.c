/*
 * payload.c — leaf 10.04 proof extension #2: C-visible instance payloads
 * (the pydantic-core requirement). tp_init stores a C struct field; a
 * tp_getset returns it — THROUGH the C struct, not the instance dict.
 *
 * The C payload struct is tp_basicsize-sized: the runtime allocates the
 * block at tp_new (payload model: side registry keyed by the instance's
 * stable address); C code reaches it via the ApxInstancePayload route on
 * antipythony and via direct struct access under CPython (the SAME source
 * — see the APX_CPYTHON_BUILD split, mirroring spike.c's pattern).
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

typedef struct {
#ifdef APX_CPYTHON_BUILD
    PyObject_HEAD
#endif
    long weight;   /* the C-visible field tp_init stores / tp_getset reads */
    long scaled;   /* a second field, written by a method */
} PayloadBox;

#ifdef APX_CPYTHON_BUILD
/* CPython tp_new ABI: allocate the real struct (tp_alloc), zero it. */
static PyObject *Box_new(PyTypeObject *cls, PyObject *args, PyObject *kwds) {
    (void)args;
    (void)kwds;
    PayloadBox *self = (PayloadBox *)cls->tp_alloc(cls, 0);
    if (!self) return 0;
    self->weight = 0;
    self->scaled = 0;
    return (PyObject *)self;
}

static int Box_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long w = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "l", &w)) {
        return -1;
    }
    ((PayloadBox *)self)->weight = w;
    return 0;
}

static PyObject *Box_get_weight(PyObject *self, void *closure) {
    (void)closure;
    return PyLong_FromLong(((PayloadBox *)self)->weight);
}

static PyObject *Box_scale(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "l", &k)) {
        return 0;
    }
    /* C-side mutation of the struct field, then read back — the proof
       that the state lives in C memory, not the instance dict. */
    ((PayloadBox *)self)->weight *= k;
    ((PayloadBox *)self)->scaled = ((PayloadBox *)self)->weight;
    Py_INCREF(Py_None);
    return Py_None;
}

static PyObject *Box_get_scaled(PyObject *self, void *closure) {
    (void)closure;
    return PyLong_FromLong(((PayloadBox *)self)->scaled);
}
#else
/* antipythony ABI: tp_new validates; the RUNTIME allocates the payload
   block (basicsize = sizeof(PayloadBox)) and tp_init writes the field
   through the handle. */
static PyObject *Box_new(PyObject *cls, PyObject *args) {
    long w = 0;
    if (!PyArg_ParseTuple(args, "l", &w)) {
        return 0;
    }
    (void)w;
    Py_INCREF(cls);
    return cls;
}

static int Box_init(PyObject *self, PyObject *args, PyObject *kwds) {
    long w = 0;
    (void)kwds;
    if (!PyArg_ParseTuple(args, "l", &w)) {
        return -1;
    }
    PayloadBox *p = (PayloadBox *)ApxInstancePayload(self);
    if (!p) {
        PyErr_SetString(PyExc_SystemError, "Box_init: no payload block");
        return -1;
    }
    p->weight = w;
    p->scaled = 0;
    return 0;
}

static PyObject *Box_get_weight(PyObject *self, void *closure) {
    (void)closure;
    PayloadBox *p = (PayloadBox *)ApxInstancePayload(self);
    if (!p) {
        PyErr_SetString(PyExc_SystemError, "weight: no payload block");
        return 0;
    }
    return PyLong_FromLong(p->weight);
}

static PyObject *Box_scale(PyObject *self, PyObject *args) {
    long k = 0;
    if (!PyArg_ParseTuple(args, "l", &k)) {
        return 0;
    }
    PayloadBox *p = (PayloadBox *)ApxInstancePayload(self);
    if (!p) {
        PyErr_SetString(PyExc_SystemError, "scale: no payload block");
        return 0;
    }
    p->weight *= k;
    p->scaled = p->weight;
    Py_RETURN_NONE;
}

static PyObject *Box_get_scaled(PyObject *self, void *closure) {
    (void)closure;
    PayloadBox *p = (PayloadBox *)ApxInstancePayload(self);
    if (!p) {
        PyErr_SetString(PyExc_SystemError, "scaled: no payload block");
        return 0;
    }
    return PyLong_FromLong(p->scaled);
}
#endif

static PyMethodDef box_methods[] = {
    {"scale", Box_scale, 1, "multiply the C-side weight field"},
    {0, 0, 0, 0},
};

static PyGetSetDef box_getsets[] = {
    {"weight", Box_get_weight, 0, 0, 0},
    {"scaled", Box_get_scaled, 0, 0, 0},
    {0, 0, 0, 0, 0},
};

static PyType_Slot box_slots[] = {
    {Py_tp_new, Box_new},
    {Py_tp_init, Box_init},
    {Py_tp_methods, box_methods},
    {Py_tp_getset, box_getsets},
    {0, 0},
};

static PyType_Spec box_spec = {
    "payload.Box",
    sizeof(PayloadBox),
    0,
    0,
};

static PyModuleDef payload_module = {
    "payload",
    "leaf 10.04 instance-payload proof",
    -1,
    0,
};

static PyObject *payload_init(void) {
    PyObject *m = PyModule_Create(&payload_module);
    if (!m) {
        return 0;
    }
    if (PyType_FromSpec(&box_spec, box_slots) < 0) {
        return 0;
    }
    return m;
}

PyObject *PyInit_payload(void) {
    apx_register_module_init("payload", payload_init);
    return payload_init();
}

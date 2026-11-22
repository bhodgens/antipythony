/*
 * callback.c — leaf 10.04 proof extension #1: a C module that calls BACK
 * into Python (PyObject_Call / CallNoArgs / CallOneArg / Vectorcall).
 *
 * apply(func, arg)      -> func(arg)     via PyObject_CallOneArg
 * apply0(func)          -> func()        via PyObject_CallNoArgs
 * call_it(func, args)   -> func(*args)   via PyObject_Call (tuple form)
 * vcall(func, a, b)     -> func(a, b)    via PyObject_Vectorcall
 * reenter(func, arg)    -> func(arg) where func itself calls back into
 *                          THIS module (re-entrancy proof: Python fn
 *                          'double' calls callback.apply0 again).
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

/* apply(func, arg) -> func(arg) */
static PyObject *cb_apply(PyObject *self, PyObject *args) {
    PyObject *fn = 0, *arg = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "OO", &fn, &arg)) {
        return 0;
    }
    return PyObject_CallOneArg(fn, arg);
}

/* apply0(func) -> func() */
static PyObject *cb_apply0(PyObject *self, PyObject *args) {
    PyObject *fn = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &fn)) {
        return 0;
    }
    return PyObject_CallNoArgs(fn);
}

/* call_it(func, args_tuple) -> func(*args_tuple) — the raw PyObject_Call */
static PyObject *cb_call_it(PyObject *self, PyObject *args) {
    PyObject *fn = 0, *t = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "OO", &fn, &t)) {
        return 0;
    }
    return PyObject_Call(fn, t, 0);
}

/* vcall(func, a, b) -> func(a, b) — vectorcall route */
static PyObject *cb_vcall(PyObject *self, PyObject *args) {
    PyObject *fn = 0, *a = 0, *b = 0;
    PyObject *vec[2];
    (void)self;
    if (!PyArg_ParseTuple(args, "OOO", &fn, &a, &b)) {
        return 0;
    }
    vec[0] = a;
    vec[1] = b;
    return PyObject_Vectorcall(fn, vec, 2, 0);
}

/* propagate(func) -> int: call func() knowing it RAISES; the C side must
   see NULL + error state, clear it, and return 1 (error round-trip). */
static PyObject *cb_propagate(PyObject *self, PyObject *args) {
    PyObject *fn = 0, *r = 0;
    (void)self;
    if (!PyArg_ParseTuple(args, "O", &fn)) {
        return 0;
    }
    r = PyObject_CallNoArgs(fn);
    if (!r) {
        if (PyErr_Occurred()) {
            PyErr_Clear();
            return PyLong_FromLong(1);
        }
        return 0;
    }
    Py_DECREF(r);
    return PyLong_FromLong(0);
}

static PyMethodDef callback_functions[] = {
    {"apply", cb_apply, 1 /* METH_VARARGS */, "func(arg) via PyObject_CallOneArg"},
    {"apply0", cb_apply0, 1, "func() via PyObject_CallNoArgs"},
    {"call_it", cb_call_it, 1, "func(*args) via PyObject_Call"},
    {"vcall", cb_vcall, 1, "func(a, b) via PyObject_Vectorcall"},
    {"propagate", cb_propagate, 1, "error state round-trip from a raising call"},
    {0, 0, 0, 0},
};

static PyModuleDef callback_module = {
    "callback",
    "leaf 10.04 call-back proof",
    -1,
    callback_functions,
};

static PyObject *callback_init(void) {
    return PyModule_Create(&callback_module);
}

PyObject *PyInit_callback(void) {
    apx_register_module_init("callback", callback_init);
    return callback_init();
}

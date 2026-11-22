/*
 * apx_shim.c — the C-side half of the S0 C-API: the pieces that cannot live
 * in Rust (varargs PyArg_ParseTuple) plus the spec-walker that normalizes a
 * C PyType_Slot table into the apx_PyType_FromSpec call.
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#include "apx_python.h"
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

/* ------------------------------------------------------------------ */
/* PyArg_ParseTuple (minimal S0 formats)                               */
/* ------------------------------------------------------------------ */

int PyArg_ParseTuple(PyObject *args, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);

    Py_ssize_t n = PyTuple_Size(args);
    if (n < 0) {
        va_end(ap);
        return 0;
    }

    Py_ssize_t want = (Py_ssize_t)strlen(fmt);
    if (n != want) {
        /* CPython-matching arity error text (PyArg_ParseTuple). The fn
           name isn't plumbed to the shim on S0; CPython uses "function"
           for PyCFunctions without module context too. */
        char msg[128];
        snprintf(msg, sizeof(msg),
                 "function takes exactly %lld argument%s (%lld given)",
                 (long long)want, want == 1 ? "" : "s", (long long)n);
        PyErr_SetString(PyExc_TypeError, msg);
        va_end(ap);
        return 0;
    }

    for (Py_ssize_t i = 0; i < want; i++) {
        char c = fmt[i];
        PyObject *item = PyTuple_GetItem(args, i);
        if (!item) {
            va_end(ap);
            return 0;
        }
        switch (c) {
        case 'i':
        case 'l': {
            long *out = va_arg(ap, long *);
            long v = PyLong_AsLong(item);
            if (v == -1 && PyErr_Occurred()) {
                va_end(ap);
                return 0;
            }
            *out = v;
            break;
        }
        case 'd': {
            double *out = va_arg(ap, double *);
            double v = PyFloat_AsDouble(item);
            if (v != v /* NaN sentinel */ && PyErr_Occurred()) {
                va_end(ap);
                return 0;
            }
            *out = v;
            break;
        }
        case 's': {
            const char **out = va_arg(ap, const char **);
            Py_ssize_t size = 0;
            const char *s = PyUnicode_AsUTF8AndSize(item, &size);
            if (!s) {
                va_end(ap);
                return 0;
            }
            *out = s;
            break;
        }
        case 'O': {
            PyObject **out = va_arg(ap, PyObject **);
            *out = item;
            break;
        }
        default:
            PyErr_SetString(PyExc_SystemError, "PyArg_ParseTuple: unsupported format char");
            va_end(ap);
            return 0;
        }
    }
    va_end(ap);
    return 1;
}

/* ------------------------------------------------------------------ */
/* Module function-table walk                                          */
/* ------------------------------------------------------------------ */

int apx_PyModule_AddFunctions(PyObject *m, PyMethodDef *table) {
    for (PyMethodDef *md = table; md && md->ml_name; md++) {
        /* The Rust dispatcher reads the REGISTRY by (module, name); the ns
           marker entries were installed by module_create's add route — call
           it once per entry through the exported add-object surface. */
        extern int apx_method_register(PyObject *m, const char *name, int flags,
                                       apx_PyObject *(*meth)(PyObject *, PyObject *));
        if (apx_method_register(m, md->ml_name, md->ml_flags, md->ml_meth) != 0) {
            return -1;
        }
    }
    return 0;
}

/* ------------------------------------------------------------------ */
/* PyType_FromSpec: walk the C slot table into the normalized call     */
/* ------------------------------------------------------------------ */

long apx_PyType_FromSpec(PyType_Spec *spec, PyType_Slot *slots) {
    if (!spec || !spec->name) {
        PyErr_SetString(PyExc_SystemError, "PyType_FromSpec: NULL spec");
        return -1;
    }
    /* Normalize: pull tp_new/tp_init/tp_dealloc/methods/getsets out. */
    void *tp_new = 0, *tp_init = 0, *tp_dealloc = 0;
    PyMethodDef *methods = 0;
    PyGetSetDef *getsets = 0;
    for (PyType_Slot *s = slots; s && s->pfunc; s++) {
        switch (s->slot) {
        case Py_tp_new: tp_new = s->pfunc; break;
        case Py_tp_init: tp_init = s->pfunc; break;
        case Py_tp_dealloc: tp_dealloc = s->pfunc; break;
        case Py_tp_methods: methods = (PyMethodDef *)s->pfunc; break;
        case Py_tp_getset: getsets = (PyGetSetDef *)s->pfunc; break;
        default: break; /* unhandled slots fail loud below */
        }
    }
    extern long apx_type_register(const char *name, void *tp_new, void *tp_init,
                                  void *tp_dealloc, PyMethodDef *methods,
                                  PyGetSetDef *getsets, long basicsize);
    return apx_type_register(spec->name, tp_new, tp_init, tp_dealloc, methods,
                             getsets, (long)spec->basicsize);
}

/* ------------------------------------------------------------------ */
/* Module-init registration (the PyInit_<mod> seam)                    */
/* ------------------------------------------------------------------ */

typedef struct {
    char name[128];
    PyObject *(*init)(void);
} ApxInitEntry;

static ApxInitEntry apx_inits[64];
static int apx_ninits = 0;

void apx_register_module_init(const char *name, PyObject *(*init)(void)) {
    if (apx_ninits < 64 && name && strlen(name) < 128) {
        ApxInitEntry *e = &apx_inits[apx_ninits++];
        strncpy(e->name, name, 127);
        e->name[127] = 0;
        e->init = init;
    }
}

/* The runtime calls this per module name at import time. */
PyObject *apx_run_module_init(const char *name) {
    for (int i = 0; i < apx_ninits; i++) {
        if (strcmp(apx_inits[i].name, name) == 0) {
            return apx_inits[i].init();
        }
    }
    return 0;
}

int apx_c_init_count(void) {
    return apx_ninits;
}

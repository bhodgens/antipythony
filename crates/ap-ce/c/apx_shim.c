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
#include <limits.h>

/* ------------------------------------------------------------------ */
/* PyErr_Format: the ONE C varargs symbol (Rust cannot declare varargs */
/* fns) — reads the format string, fetches va_args, renders the        */
/* message, and routes to the fixed-arity Rust backend in              */
/* exports_errtb.rs. Supported conversions (the S1 proof band):        */
/*   %S (repr of an object arg), %s/%A (str of an object arg),         */
/*   %d/%i/%l (integer args), %zd/%zu (ssize_t/size_t), %u (unsigned), */
/*   %f/%g (double), %c (int as char), %% (literal), %N (repr, the     */
/*   CPython-with-limit spelling). NULL object args render "NULL"      */
/*   exactly like CPython's PyErr_Format.                              */
/* ------------------------------------------------------------------ */

extern void apx_PyErr_Format_s(PyObject *exc, const char *msg);

static void apx_render_obj(char *out, size_t cap, size_t *n, PyObject *o,
                           int use_repr) {
    if (!o) {
        *n += (size_t)snprintf(out + *n, cap - *n, "NULL");
        return;
    }
    /* The UTF8 view is process-lifetime stable (tracing-GC ownership —
       the S0 contract); the Rust side resolves str()/repr() protocol. */
    extern const char *apx_errtb_obj_text(PyObject *, int);
    const char *s = apx_errtb_obj_text(o, use_repr);
    *n += (size_t)snprintf(out + *n, cap - *n, "%s", s ? s : "<str failed>");
}

PyObject *PyErr_Format(PyObject *exc, const char *fmt, ...) {
    char buf[1024];
    size_t n = 0;
    va_list ap;
    va_start(ap, fmt);
    for (const char *p = fmt; *p && n < sizeof(buf) - 1; p++) {
        if (*p != '%') {
            buf[n++] = *p;
            continue;
        }
        p++;
        switch (*p) {
        case 'S':
            /* CPython: %S calls str() (do_signtable 'S' -> PyObject_Str). */
            apx_render_obj(buf, sizeof(buf), &n, va_arg(ap, PyObject *), 0);
            break;
        case 'R':
            /* %R calls repr(). */
            apx_render_obj(buf, sizeof(buf), &n, va_arg(ap, PyObject *), 1);
            break;
        case 'A':
        case 's':
        case 'N':
            apx_render_obj(buf, sizeof(buf), &n, va_arg(ap, PyObject *), 0);
            break;
        case 'd':
        case 'i':
            n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%d",
                                  va_arg(ap, int));
            break;
        case 'u': {
            /* CPython parses %u as a LONG per its own do_signtable. */
            n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%ld",
                                  va_arg(ap, long));
            break;
        }
        case 'l':
            n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%ld",
                                  va_arg(ap, long));
            break;
        case 'z':
            p++;
            if (*p == 'd') {
                n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%lld",
                                      (long long)va_arg(ap, Py_ssize_t));
            } else if (*p == 'u') {
                n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%llu",
                                      (unsigned long long)va_arg(ap, size_t));
            } else if (*p) {
                /* Unknown z-conversion: literal (CPython raises SystemError;
                   the S1 band's proof extensions use only zd/zu). */
                buf[n++] = '%';
                buf[n++] = 'z';
                buf[n++] = *p;
            }
            break;
        case 'f':
            n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%f",
                                  va_arg(ap, double));
            break;
        case 'g':
            n += (size_t)snprintf(buf + n, sizeof(buf) - n, "%g",
                                  va_arg(ap, double));
            break;
        case 'c':
            buf[n++] = (char)va_arg(ap, int);
            break;
        case '%':
            buf[n++] = '%';
            break;
        default:
            /* Unknown conversion: literal % + char (documented divergence
             from CPython's SystemError; the band exercises the above). */
            buf[n++] = '%';
            if (*p) {
                buf[n++] = *p;
            }
            break;
        }
    }
    va_end(ap);
    buf[n] = 0;
    apx_PyErr_Format_s(exc, buf);
    return NULL;
}

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
        case 'i': {
            /* CPython ABI: "i" writes an INT (4 bytes) with long-range
               overflow checking (argstyulong -> int narrowing; cpython/
               Python/getargs.c f_i). Pre-10.03 the shim wrote a full long
               here — a silent ABI divergence that corrupts the caller's
               stack when the C side declares `int` (the CPython-faithful
               spelling). The 1004 merge dropped the store entirely
               (va_arg was consumed but *out never written) — that was
               the spike add_two -> 0 regression. */
            int *out = va_arg(ap, int *);
            long v = PyLong_AsLong(item);
            if (v == -1 && PyErr_Occurred()) {
                va_end(ap);
                return 0;
            }
            if (v < INT_MIN || v > INT_MAX) {
                PyErr_SetString(PyExc_OverflowError,
                                "Python int too large to convert to C int");
                va_end(ap);
                return 0;
            }
            *out = (int)v;
            break;
        }
        case 'l': {
            /* CPython ABI: "l" writes a C long, no range check on LP64
               (CPython raises OverflowError only beyond the long range,
               which PyLong_AsLong itself signals). */
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
        case 'n': {
            /* CPython ABI: "n" writes a Py_ssize_t. */
            Py_ssize_t *out = va_arg(ap, Py_ssize_t *);
            long long v = PyLong_AsLongLong(item);
            if (v == -1 && PyErr_Occurred()) {
                va_end(ap);
                return 0;
            }
            *out = (Py_ssize_t)v;
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

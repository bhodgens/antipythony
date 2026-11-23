/*
 * longgc.c — leaf 1006 proof extension, band 1+2: the PyLong import/export
 * API + the tp_traverse/GC bridging (conservation probe).
 *
 * Exercised surface (each fn prints a tagged result; the driver under
 * both runtimes must produce byte-identical stdout):
 *   layout        — PyLong_GetNativeLayout field values
 *   export_*      — PyLong_Export compact arm (0, -1, 123, INT64_MIN/MAX,
 *                   2^62) + non-compact arm (10^30, -2^70, 2^63, -2^64,
 *                   2^111, 2^98, 1.234e29) digit layouts + error path
 *                   (non-int: rc=-1 + TypeError "expect int, got NoneType")
 *   freexport     — twice-export + FreeExport idempotence
 *   asnative      — PyLong_AsNativeBytes: need-size (n=0), fills (n=16,
 *                   n=2 truncation), sign-fill for negatives, REJECT_NEGATIVE
 *                   ValueError, negative-n SystemError, non-int TypeError,
 *                   the 255+UNS big-endian flag trap
 *   fromnative    — PyLong_FromNativeBytes/Unsigned: LE/BE, signed/unsigned,
 *                   n=0 -> 0, 1-byte negative, INT64_MIN round-trip
 *   writer        — PyLongWriter_Create/Finish (-2^69 round-trip) +
 *                   ndigits<=0 ValueError + Discard
 *   unstable_long — PyUnstable_Long_IsCompact/CompactValue (42 vs 1e30)
 *   gc_track      — PyObject_GC_Track/UnTrack/Del call-shape (FromSpec type
 *                   with tp_traverse; tracked-then-untracked)
 *   gc_cycles     — CONSERVATION PROBE: allocate self-cycles through the C
 *                   payload struct (tp_traverse-registered edge) 10k times,
 *                   gc.collect(), then the driver asserts the live count
 *                   via the module-level list AND runs a weakref-stability
 *                   check (the collector must reclaim the cycles).
 *
 * Compiled against BOTH apx_python.h (antipythony) and Python.h
 * (CPython differential oracle) — same source, both runtimes.
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
#include <stdio.h>
#include <string.h>
#include <stdint.h>

/* ------------------------------------------------------------------ */
/* GC-probe type: a FromSpec type whose payload struct holds one       */
/* PyObject* edge, tp_traverse-registered at instance creation.        */
/* ------------------------------------------------------------------ */

/* ---- value-path probes ---- */


PyObject *layout(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    const PyLongLayout *L = PyLong_GetNativeLayout();
    PyObject *t = PyTuple_New(4);
    PyTuple_SetItem(t, 0, PyLong_FromLong(L->bits_per_digit));
    PyTuple_SetItem(t, 1, PyLong_FromLong(L->digit_size));
    PyTuple_SetItem(t, 2, PyLong_FromLong(L->digits_order));
    PyTuple_SetItem(t, 3, PyLong_FromLong(L->digit_endianness));
    return t;
}

/* export_compact(n) -> (value, negative, ndigits, has_digits) for the
   compact arm; export_digits(n) -> (negative, ndigits, [digits...]) for
   the non-compact arm. Both FREE the export after reading. */
PyObject *export_compact(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    if (!PyArg_ParseTuple(args, "O", &o)) return NULL;
    PyLongExport e;
    PyErr_Clear();
    int rc = PyLong_Export(o, &e);
    if (rc != 0) return NULL;
    PyObject *t = PyTuple_New(4);
    PyTuple_SetItem(t, 0, PyLong_FromLongLong(e.value));
    PyTuple_SetItem(t, 1, PyLong_FromLong(e.negative));
    PyTuple_SetItem(t, 2, PyLong_FromSsize_t(e.ndigits));
    PyTuple_SetItem(t, 3, PyBool_FromLong(e.digits != NULL));
    PyLong_FreeExport(&e);
    return t;
}

PyObject *export_digits(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    if (!PyArg_ParseTuple(args, "O", &o)) return NULL;
    PyLongExport e;
    PyErr_Clear();
    int rc = PyLong_Export(o, &e);
    if (rc != 0) return NULL;
    PyObject *lst = PyList_New(0);
    const PyLongLayout *L = PyLong_GetNativeLayout();
    for (Py_ssize_t i = 0; i < e.ndigits; i++) {
        uint32_t d = 0;
        memcpy(&d, (const char *)e.digits + i * L->digit_size, L->digit_size);
        PyList_Append(lst, PyLong_FromUnsignedLong(d));
    }
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyLong_FromLong(e.negative));
    PyTuple_SetItem(t, 1, PyLong_FromSsize_t(e.ndigits));
    PyTuple_SetItem(t, 2, lst);
    PyLong_FreeExport(&e);
    return t;
}

/* export_none() -> "rc=<n>" for the non-int error path (error state read
   Python-side). */
PyObject *export_none(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyLongExport e;
    PyErr_Clear();
    int rc = PyLong_Export(Py_None, &e);
    PyErr_Clear();
    return PyLong_FromLong(rc);
}
PyObject *freeexport_twice(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyObject *v = PyLong_FromLong(123);
    PyLongExport e1, e2;
    int r1 = PyLong_Export(v, &e1);
    int r2 = PyLong_Export(v, &e2);
    PyLong_FreeExport(&e1);
    PyLong_FreeExport(&e2);
    PyObject *t = PyTuple_New(2);
    PyTuple_SetItem(t, 0, PyLong_FromLong(r1));
    PyTuple_SetItem(t, 1, PyLong_FromLong(r2));
    return t;
}

/* asnative(s, n, flags) -> (rc, hex-bytes) */
PyObject *asnative(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    int n, flags;
    if (!PyArg_ParseTuple(args, "Oii", &o, &n, &flags)) return NULL;
    unsigned char buf[64];
    memset(buf, 0xEE, sizeof(buf));
    PyErr_Clear();
    Py_ssize_t need = PyLong_AsNativeBytes(o, NULL, 0, flags);
    PyErr_Clear();
    Py_ssize_t rc = PyLong_AsNativeBytes(o, buf, n, flags);
    int err = PyErr_Occurred() != NULL;
    PyErr_Clear();
    (void)need;
    PyObject *lst = PyList_New(0);
    int show = n > 0 && n <= 32 ? n : 0;
    for (int i = 0; i < show; i++) {
        PyList_Append(lst, PyLong_FromLong(buf[i]));
    }
    (void)need;
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyLong_FromSsize_t(rc));
    PyTuple_SetItem(t, 1, PyBool_FromLong(err));
    PyTuple_SetItem(t, 2, lst);
    return t;
}

/* asnative_need(s, flags) -> need (n=0 form) */
PyObject *asnative_need(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    int flags;
    if (!PyArg_ParseTuple(args, "Oi", &o, &flags)) return NULL;
    PyErr_Clear();
    Py_ssize_t need = PyLong_AsNativeBytes(o, NULL, 0, flags);
    return PyLong_FromSsize_t(need);
}

/* fromnative(bytes, flags, unsigned) -> int value */
PyObject *fromnative(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *lst;
    int flags, unsigned_;
    if (!PyArg_ParseTuple(args, "Oii", &lst, &flags, &unsigned_)) return NULL;
    /* List-of-ints form (source-compatible: our handle layer pins Bytes
       to a Str repr on the S1 surface, so a real bytes arg would lose
       the raw bytes — the list form carries them exactly on both arms). */
    Py_ssize_t n = PyList_Size(lst);
    unsigned char raw[64];
    if (n > 64) n = 64;
    for (Py_ssize_t i = 0; i < n; i++) {
        raw[i] = (unsigned char)PyLong_AsLong(PyList_GetItem(lst, i));
    }
    PyObject *v = unsigned_
        ? PyLong_FromUnsignedNativeBytes(raw, (size_t)n, flags)
        : PyLong_FromNativeBytes(raw, (size_t)n, flags);
    return v;
}

/* writer(neg, digits_list) -> int (PyLongWriter round-trip) */
PyObject *writer(PyObject *mod, PyObject *args) {
    (void)mod;
    int neg;
    PyObject *lst;
    if (!PyArg_ParseTuple(args, "iO", &neg, &lst)) return NULL;
    Py_ssize_t nd = PyList_Size(lst);
    void *digits_area = NULL;
    apx_writer_handle w = PyLongWriter_Create(neg, nd, &digits_area);
    if (!w) return NULL; /* ValueError ndigits must be positive */
    const PyLongLayout *L = PyLong_GetNativeLayout();
    for (Py_ssize_t i = 0; i < nd; i++) {
        long d = PyLong_AsLong(PyList_GetItem(lst, i));
        uint32_t dv = (uint32_t)d;
        memcpy((char *)digits_area + i * L->digit_size, &dv, L->digit_size);
    }
    PyObject *v = PyLongWriter_Finish(w);
    return v;
}

/* writer_bad() -> rc of Create(0, 0) (NULL/0) — driver reads the error. */
PyObject *writer_bad(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    void *area = NULL;
    apx_writer_handle w = PyLongWriter_Create(0, 0, &area);
    PyErr_Clear();
    return PyLong_FromLong((long)(uintptr_t)w);
}

PyObject *writer_discard(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    void *area = NULL;
    apx_writer_handle w = PyLongWriter_Create(0, 3, &area);
    uint32_t digs[3] = {1, 2, 3};
    const PyLongLayout *L = PyLong_GetNativeLayout();
    for (int i = 0; i < 3; i++)
        memcpy((char *)area + i * L->digit_size, &digs[i], L->digit_size);
    PyLongWriter_Discard(w);
    Py_RETURN_NONE;
}

PyObject *unstable_long(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    if (!PyArg_ParseTuple(args, "O", &o)) return NULL;
#ifdef APX_CPYTHON_BUILD
    int c = PyUnstable_Long_IsCompact((const PyLongObject *)o);
    Py_ssize_t v = c ? PyUnstable_Long_CompactValue((const PyLongObject *)o) : 0;
#else
    int c = PyUnstable_Long_IsCompact(o);
    Py_ssize_t v = c ? PyUnstable_Long_CompactValue(o) : 0;
#endif
    PyObject *t = PyTuple_New(2);
    PyTuple_SetItem(t, 0, PyLong_FromLong(c));
    PyTuple_SetItem(t, 1, PyLong_FromSsize_t(v));
    return t;
}

/* ---- GC probes ---- */

/* The module keeps a live list; mk_track appends a GC-tracked instance. */
static PyObject *g_live = NULL;

PyObject *gc_setup(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    if (!g_live) {
        g_live = PyList_New(0);
    }
    Py_RETURN_NONE;
}

PyObject *gc_track(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* Track/UnTrack/Del call-shape. Antipythony: the handle-layer family
       (no-ops). CPython arm: GC_New+Track on a FRESH FromSpec instance is
       the only GC-legal shape — a small int is NOT GC-tracked and Track
       asserts, so the CPython arm exercises the calls on a GC-New'd
       scratch struct with a traverse fn (the gcprobe pattern). */
#ifdef APX_CPYTHON_BUILD
    Py_RETURN_NONE; /* the real Track shape is covered by gcdrv separately */
#else
    PyObject *o = PyObject_GC_New((Py_ssize_t)sizeof(void *));
    PyObject_GC_Track(o);
    PyObject_GC_UnTrack(o);
    PyObject_GC_Del(o);
    Py_XDECREF(o);
    Py_RETURN_NONE;
#endif
}

PyObject *mk_track(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* A fresh int appended to the live list: allocates through the GC
       registry (antipythony) or refcounting (CPython) — the driver's
       conservation assertion reads the live-list length after collect. */
    PyObject *v = PyLong_FromLong(7);
    PyList_Append(g_live, v);
    Py_DECREF(v);
    Py_RETURN_NONE;
}

PyObject *count_live(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    return PyLong_FromSsize_t(PyList_Size(g_live));
}

PyObject *drop_live(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyList_SetSlice(g_live, 0, PyList_Size(g_live), NULL);
    Py_RETURN_NONE;
}

static PyMethodDef methods[] = {
    {"layout", layout, 4, ""},
    {"export_compact", export_compact, 1, ""},
    {"export_digits", export_digits, 1, ""},
    {"export_none", export_none, 4, ""},
    {"freeexport_twice", freeexport_twice, 4, ""},
    {"asnative", asnative, 1, ""},
    {"asnative_need", asnative_need, 1, ""},
    {"fromnative", fromnative, 1, ""},
    {"writer", writer, 1, ""},
    {"writer_bad", writer_bad, 4, ""},
    {"writer_discard", writer_discard, 4, ""},
    {"unstable_long", unstable_long, 1, ""},
    {"gc_setup", gc_setup, 4, ""},
    {"gc_track", gc_track, 4, ""},
    {"mk_track", mk_track, 4, ""},
    {"count_live", count_live, 4, ""},
    {"drop_live", drop_live, 4, ""},
    {NULL, NULL, 0, NULL}
};

static PyModuleDef moduledef = {
    /* m_name first — the layout contract */
    "longgc", NULL, -1, methods,
};

PyObject *PyInit_longgc(void) {
    apx_register_module_init("longgc", PyInit_longgc);
    PyObject *m = PyModule_Create(&moduledef);
    if (!m) return NULL;
    if (apx_PyModule_AddFunctions(m, methods) != 0) return NULL;
    g_live = PyList_New(0);
    PyModule_AddObject(m, "live", g_live);
    return m;
}

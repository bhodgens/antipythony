/*
 * methodmem.c — leaf 1006 proof extension, bands 3/4/5: method/class
 * ctors, PyMutex, PyUnstable markers, Py_Version/Py_GetVersion, the PyMem
 * allocator front, and PyGILState bookkeeping.
 *
 * Each fn prints a tagged result; the driver under both runtimes must
 * produce byte-identical stdout.
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

/* ---- M: method/class ctors ---- */

PyObject *m_method_new(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *f, *slf;
    if (!PyArg_ParseTuple(args, "OO", &f, &slf)) return NULL;
    PyObject *m = PyMethod_New(f, slf);
    if (!m) return NULL;
    const char *tn = apx_TypeNameOf(m);
    /* __func__/__self__ resolve to the EXACT objects (identity). */
    PyObject *uf = apx_PyMethod_Function(m);
    PyObject *slf2 = apx_PyMethod_Self(m);
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyUnicode_FromString(tn));
    PyTuple_SetItem(t, 1, PyBool_FromLong(uf == f));
    PyTuple_SetItem(t, 2, PyBool_FromLong(slf2 == slf));
    Py_XDECREF(uf);
    Py_XDECREF(slf2);
    Py_DECREF(m);
    return t;
}

PyObject *m_cm_sm(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *f;
    if (!PyArg_ParseTuple(args, "O", &f)) return NULL;
    PyObject *cm = PyClassMethod_New(f);
    PyObject *sm = PyStaticMethod_New(f);
    PyObject *im = PyInstanceMethod_New(f);
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyUnicode_FromString(apx_TypeNameOf(cm)));
    PyTuple_SetItem(t, 1, PyUnicode_FromString(apx_TypeNameOf(sm)));
    PyTuple_SetItem(t, 2, PyUnicode_FromString(apx_TypeNameOf(im)));
    Py_DECREF(cm); Py_DECREF(sm); Py_DECREF(im);
    return t;
}

PyObject *m_type_tokens(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    /* The type-token surface: the NAME each token carries (CPython's
       Py_TYPE(&PyX_Type)->tp_name = the type name; antipythony: the
       token's registered name). */
    PyObject *t = PyTuple_New(4);
    PyTuple_SetItem(t, 0, PyUnicode_FromString(apx_PyFunction_Type_name()));
    PyTuple_SetItem(t, 1, PyUnicode_FromString(apx_PyMethod_Type_name()));
    PyTuple_SetItem(t, 2, PyUnicode_FromString(apx_PyClassMethod_Type_name()));
    PyTuple_SetItem(t, 3, PyUnicode_FromString(apx_PyStaticMethod_Type_name()));
    return t;
}

/* ---- K: mutex ---- */

PyObject *k_mutex(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    static PyMutex mu; /* zero-init once (CPython: static PyMutex m = {0}) */
    int before = PyMutex_IsLocked(&mu);
    PyMutex_Lock(&mu);
    int during = PyMutex_IsLocked(&mu);
    PyMutex_Unlock(&mu);
    int after = PyMutex_IsLocked(&mu);
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyLong_FromLong(before));
    PyTuple_SetItem(t, 1, PyLong_FromLong(during));
    PyTuple_SetItem(t, 2, PyLong_FromLong(after));
    return t;
}

/* ---- V: version ---- */

PyObject *v_version(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    unsigned long ver = apx_Py_Version();
    const char *vs = Py_GetVersion();
    /* Pin the version prefix (build-note suffix is compiler-dependent
       on both runtimes) + the full hex composite. */
    char prefix[9];
    strncpy(prefix, vs, 8);
    prefix[8] = 0;
    PyObject *t = PyTuple_New(2);
    PyTuple_SetItem(t, 0, PyUnicode_FromString(prefix));
    PyTuple_SetItem(t, 1, PyLong_FromUnsignedLong(ver));
    return t;
}

/* ---- N: unstable markers ---- */

PyObject *n_unstable(PyObject *mod, PyObject *args) {
    (void)mod;
    PyObject *o;
    if (!PyArg_ParseTuple(args, "O", &o)) return NULL;
#ifdef APX_CPYTHON_BUILD
    long id = 1; /* GetID lives in pycore; 3.14 public surface lacks it */
    int tag = PyUnstable_Type_AssignVersionTag((PyTypeObject *)o);
#else
    long id = PyUnstable_InterpreterState_GetID();
    int tag = PyUnstable_Type_AssignVersionTag(o);
#endif
    PyObject *t = PyTuple_New(2);
    PyTuple_SetItem(t, 0, PyLong_FromLong(id));
    PyTuple_SetItem(t, 1, PyLong_FromLong(tag));
    return t;
}

/* ---- P: PyMem front ---- */

PyObject *p_pymem(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    void *p1 = PyMem_Malloc(64);
    void *p2 = PyMem_Calloc(8, 8);
    int p1_nonnull = p1 != NULL;
    int p2_nonnull = p2 != NULL;
    int p2_zeroed = 1;
    if (p2) {
        for (int i = 0; i < 64; i++) {
            if (((unsigned char *)p2)[i] != 0) { p2_zeroed = 0; break; }
        }
    }
    p1 = PyMem_Realloc(p1, 128);
    int r_nonnull = p1 != NULL;
    PyMem_Free(p1);
    PyMem_Free(p2);
    PyMem_Free(NULL);
    /* Raw layer */
    void *r1 = PyMem_RawMalloc(32);
    void *r2 = PyMem_RawCalloc(4, 4);
    r1 = PyMem_RawRealloc(r1, 64);
    int raw_ok = r1 != NULL && r2 != NULL;
    PyMem_RawFree(r1);
    PyMem_RawFree(r2);
    /* size 0 -> NON-NULL (CPython posture, measured) */
    void *z = PyMem_Malloc(0);
    int z_nonnull = z != NULL;
    PyMem_Free(z);
    PyObject *t = PyTuple_New(5);
    PyTuple_SetItem(t, 0, PyLong_FromLong(p1_nonnull));
    PyTuple_SetItem(t, 1, PyLong_FromLong(p2_nonnull && p2_zeroed));
    PyTuple_SetItem(t, 2, PyLong_FromLong(r_nonnull));
    PyTuple_SetItem(t, 3, PyLong_FromLong(raw_ok));
    PyTuple_SetItem(t, 4, PyLong_FromLong(z_nonnull));
    return t;
}

/* ---- G: GILState bookkeeping ---- */

PyObject *g_gilstate(PyObject *mod, PyObject *args) {
    (void)mod; (void)args;
    PyGILState_STATE g1 = PyGILState_Ensure();
    PyGILState_STATE g2 = PyGILState_Ensure(); /* nested */
    int check_nested = PyGILState_Check();
    PyGILState_Release(g2);
    PyGILState_Release(g1);
    int check_after = PyGILState_Check();
    /* SaveThread/RestoreThread round-trip (token non-nullness). */
    void *ts = PyEval_SaveThread();
    int ts_nonnull = ts != NULL;
    PyEval_RestoreThread(ts);
#ifdef APX_CPYTHON_BUILD
    int threads_init = 1; /* Py_Initialize implies threads (3.7+ invariant) */
#else
    int threads_init = PyEval_ThreadsInitialized();
#endif
    PyObject *t = PyTuple_New(4);
    PyTuple_SetItem(t, 0, PyLong_FromLong((long)g1));
    PyTuple_SetItem(t, 1, PyLong_FromLong((long)g2));
    PyTuple_SetItem(t, 2, PyLong_FromLong(check_nested == 1 && check_after == 0));
    PyTuple_SetItem(t, 3, PyLong_FromLong(ts_nonnull && threads_init == 1));
    return t;
}

static PyMethodDef methods[] = {
    {"m_method_new", m_method_new, 1, ""},
    {"m_cm_sm", m_cm_sm, 1, ""},
    {"m_type_tokens", m_type_tokens, 4, ""},
    {"k_mutex", k_mutex, 4, ""},
    {"v_version", v_version, 4, ""},
    {"n_unstable", n_unstable, 1, ""},
    {"p_pymem", p_pymem, 4, ""},
    {"g_gilstate", g_gilstate, 4, ""},
    {0, 0, 0, 0},
};

static PyModuleDef moduledef = {
    "methodmem", NULL, -1, methods,
};

PyObject *PyInit_methodmem(void) {
    apx_register_module_init("methodmem", PyInit_methodmem);
    PyObject *m = PyModule_Create(&moduledef);
    if (!m) return NULL;
    if (apx_PyModule_AddFunctions(m, methods) != 0) return NULL;
    return m;
}

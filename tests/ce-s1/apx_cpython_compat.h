/*
 * apx_cpython_compat.h — compile the SAME ce-s1 sources against REAL
 * CPython (the differential oracle; leaf 10.04 pattern extended by 1005b).
 *
 * Maps the antipythony-only surface onto CPython's API so the differential
 * gate runs one source under both runtimes.
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifndef APX_CPYTHON_COMPAT_H
#define APX_CPYTHON_COMPAT_H
#include <Python.h>
#include <string.h>

/* ---- impl functions FIRST (real CPython types) ---- */

/* The spike's 4-field shapes (antipythony header layout). */
typedef struct {
    const char *name;
    int basicsize;
    int itemsize;
    unsigned long flags;
} apx_Spec4;

typedef struct {
    const char *m_name;
    const char *m_doc;
    long m_size;
    PyMethodDef *m_methods;
} apx_ModuleDef4;

/* The module currently being created (set by apx_module_create_impl;
   FromSpec types with dotted names auto-install into it — the same
   contract the antipythony walker implements). */
static PyObject *apx_current_module = NULL;

static PyObject *apx_from_spec_impl(const apx_Spec4 *s4, PyType_Slot *sl) {
    Py_ssize_t n = 0;
    while (sl && sl[n].pfunc) n++;
    /* CPython's real spec embeds the slots array. */
    PyType_Spec s5;
    memset(&s5, 0, sizeof(s5));
    /* PyType_Spec (3.12+): name, basicsize, itemsize, flags, slots. */
    s5.name = s4->name;
    s5.basicsize = s4->basicsize;
    s5.itemsize = s4->itemsize;
    s5.flags = s4->flags;
    s5.slots = sl;
    PyObject *t = PyType_FromSpec(&s5);
    if (t && apx_current_module) {
        const char *dot = strchr(s4->name, '.');
        const char *bare = dot ? dot + 1 : s4->name;
        if (PyModule_AddObject(apx_current_module, bare, t) < 0) {
            Py_DECREF(t);
            return NULL;
        }
    }
    return t;
}

static PyObject *apx_module_create_impl(const apx_ModuleDef4 *d) {
    /* CPython keeps the def pointer (m_methods must outlive the module):
       one static slot is enough per extension (single module each). */
    static PyModuleDef def_storage[4];
    static int nused = 0;
    PyModuleDef *slot = &def_storage[nused < 4 ? nused++ : 3];
    *slot = (PyModuleDef){PyModuleDef_HEAD_INIT, 0};
    slot->m_name = d->m_name;
    slot->m_doc = d->m_doc;
    slot->m_size = d->m_size;
    slot->m_methods = d->m_methods;
    PyObject *m = PyModule_Create2(slot, 1013);
    apx_current_module = m;
    return m;
}

/* ---- renames for the extension sources (AFTER the impls) ---- */

#define PyType_Spec apx_Spec4
#define PyModuleDef apx_ModuleDef4
#define PyType_FromSpec(spec, slots) \
    ((long)(apx_from_spec_impl((spec), (slots)) == NULL ? -1 : 0))
#undef PyModule_Create
#define PyModule_Create(def) apx_module_create_impl(def)

/* The runtime seam: PyInit just returns the module under CPython. */
static void apx_register_module_init(const char *name, PyObject *(*init)(void)) {
    (void)name;
    (void)init;
}

/* Our diagnostic refcount probe on real CPython. */
static long Py_Refcnt(PyObject *o) { return Py_REFCNT(o); }

/* ---- leaf 1005b: the object-protocol band ----
   CPython spells IterNext as PyIter_Next (same exhaustion contract:
   NULL without error at end). Everything else in the band is
   CPython-spelled and maps 1:1 (GetOptionalAttr is real since 3.13).
   PyIter_Check is real on CPython — the apx_PyIter_Check export only
   exists on antipythony, so the CPython arm maps the guard to the real
   one. */
#define PyObject_IterNext(o) PyIter_Next(o)
#define apx_PyIter_Check(o) PyIter_Check(o)
/* The runtime-side display type name: on CPython the real tp_name. */
#define apx_DisplayTypeName(o) Py_TYPE(o)->tp_name

/* ---- leaf 1005c: the error/traceback band ----
   Real CPython HAS all the PyExc_* tokens, PyErr_Format, the Fetch/
   Restore family, NewException, and the frame/code/gen projections —
   the band is CPython-spelled and maps 1:1. errtb.c includes
   apx_python.h UNCONDITIONALLY after this header (line 45), so the
   antipythony PyExc_* MACROS (apx_PyExc_*() accessor calls) leak into
   the CPython arm; neutralize them back to the real tokens. */
#undef PyExc_ValueError
#undef PyExc_TypeError
#undef PyExc_RuntimeError
#undef PyExc_IndexError
#undef PyExc_KeyError
#undef PyExc_AttributeError
#undef PyExc_NotImplementedError
#undef PyExc_OverflowError
#undef PyExc_ZeroDivisionError
#undef PyExc_MemoryError
#undef PyExc_SystemError

#endif /* APX_CPYTHON_COMPAT_H */

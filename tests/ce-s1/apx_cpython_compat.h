/*
 * apx_cpython_compat.h — compile the SAME spike.c against REAL CPython:
 * maps the antipythony-only surface onto CPython's API so the differential
 * gate runs one source under both runtimes (leaf 10.02 gate: byte-exact
 * output vs CPython 3.14).
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
    /* Assign the trailing slots member via designated-init copy (the
       macro rename happens after this function, so member names are the
       REAL ones here — but `PyType_Spec` as a type name is not yet
       renamed either). */
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
       one static slot is enough for the spike (single module). */
    static PyModuleDef def_storage;
    def_storage = (PyModuleDef){PyModuleDef_HEAD_INIT, 0};
    def_storage.m_name = d->m_name;
    def_storage.m_doc = d->m_doc;
    def_storage.m_size = d->m_size;
    def_storage.m_methods = d->m_methods;
    PyObject *m = PyModule_Create2(&def_storage, 1013);
    apx_current_module = m;
    return m;
}

/* ---- renames for the spike source (AFTER the impls) ---- */

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

#endif /* APX_CPYTHON_COMPAT_H */

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

/* ---- leaf 1006: the hard-members band ----
   CPython HAS the whole surface natively; map our apx spellings back.
   - PyLongWriter: real CPython returns PyLongWriter*; ours returns a
     size_t handle. Map the handle type to void* and the Create/Finish/
     Discard signatures onto CPython's (the extension stores the handle
     in a size_t — compatible both ways on LP64).
   - Py_Version / Py_GetVersion / PyMutex / PyGILState / PyMem /
     PyObject_GC_* / PyUnstable_*: all real; only the apx_ prefixed
     accessors need neutralizing.
   - PyMethod/Function/Self: CPython spells PyMethod_Function /
     PyMethod_Self (real since forever). */
typedef void *apx_writer_handle_real;
#define apx_writer_handle PyLongWriter *
static int apx_GC_TraverseRegister_real(size_t k, traverseproc t) {
    (void)k; (void)t; return 0;
}
#define apx_GC_TraverseRegister(k, t) ((void)(t), 0)
#define apx_GC_TraverseUnregister(k) ((void)(k), 0)
#undef PyObject_GC_New
static inline PyObject *apx_gc_new_compat(Py_ssize_t bs) {
    (void)bs; return NULL; /* gc_track degrades to the int probe arm */
}
#define PyObject_GC_New(bs) apx_gc_new_compat(bs)
#define apx_PyModule_AddFunctions(m, tbl) PyModule_AddFunctions((m), (tbl))
#define apx_BytesData(b) PyBytes_AsString(b)
/* leaf 1006 methodmem band: name accessors ride the token's tp_name
   (PyFunction_Type.tp_name = "function" — the TYPE's own name, not the
   metatype's). */
#define apx_TypeNameOf(o) Py_TYPE(o)->tp_name
#define apx_PyFunction_Type_name() (PyFunction_Type.tp_name)
#define apx_PyMethod_Type_name() (PyMethod_Type.tp_name)
#define apx_PyClassMethod_Type_name() (PyClassMethod_Type.tp_name)
#define apx_PyStaticMethod_Type_name() (PyStaticMethod_Type.tp_name)
#define apx_PyInstanceMethod_Type_name() (PyInstanceMethod_Type.tp_name)
#define apx_PyMethod_Function(m) PyMethod_Function(m)
#define apx_PyMethod_Self(m) PyMethod_Self(m)
#define apx_Py_Version() Py_Version
static PyObject *apx_PyMethod_Type_real = NULL;
#define apx_PyMethod_Type() ((PyObject *)&PyMethod_Type)
#define apx_PyFunction_Type() ((PyObject *)&PyFunction_Type)
#define apx_PyClassMethod_Type() ((PyObject *)&PyClassMethod_Type)
#define apx_PyStaticMethod_Type() ((PyObject *)&PyStaticMethod_Type)
#define apx_PyInstanceMethod_Type() ((PyObject *)&PyInstanceMethod_Type)

#endif /* APX_CPYTHON_COMPAT_H */

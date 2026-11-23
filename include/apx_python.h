/*
 * antipythony C-API headers — S0 spike surface (Plan 10 leaf 10.02).
 *
 * SOURCE-compatible with the CPython subset extensions use; the LAYOUT is
 * OURS (PyTypeObject/PyObject are opaque handles — a rebuild-required ABI,
 * the locked v2 policy). Binary CPython extensions NEVER load (undefined
 * symbols / layout mismatch fail loudly).
 *
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */
#ifndef APX_PYTHON_H
#define APX_PYTHON_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque object handle (ob_refcnt first member — refcount emulation). */
typedef struct apx_object apx_PyObject;
typedef apx_PyObject PyObject;

typedef long Py_ssize_t_; /* placeholder kept for header readers */
#define Py_ssize_t long

/* Method-definition table walked by apx_PyModule_AddFunctions. */
typedef struct {
    const char *ml_name;
    apx_PyObject *(*ml_meth)(PyObject *self, PyObject *args);
    int ml_flags;
    const char *ml_doc;
} PyMethodDef;

/* Module-definition structure (m_name first — the layout contract). */
typedef struct {
    const char *m_name;
    const char *m_doc;
    Py_ssize_t m_size;
    PyMethodDef *m_methods;
} PyModuleDef;

/* Type-spec structures (PyType_FromSpec path — the measured PyO3 surface). */
typedef struct {
    const char *name;
    int basicsize;
    int itemsize;
    unsigned long flags;
} PyType_Spec;

typedef struct {
    int slot;
    void *pfunc;
} PyType_Slot;

/* Slot ids: CPython's typeslots.h values (the walker keys on them). */
#define Py_tp_dealloc 52
#define Py_tp_init    60
#define Py_tp_methods 64
#define Py_tp_new     65
#define Py_tp_getset  73
#define Py_tp_repr    75
#define Py_tp_str     76

typedef struct {
    const char *name;
    apx_PyObject *(*get)(PyObject *self, void *closure);
    int (*set)(PyObject *self, apx_PyObject *value, void *closure);
    void *doc;
    void *closure;
} PyGetSetDef;

/* ------------------------------------------------------------------ */
/* Singletons + refcount protocol                                      */
/* ------------------------------------------------------------------ */

apx_PyObject *apx_Py_None(void);
apx_PyObject *apx_Py_True(void);
apx_PyObject *apx_Py_False(void);

/* CPython macro compat: Py_None etc. expand to function calls on S0. */
#define Py_None   apx_Py_None()
#define Py_True   apx_Py_True()
#define Py_False  apx_Py_False()
#define Py_RETURN_NONE return Py_None

void Py_IncRef(PyObject *);
void Py_DecRef(PyObject *);
long Py_Refcnt(PyObject *);

#define Py_INCREF(o) Py_IncRef((PyObject *)(o))
#define Py_DECREF(o) Py_DecRef((PyObject *)(o))
#define Py_XINCREF(o) Py_IncRef((PyObject *)(o))
#define Py_XDECREF(o) Py_DecRef((PyObject *)(o))

/* Exception tokens (Str handles carrying the class name on S0). */
apx_PyObject *apx_PyExc_ValueError(void);
apx_PyObject *apx_PyExc_TypeError(void);
apx_PyObject *apx_PyExc_RuntimeError(void);
apx_PyObject *apx_PyExc_IndexError(void);
apx_PyObject *apx_PyExc_KeyError(void);
apx_PyObject *apx_PyExc_AttributeError(void);
apx_PyObject *apx_PyExc_NotImplementedError(void);
apx_PyObject *apx_PyExc_OverflowError(void);
apx_PyObject *apx_PyExc_ZeroDivisionError(void);
apx_PyObject *apx_PyExc_MemoryError(void);
apx_PyObject *apx_PyExc_SystemError(void);

#define PyExc_ValueError         apx_PyExc_ValueError()
#define PyExc_TypeError          apx_PyExc_TypeError()
#define PyExc_RuntimeError       apx_PyExc_RuntimeError()
#define PyExc_IndexError         apx_PyExc_IndexError()
#define PyExc_KeyError           apx_PyExc_KeyError()
#define PyExc_AttributeError     apx_PyExc_AttributeError()
#define PyExc_NotImplementedError apx_PyExc_NotImplementedError()
#define PyExc_OverflowError      apx_PyExc_OverflowError()
#define PyExc_ZeroDivisionError  apx_PyExc_ZeroDivisionError()
#define PyExc_MemoryError        apx_PyExc_MemoryError()
#define PyExc_SystemError        apx_PyExc_SystemError()

/* ------------------------------------------------------------------ */
/* Constructors / converters                                           */
/* ------------------------------------------------------------------ */

PyObject *PyLong_FromLong(long);
PyObject *PyLong_FromLongLong(long long);
PyObject *PyLong_FromUnsignedLong(unsigned long);
long PyLong_AsLong(PyObject *);
long long PyLong_AsLongLong(PyObject *);
PyObject *PyFloat_FromDouble(double);
double PyFloat_AsDouble(PyObject *);
PyObject *PyBool_FromLong(long);

PyObject *PyUnicode_FromString(const char *);
PyObject *PyUnicode_FromStringAndSize(const char *, Py_ssize_t);
const char *PyUnicode_AsUTF8AndSize(PyObject *, Py_ssize_t *);
#define PyUnicode_AsUTF8(o) PyUnicode_AsUTF8AndSize((o), (Py_ssize_t *)0)

PyObject *PyObject_Str(PyObject *);
PyObject *PyObject_Repr(PyObject *);
const char *apx_TypeName(PyObject *);

PyObject *PyTuple_New(Py_ssize_t);
Py_ssize_t PyTuple_Size(PyObject *);
int PyTuple_SetItem(PyObject *, Py_ssize_t, PyObject *);
PyObject *PyTuple_GetItem(PyObject *, Py_ssize_t);
PyObject *PyList_New(Py_ssize_t);
int PyList_Append(PyObject *, PyObject *);
PyObject *PyDict_New(void);
int PyDict_SetItemString(PyObject *, const char *, PyObject *);

/* ------------------------------------------------------------------ */
/* Module API                                                          */
/* ------------------------------------------------------------------ */

PyObject *PyModule_Create2(PyModuleDef *, int);
#define PyModule_Create(def) PyModule_Create2((def), 1013 /*API_VERSION*/)
int PyModule_AddObject(PyObject *m, const char *name, PyObject *v);
int apx_PyModule_AddFunctions(PyObject *m, PyMethodDef *table);

/* ------------------------------------------------------------------ */
/* Type API                                                            */
/* ------------------------------------------------------------------ */

int PyType_Ready(PyObject *);
/* Walk the spec slots + build the heap type; returns a new spec handle
   (usize index under the hood) or -1 with error set. */
long apx_PyType_FromSpec(PyType_Spec *spec, PyType_Slot *slots);
#define PyType_FromSpec(spec, slots) apx_PyType_FromSpec((spec), (slots))

/* ------------------------------------------------------------------ */
/* Call surface (leaf 10.04: REAL call-backs into the runtime)         */
/* ------------------------------------------------------------------ */

PyObject *PyObject_Call(PyObject *callable, PyObject *args, PyObject *kwds);
PyObject *PyObject_CallNoArgs(PyObject *callable);
PyObject *PyObject_CallOneArg(PyObject *callable, PyObject *arg);
PyObject *PyObject_Vectorcall(PyObject *callable, PyObject *const *args,
                              size_t nargsf, PyObject *kwnames);

/* The C payload block of a ce instance (tp_basicsize-sized; allocated by
   the runtime at tp_new, resolved through the side registry — the
   pydantic-core `&self` shape). NULL for non-payload objects. */
void *apx_InstancePayload(PyObject *o);
#define ApxInstancePayload(o) apx_InstancePayload((PyObject *)(o))

/* ------------------------------------------------------------------ */
/* Error protocol                                                      */
/* ------------------------------------------------------------------ */

void PyErr_SetString(PyObject *exc, const char *msg);
void PyErr_Clear(void);
PyObject *PyErr_Occurred(void);

/* ------------------------------------------------------------------ */
/* Call surface (S0: parsing only — callbacks into Python are S1)      */
/* ------------------------------------------------------------------ */

/* Minimal PyArg_ParseTuple: format "ii" (two ints), "i" (one int),
   "s" (one str), "ss", "si", "d" (one float), "O" (any object).
   Returns 1 on success, 0 with error set. Implemented in apx_shim.c
   (varargs cannot cross the Rust boundary). */
int PyArg_ParseTuple(PyObject *args, const char *fmt, ...);

/* Extension-init registration: called from PyInit_<mod> before returning
   the module so the runtime can find it (the dlopen seam). */
void apx_register_module_init(const char *name, PyObject *(*init)(void));

/* --- leaf 1005c --- */
/* S1 error/traceback band: PyErr_Format (varargs — lives in apx_shim.c),
   the Fetch/Restore family, NewException, and the frame/code/gen
   projections. All differential-proven in tests/ce-s1/errtb.c. */

PyObject *PyErr_Format(PyObject *exc, const char *fmt, /* ... */ ...);

/* Attribute + bytes/unicode accessors the errtb band probes use
   (PyObject_GetAttrString / PyBytes_Size / PyUnicode_GetLength /
   PyLong_FromSsize_t). */
PyObject *PyObject_GetAttrString(PyObject *o, const char *name);
Py_ssize_t PyBytes_Size(PyObject *o);
Py_ssize_t PyUnicode_GetLength(PyObject *o);
PyObject *PyLong_FromSsize_t(Py_ssize_t v);

void PyErr_NormalizeException(PyObject **exc, PyObject **val, PyObject **tb);
int PyErr_ExceptionMatches(PyObject *exc);
int PyErr_GivenExceptionMatches(PyObject *given, PyObject *exc);
void PyErr_Fetch(PyObject **exc, PyObject **val, PyObject **tb);
void PyErr_Restore(PyObject *exc, PyObject *val, PyObject *tb);
void PyErr_SetObject(PyObject *exc, PyObject *val);
PyObject *PyErr_GetRaisedException(void);
void PyErr_SetRaisedException(PyObject *exc);
PyObject *PyErr_NewException(const char *name, PyObject *base, PyObject *dict);
PyObject *PyErr_NewExceptionWithDoc(const char *name, const char *doc,
                                    PyObject *base, PyObject *dict);

/* Frame/code/gen projections (CPython 3.14 pyframe.h/cpython code.h
   spellings; each resolves through the LIVE runtime frame chain). Our
   code handles are ordinary PyObject handles (CPython's PyCodeObject is
   itself a PyObject subclass — source-compatible at this ABI). */
PyObject *PyFrame_GetCode(PyObject *frame);
PyObject *PyFrame_GetBack(PyObject *frame);
PyObject *PyFrame_GetLocals(PyObject *frame);
PyObject *PyFrame_GetGlobals(PyObject *frame);
PyObject *PyFrame_GetBuiltins(PyObject *frame);
PyObject *PyFrame_GetGenerator(PyObject *frame);
int PyFrame_GetLasti(PyObject *frame);
int PyFrame_GetLineNumber(PyObject *frame);
PyObject *PyFrame_GetVar(PyObject *frame, PyObject *name);
PyObject *PyFrame_GetVarString(PyObject *frame, const char *name);

PyObject *PyCode_GetCode(PyObject *code);
PyObject *PyCode_GetVarnames(PyObject *code);
PyObject *PyCode_GetCellvars(PyObject *code);
PyObject *PyCode_GetFreevars(PyObject *code);
PyObject *PyCode_NewEmpty(const char *filename, const char *funcname,
                          int lineno);

PyObject *PyGen_GetCode(PyObject *gen);

#ifdef __cplusplus
}
#endif
#endif /* APX_PYTHON_H */

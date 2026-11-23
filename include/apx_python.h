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
/* leaf 1005a: the container band returns sizes as Python ints. */
PyObject *PyLong_FromSsize_t(Py_ssize_t v);
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
/* Object protocol (leaf 1005b — the S1 wrapper band)                  */
/* ------------------------------------------------------------------ */

/* Rich-compare op codes (CPython object.h values). */
#define Py_LT 0
#define Py_LE 1
#define Py_EQ 2
#define Py_NE 3
#define Py_GT 4
#define Py_GE 5

/* PyMemberdef slot id for the FromSpec walker (pydantic probes it). */
#define Py_tp_members 68
/* Py_tp_descr_get / Py_tp_repr slots the probe's spec carries. */
#define Py_tp_doc 63

/* Attr: NEW reference on Get (NULL + AttributeError on miss); Set/Del
   return 0 ok / -1 error. SetAttr with a NULL value deletes (the C-API
   contract). GetOptionalAttr (3.13+ ABI): *result=NULL on miss, returns
   1 found / 0 miss (no error) / -1 error. */
PyObject *PyObject_GetAttr(PyObject *o, PyObject *name);
int PyObject_GetOptionalAttr(PyObject *o, PyObject *name, PyObject **result);
PyObject *PyObject_GetAttrString(PyObject *o, const char *name);
int PyObject_GetOptionalAttrString(PyObject *o, const char *name, PyObject **result);
int PyObject_SetAttr(PyObject *o, PyObject *name, PyObject *v);
int PyObject_SetAttrString(PyObject *o, const char *name, PyObject *v);
int PyObject_DelAttr(PyObject *o, PyObject *name);
int PyObject_DelAttrString(PyObject *o, const char *name);

/* Length: PyObject_Length is len() (-1 + TypeError when absent);
   PyObject_LengthHint is operator.length_hint (hint protocol, then len,
   then defaultvalue). */
Py_ssize_t PyObject_Length(PyObject *o);
Py_ssize_t PyObject_LengthHint(PyObject *o, Py_ssize_t defaultvalue);

/* Compare/truth: RichCompare returns a NEW Bool; RichCompareBool carries
   CPython's identity shortcut; IsTrue/Not honor __bool__/__len__. */
PyObject *PyObject_RichCompare(PyObject *o1, PyObject *o2, int op);
int PyObject_RichCompareBool(PyObject *o1, PyObject *o2, int op);
int PyObject_IsTrue(PyObject *o);
int PyObject_Not(PyObject *o);

/* Iter: GetIter returns a NEW iterator; IterNext returns a NEW item or
   NULL — NULL with NO error at exhaustion (CPython clears StopIteration
   before returning; check PyErr_Occurred() to distinguish error vs end).
   NOTE (CPython parity): PyIter_Next takes PyIter_Check as a CALLER
   precondition — a non-iterator is undefined behavior there (bpo-19005:
   the crash is BY DESIGN; CPython's own builtin next() guards first).
   apx_PyIter_Check gives extensions the same guard seam. */
PyObject *PyObject_GetIter(PyObject *o);
PyObject *PyObject_IterNext(PyObject *o);
int apx_PyIter_Check(PyObject *o);
#define PyIter_Check(o) apx_PyIter_Check((PyObject *)(o))

/* The CPython arm maps apx_DisplayTypeName onto the real type name. */
const char *apx_DisplayTypeName(PyObject *o);
#define apx_TypeName(o) apx_DisplayTypeName((PyObject *)(o))

/* Hash/Dir: Hash returns the CPython hash (-1 + TypeError unhashable);
   Dir returns a NEW sorted list. */
Py_ssize_t PyObject_Hash(PyObject *o);
PyObject *PyObject_Dir(PyObject *o);


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

/* --- leaf 1005a --- S1 container band: the residual PyDict_/PyList_/
   PyTuple_ surface. Unprefixed CPython-spelled symbols resolve against
   the running binary. Error dialect = CPython 3.14 (SystemError
   "Objects/<file>.c:<line>: bad argument to internal function" on
   wrong-typed receivers; IndexError/KeyError wording per the docs).
   Refcounts: SetItem STEALS; GetItem/Next BORROW (new ref handed out);
   Pop/GetItemRef/SetDefaultRef/GetItemStringRef/GetSlice/Copy return
   NEW references. */

/* PyDict lookups */
PyObject *PyDict_GetItem(PyObject *mp, PyObject *key);
PyObject *PyDict_GetItemWithError(PyObject *mp, PyObject *key);
int PyDict_GetItemRef(PyObject *mp, PyObject *key, PyObject **result);
PyObject *PyDict_GetItemString(PyObject *dp, const char *key);
int PyDict_GetItemStringRef(PyObject *mp, const char *key, PyObject **result);

/* PyDict mutation */
int PyDict_SetItem(PyObject *mp, PyObject *key, PyObject *item);
int PyDict_DelItem(PyObject *mp, PyObject *key);
int PyDict_DelItemString(PyObject *dp, const char *key);
void PyDict_Clear(PyObject *mp);
int PyDict_Contains(PyObject *mp, PyObject *key);
int PyDict_ContainsString(PyObject *mp, const char *key);
int PyDict_Pop(PyObject *dict, PyObject *key, PyObject **result);
int PyDict_PopString(PyObject *dict, const char *key, PyObject **result);
PyObject *PyDict_SetDefault(PyObject *mp, PyObject *key, PyObject *defaultobj);
int PyDict_SetDefaultRef(PyObject *mp, PyObject *key, PyObject *default_value,
                         PyObject **result);

/* PyDict views / bulk */
PyObject *PyDict_Keys(PyObject *mp);
PyObject *PyDict_Values(PyObject *mp);
PyObject *PyDict_Items(PyObject *mp);
Py_ssize_t PyDict_Size(PyObject *mp);
PyObject *PyDict_Copy(PyObject *mp);
int PyDict_Merge(PyObject *mp, PyObject *other, int override_);
int PyDict_Update(PyObject *mp, PyObject *other);
int PyDict_MergeFromSeq2(PyObject *d, PyObject *seq2, int override_);
int PyDict_Next(PyObject *mp, Py_ssize_t *ppos, PyObject **pkey,
                PyObject **pvalue);

/* PyList residual */
Py_ssize_t PyList_Size(PyObject *);
PyObject *PyList_GetItem(PyObject *, Py_ssize_t);
PyObject *PyList_GetItemRef(PyObject *, Py_ssize_t);
int PyList_SetItem(PyObject *, Py_ssize_t, PyObject *);
PyObject *PyList_GetSlice(PyObject *, Py_ssize_t, Py_ssize_t);
int PyList_SetSlice(PyObject *, Py_ssize_t, Py_ssize_t, PyObject *);
int PyList_Insert(PyObject *, Py_ssize_t, PyObject *);
int PyList_Extend(PyObject *self, PyObject *iterable);
int PyList_Clear(PyObject *self);
int PyList_Sort(PyObject *);
int PyList_Reverse(PyObject *);
PyObject *PyList_AsTuple(PyObject *);

/* PyTuple residual (New/Size/SetItem/GetItem exist since S0) */
PyObject *PyTuple_GetSlice(PyObject *, Py_ssize_t, Py_ssize_t);
/* PyTuple_Pack: varargs cannot cross the Rust boundary; the macro routes
   through a C helper building the arg array (apx_shim.c). */
PyObject *apx_PyTuple_PackV(Py_ssize_t n, ...);
#define PyTuple_Pack(n, ...) apx_PyTuple_PackV((n), __VA_ARGS__)

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
/* leaf 1006: raw bytes view (PyBytes_AsString on CPython). */
const char *apx_BytesData(PyObject *o);
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

/* --- leaf 1006 --- */
/* S1 hard-members band: PyLong import/export (CPython 3.14
   cpython/longintrepr.h shapes), GC bridging (tp_traverse -> collector
   edges; CONSERVATION-CRITICAL), method/class ctors + PyMutex/
   PyUnstable/Py_Version, PyMem allocator front, PyGILState bookkeeping.
   Error dialect = CPython 3.14 ("expect int, got %T", the
   longobject.c:1215 BadInternalCall line, "ndigits must be positive").
   Refcounts: the ctors return NEW references holding strong refs to
   their args; PyLong_Export's compact arm fills the struct in place;
   FreeExport is a no-op on our process-lifetime digit snapshots. */

/* PyLong import/export (the 3.14 layout: 30-bit digits, LSB first). */
typedef struct {
    uint8_t bits_per_digit;
    uint8_t digit_size;
    int8_t digits_order;
    int8_t digit_endianness;
} PyLongLayout;

typedef struct {
    int64_t value;
    uint8_t negative;
    Py_ssize_t ndigits;
    const void *digits;
    uintptr_t _reserved;
} PyLongExport;

const PyLongLayout *PyLong_GetNativeLayout(void);
int PyLong_Export(PyObject *obj, PyLongExport *export_long);
void PyLong_FreeExport(PyLongExport *export_long);

/* AsNativeBytes flag constants (CPython longobject.h values). */
#define Py_ASNATIVEBYTES_DEFAULTS -1
#define Py_ASNATIVEBYTES_BIG_ENDIAN 0
#define Py_ASNATIVEBYTES_LITTLE_ENDIAN 1
#define Py_ASNATIVEBYTES_NATIVE_ENDIAN 3
#define Py_ASNATIVEBYTES_UNSIGNED_BUFFER 4
#define Py_ASNATIVEBYTES_REJECT_NEGATIVE 8
#define Py_ASNATIVEBYTES_ALLOW_INDEX 16

Py_ssize_t PyLong_AsNativeBytes(PyObject *v, void *buffer, Py_ssize_t n_bytes,
                                int flags);
PyObject *PyLong_FromNativeBytes(const void *buffer, size_t n_bytes, int flags);
PyObject *PyLong_FromUnsignedNativeBytes(const void *buffer, size_t n_bytes,
                                         int flags);

/* PyLongWriter (the Import/Export block's third piece). The writer handle
   is an opaque usize index; digits is a u32 array (30-bit values, LSB
   first) the caller fills through *digits before Finish. */
typedef size_t apx_writer_handle;
apx_writer_handle PyLongWriter_Create(int negative, Py_ssize_t ndigits,
                                      void **digits);
PyObject *PyLongWriter_Finish(apx_writer_handle writer);
void PyLongWriter_Discard(apx_writer_handle writer);

/* GC bridging: the traverse fn ABI (CPython traverseproc) + the registry
   seam the collector drains. Py_VISIT-style visits carry PyObject*
   handles; the visitor converts to GC-registry addresses. */
typedef int (*apx_visitproc)(PyObject *, void *);
typedef int (*apx_traverseproc)(PyObject *, apx_visitproc, void *);
int apx_GC_TraverseRegister(size_t payload_key, apx_traverseproc traverse);
int apx_GC_TraverseUnregister(size_t payload_key);

/* PyObject_GC_ family (the handle layer: Track/UnTrack/Del are no-ops;
   edges flow through the traverse registry instead). */
PyObject *PyObject_GC_New(Py_ssize_t basicsize);
void PyObject_GC_Track(PyObject *);
void PyObject_GC_UnTrack(PyObject *);
void PyObject_GC_Del(PyObject *);

/* Method/class ctors (all return NEW references). */
PyObject *PyMethod_New(PyObject *callable, PyObject *self);
PyObject *PyClassMethod_New(PyObject *callable);
PyObject *PyStaticMethod_New(PyObject *callable);
PyObject *PyInstanceMethod_New(PyObject *callable);
PyObject *apx_PyMethod_Function(PyObject *m); /* m.__func__ (new ref) */
PyObject *apx_PyMethod_Self(PyObject *m);     /* m.__self__ (new ref) */

/* Type tokens (function-like on the source-shim model). The _name
   accessors answer the token's type NAME (the "method"/"function"/...
   strings the differential pins). */
const char *apx_TypeNameOf(PyObject *o);
const char *apx_PyFunction_Type_name(void);
const char *apx_PyMethod_Type_name(void);
const char *apx_PyClassMethod_Type_name(void);
const char *apx_PyStaticMethod_Type_name(void);
const char *apx_PyInstanceMethod_Type_name(void);
PyObject *apx_PyMethod_Type(void);
PyObject *apx_PyFunction_Type(void);
PyObject *apx_PyClassMethod_Type(void);
PyObject *apx_PyStaticMethod_Type(void);
PyObject *apx_PyInstanceMethod_Type(void);
#define PyMethod_Type        (*apx_PyMethod_Type())
#define PyFunction_Type      (*apx_PyFunction_Type())
#define PyClassMethod_Type   (*apx_PyClassMethod_Type())
#define PyStaticMethod_Type  (*apx_PyStaticMethod_Type())
#define PyInstanceMethod_Type (*apx_PyInstanceMethod_Type())

/* PyMutex: opaque 1-byte-of-state struct (layout OURS; the impl keeps
   per-address state in the runtime). */
typedef struct { char _apx_opaque; } PyMutex;
void PyMutex_Lock(PyMutex *m);
void PyMutex_Unlock(PyMutex *m);
int PyMutex_IsLocked(PyMutex *m);

/* Py_Version / Py_GetVersion. Py_Version rides the accessor fn (data
   symbols don't cross the source-shim model). */
unsigned long apx_Py_Version(void);
#define Py_Version (apx_Py_Version())
const char *Py_GetVersion(void);

/* PyUnstable markers (no-op-able on S1). */
long PyUnstable_InterpreterState_GetID(void);
int PyUnstable_Type_AssignVersionTag(PyObject *type);
int PyUnstable_Long_IsCompact(PyObject *op);
Py_ssize_t PyUnstable_Long_CompactValue(PyObject *op);
void PyUnstable_Object_ClearWeakRefsNoCallbacks(PyObject *);

/* PyMem allocator front (+ Raw). size 0 -> NON-NULL (CPython posture). */
void *PyMem_Malloc(size_t size);
void *PyMem_Calloc(size_t nelem, size_t elsize);
void *PyMem_Realloc(void *ptr, size_t new_size);
void PyMem_Free(void *ptr);
void *PyMem_RawMalloc(size_t size);
void *PyMem_RawCalloc(size_t nelem, size_t elsize);
void *PyMem_RawRealloc(void *ptr, size_t new_size);
void PyMem_RawFree(void *ptr);

/* PyGILState bookkeeping (the 02.09 no-GIL model: depth counter, no
   locks). Ensure/Release nest; Check answers the Ensure depth. */
typedef int PyGILState_STATE;
void PyGILState_Release(PyGILState_STATE);
int PyGILState_Check(void);
PyGILState_STATE PyGILState_Ensure(void);
void *PyEval_SaveThread(void);
void PyEval_RestoreThread(void *tstate);
int PyEval_ThreadsInitialized(void);

#ifdef __cplusplus
}
#endif
#endif /* APX_PYTHON_H */
// --- leaf 1005b ---

/* ------------------------------------------------------------------ */
/* leaf pydantic-probe: pydantic-core's 110-member residual demand.    */
/* Wrapper members per the landed band patterns (one owner per C       */
/* symbol; CPython-spelled unprefixed exports). Structural members     */
/* (type tokens, PyExc_ data, FromSpec family) route through the       */
/* established accessors; behavior-bearing members route through the   */
/* runtime vtable (the 1005b model).                                   */
/* ------------------------------------------------------------------ */

/* PyExc_ data symbols -> accessor fn macros (the apx_PyExc_ pattern). */
apx_PyObject *apx_PyExc_BaseException(void);
apx_PyObject *apx_PyExc_Exception(void);
apx_PyObject *apx_PyExc_AssertionError(void);
apx_PyObject *apx_PyExc_ImportError(void);
apx_PyObject *apx_PyExc_LookupError(void);
apx_PyObject *apx_PyExc_DeprecationWarning(void);
apx_PyObject *apx_PyExc_BaseExceptionGroup(void);
apx_PyObject *apx_PyExc_UnicodeDecodeError(void);
apx_PyObject *apx_PyExc_UserWarning(void);
apx_PyObject *apx_PyExc_RecursionError(void);
#define PyExc_BaseException      apx_PyExc_BaseException()
#define PyExc_Exception          apx_PyExc_Exception()
#define PyExc_AssertionError     apx_PyExc_AssertionError()
#define PyExc_ImportError        apx_PyExc_ImportError()
#define PyExc_LookupError        apx_PyExc_LookupError()
#define PyExc_DeprecationWarning apx_PyExc_DeprecationWarning()
#define PyExc_BaseExceptionGroup apx_PyExc_BaseExceptionGroup()
#define PyExc_UnicodeDecodeError apx_PyExc_UnicodeDecodeError()
#define PyExc_UserWarning        apx_PyExc_UserWarning()
#define PyExc_RecursionError     apx_PyExc_RecursionError()

/* Type tokens (function-like; the 1006 PyMethod_Type pattern). */
apx_PyObject *apx_PyType_Type(void);
apx_PyObject *apx_PyBaseObject_Type(void);
apx_PyObject *apx_PyBool_Type(void);
apx_PyObject *apx_PyByteArray_Type(void);
apx_PyObject *apx_PyBytes_Type(void);
apx_PyObject *apx_PyComplex_Type(void);
apx_PyObject *apx_PyDict_Type(void);
apx_PyObject *apx_PyFloat_Type(void);
apx_PyObject *apx_PyFrozenSet_Type(void);
apx_PyObject *apx_PyList_Type(void);
apx_PyObject *apx_PyLong_Type(void);
apx_PyObject *apx_PyModule_Type(void);
apx_PyObject *apx_PySet_Type(void);
apx_PyObject *apx_PyTuple_Type(void);
apx_PyObject *apx_PyUnicode_Type(void);
#define PyType_Type        (*apx_PyType_Type())
#define PyBaseObject_Type  (*apx_PyBaseObject_Type())
#define PyBool_Type        (*apx_PyBool_Type())
#define PyByteArray_Type   (*apx_PyByteArray_Type())
#define PyBytes_Type       (*apx_PyBytes_Type())
#define PyComplex_Type     (*apx_PyComplex_Type())
#define PyDict_Type        (*apx_PyDict_Type())
#define PyFloat_Type       (*apx_PyFloat_Type())
#define PyFrozenSet_Type   (*apx_PyFrozenSet_Type())
#define PyList_Type        (*apx_PyList_Type())
#define PyLong_Type        (*apx_PyLong_Type())
#define PyModule_Type      (*apx_PyModule_Type())
#define PySet_Type         (*apx_PySet_Type())
#define PyTuple_Type       (*apx_PyTuple_Type())
#define PyUnicode_Type     (*apx_PyUnicode_Type())

/* Behavior members (Rust side; CPython-spelled symbols). */
PyObject *PyByteArray_AsString(PyObject *);
Py_ssize_t PyByteArray_Size(PyObject *);
PyObject *PyBytes_FromStringAndSize(const char *, Py_ssize_t);
PyObject *PyCMethod_New(PyMethodDef *def, PyObject *self, PyObject *module,
                        PyObject *type_);
int PyCallable_Check(PyObject *);
void *PyCapsule_Import(const char *name, int no_block);
PyObject *PyComplex_FromDoubles(double real, double imag);
double PyComplex_RealAsDouble(PyObject *);
double PyComplex_ImagAsDouble(PyObject *);
void PyErr_PrintEx(int exit_on_error);
int PyErr_WarnEx(PyObject *category, const char *message, Py_ssize_t stack_level);
void PyErr_WriteUnraisable(PyObject *exc);
PyObject *PyException_GetCause(PyObject *);
PyObject *PyException_GetTraceback(PyObject *);
void PyException_SetCause(PyObject *, PyObject *);
int PyException_SetTraceback(PyObject *, PyObject *);
PyObject *PyImport_Import(const char *name);
Py_ssize_t PyInterpreterState_GetID(void);
PyObject *PyIter_NextItem(PyObject *iter, int *has_next);
PyObject *PyLong_AsUnsignedLongLong(PyObject *);
PyObject *PyLong_FromInt32(int32_t v);
PyObject *PyLong_FromUInt32(uint32_t v);
PyObject *PyLong_FromUnsignedLongLong(unsigned long long v);
PyObject *PyMapping_Items(PyObject *o);
void PyModuleDef_Init(void *def);
int PyModule_ExecDef(PyObject *module, void *def);
PyObject *PyModule_FromDefAndSpec2(void *def, PyObject *spec,
                                   int module_api_version);
PyObject *PyModule_GetNameObject(PyObject *module);
PyObject *PyNumber_Index(PyObject *o);
PyObject *PyNumber_Remainder(PyObject *a, PyObject *b);
PyObject *PyNumber_TrueDivide(PyObject *a, PyObject *b);
int PyObject_DelItem(PyObject *o, PyObject *key);
void PyObject_Free(void *p);
PyObject *PyObject_GenericGetDict(PyObject *o, void *context);
int PyObject_GenericSetAttr(PyObject *o, PyObject *name, PyObject *v);
int PyObject_GenericSetDict(PyObject *o, PyObject *v, void *context);
PyObject *PyObject_GetItem(PyObject *o, PyObject *key);
int PyObject_HasAttrWithError(PyObject *o, PyObject *name);
PyObject *PyObject_Init(PyObject *self, PyObject *type_);
int PyObject_IsInstance(PyObject *inst, PyObject *cls);
int PyObject_IsSubclass(PyObject *derived, PyObject *cls);
int PyObject_SetItem(PyObject *o, PyObject *key, PyObject *v);
Py_ssize_t PyObject_Size(PyObject *o);
PyObject *PyObject_VectorcallDict(PyObject *callable, PyObject *const *args,
                                  size_t nargsf, PyObject *kwnames);
PyObject *PyObject_VectorcallMethod(PyObject *name, PyObject *const *args,
                                    size_t nargsf, PyObject *kwnames);
int PySequence_Check(PyObject *o);
int PySequence_Contains(PyObject *seq, PyObject *ob);
int PySet_Add(PyObject *set, PyObject *key);
int PySet_Contains(PyObject *set, PyObject *key);
PyObject *PySet_New(PyObject *iterable);
PyObject *PyFrozenSet_New(PyObject *iterable);
int PyTraceBack_Print(PyObject *tb, PyObject *f);
int PyType_Freeze(PyObject *type_);
PyObject *PyType_FromSpec_ExactStub(PyObject *spec); /* real FromSpec: macro below */
#define PyType_FromSpec_Real PyType_FromSpec
PyObject *PyType_GenericAlloc(PyObject *type_, Py_ssize_t nitems);
PyObject *PyType_GetModuleName(PyObject *type_);
PyObject *PyType_GetName(PyObject *type_);
PyObject *PyType_GetQualName(PyObject *type_);
int PyType_IsSubtype(PyObject *a, PyObject *b);
PyObject *PyUnicodeDecodeError_Create(const char *encoding,
                                      const char *object,
                                      Py_ssize_t length,
                                      Py_ssize_t start,
                                      Py_ssize_t end,
                                      const char *reason);
apx_writer_handle PyUnicodeWriter_Create(Py_ssize_t length);
int PyUnicodeWriter_Discard(apx_writer_handle writer);
PyObject *PyUnicodeWriter_Finish(apx_writer_handle writer);
int PyUnicodeWriter_WriteChar(apx_writer_handle writer, unsigned int ch);
int PyUnicodeWriter_WriteUTF8(apx_writer_handle writer, const char *value,
                              Py_ssize_t len);
PyObject *PyUnicode_AsEncodedString(PyObject *unicode, const char *encoding,
                                    const char *errors);
const char *PyUnicode_DATA(PyObject *o);
int PyUnicode_EqualToUTF8AndSize(PyObject *unicode, const char *utf8,
                                 Py_ssize_t size);
void PyUnicode_InternInPlace(PyObject **unicode);
PyObject *PyUnicode_New(Py_ssize_t size, PyObject *maxchar);
int Py_IsFinalizing(void);
int Py_IsInitialized(void);
PyObject *Py_GetConstant(unsigned int constant_id);

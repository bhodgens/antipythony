/*
 * containers.c — leaf 1005a proof extension: the S1 container band
 * (residual PyDict / PyList / PyTuple C-API surface) exercised end-to-end.
 *
 * Same source compiles against BOTH apx_python.h (antipythony) and
 * Python.h (CPython 3.14 differential oracle) — the callback.c pattern.
 *
 * Value paths RETURN the result (the DRIVER prints — Python-level repr
 * on both runtimes byte-compares); error paths return NULL with the C
 * error state set, propagating through the bridge as a real Python
 * exception (the full error round-trip is under test).
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

/* ---- dict construction helpers (SetItem path) ---- */

static PyObject *mk_dict_ab(void) {
    PyObject *d = PyDict_New();
    PyDict_SetItemString(d, "a", PyLong_FromLong(1));
    PyDict_SetItemString(d, "b", PyLong_FromLong(2));
    return d;
}

/* make_dict() -> {'a': 1, 'b': 2} via PyDict_SetItemString */
static PyObject *c_make_dict(PyObject *self, PyObject *args) {
    (void)self; (void)args;
    return mk_dict_ab();
}

/* dict_setitem(d, k, v) -> size after (PyDict_SetItem; unhashable raises) */
static PyObject *c_dict_setitem(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *v;
    if (!PyArg_ParseTuple(args, "OOO", &d, &k, &v)) return 0;
    if (PyDict_SetItem(d, k, v) < 0) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* dict_getitem_ref(d, k) -> (rc, value-or-None) */
static PyObject *c_dict_getitem_ref(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    int rc = PyDict_GetItemRef(d, k, &res);
    if (rc < 0) return 0;
    PyObject *val = res ? res : Py_None;
    PyObject *out = PyTuple_Pack(2, PyLong_FromLong(rc), val);
    Py_XDECREF(res);
    return out;
}

/* dict_getitem_string_ref(d, name) -> (rc, value-or-None) */
static PyObject *c_dict_getitem_string_ref(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    const char *name;
    if (!PyArg_ParseTuple(args, "Os", &d, &name)) return 0;
    PyObject *res = NULL;
    int rc = PyDict_GetItemStringRef(d, name, &res);
    if (rc < 0) return 0;
    PyObject *val = res ? res : Py_None;
    PyObject *out = PyTuple_Pack(2, PyLong_FromLong(rc), val);
    Py_XDECREF(res);
    return out;
}

/* dict_getitem_masked(d, k) -> value-or-None; NULL + NO error => None
   (the GetItem masking contract: missing key AND unhashable key both
   yield NULL with no pending error — the driver asserts no error). */
static PyObject *c_dict_getitem_masked(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    PyObject *r = PyDict_GetItem(d, k);
    if (PyErr_Occurred()) return 0; /* must NOT happen; propagate loudly */
    if (!r) Py_RETURN_NONE;
    Py_INCREF(r);
    return r;
}

/* dict_getitem_with_error(d, k) -> value-or-None; unhashable raises */
static PyObject *c_dict_getitem_with_error(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    PyObject *r = PyDict_GetItemWithError(d, k);
    if (!r) {
        if (PyErr_Occurred()) return 0;
        Py_RETURN_NONE;
    }
    Py_INCREF(r);
    return r;
}

/* dict_contains(d, k) -> bool (unhashable raises) */
static PyObject *c_dict_contains(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    int c = PyDict_Contains(d, k);
    if (c < 0) return 0;
    return PyBool_FromLong(c);
}

/* dict_contains_string(d, name) -> bool */
static PyObject *c_dict_contains_string(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    const char *name;
    if (!PyArg_ParseTuple(args, "Os", &d, &name)) return 0;
    int c = PyDict_ContainsString(d, name);
    if (c < 0) return 0;
    return PyBool_FromLong(c);
}

/* dict_delitem(d, k) -> size after (missing raises KeyError) */
static PyObject *c_dict_delitem(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    if (PyDict_DelItem(d, k) < 0) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* dict_delitem_string(d, name) -> size after */
static PyObject *c_dict_delitem_string(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    const char *name;
    if (!PyArg_ParseTuple(args, "Os", &d, &name)) return 0;
    if (PyDict_DelItemString(d, name) < 0) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* dict_pop(d, k) -> (rc, value-or-None) */
static PyObject *c_dict_pop(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    int rc = PyDict_Pop(d, k, &res);
    if (rc < 0) return 0;
    PyObject *val = res ? res : Py_None;
    PyObject *out = PyTuple_Pack(2, PyLong_FromLong(rc), val);
    Py_XDECREF(res);
    return out;
}

/* dict_pop_string(d, name) -> (rc, value-or-None) */
static PyObject *c_dict_pop_string(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    const char *name;
    if (!PyArg_ParseTuple(args, "Os", &d, &name)) return 0;
    PyObject *res = NULL;
    int rc = PyDict_PopString(d, name, &res);
    if (rc < 0) return 0;
    PyObject *val = res ? res : Py_None;
    PyObject *out = PyTuple_Pack(2, PyLong_FromLong(rc), val);
    Py_XDECREF(res);
    return out;
}

/* dict_setdefault(d, k, v) -> value (existing or inserted) */
static PyObject *c_dict_setdefault(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *v;
    if (!PyArg_ParseTuple(args, "OOO", &d, &k, &v)) return 0;
    PyObject *r = PyDict_SetDefault(d, k, v);
    if (!r) return 0;
    Py_INCREF(r);
    return r;
}

/* dict_setdefault_ref(d, k, v) -> (rc, value) */
static PyObject *c_dict_setdefault_ref(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *v, *res = NULL;
    if (!PyArg_ParseTuple(args, "OOO", &d, &k, &v)) return 0;
    int rc = PyDict_SetDefaultRef(d, k, v, &res);
    if (rc < 0) return 0;
    PyObject *val = res ? res : Py_None;
    PyObject *out = PyTuple_Pack(2, PyLong_FromLong(rc), val);
    Py_XDECREF(res);
    return out;
}

/* dict_size(d) -> len */
static PyObject *c_dict_size(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    Py_ssize_t n = PyDict_Size(d);
    if (n < 0) return 0;
    return PyLong_FromSsize_t(n);
}

/* dict_keys(d) -> list */
static PyObject *c_dict_keys(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *r = PyDict_Keys(d);
    if (!r) return 0;
    return r;
}

static PyObject *c_dict_values(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *r = PyDict_Values(d);
    if (!r) return 0;
    return r;
}

static PyObject *c_dict_items(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *r = PyDict_Items(d);
    if (!r) return 0;
    return r;
}

static PyObject *c_dict_copy(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *r = PyDict_Copy(d);
    if (!r) return 0;
    return r;
}

/* dict_copy_then_mut(d) -> (copy, orig): proves Copy independence */
static PyObject *c_dict_copy_then_mut(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *c = PyDict_Copy(d);
    if (!c) return 0;
    PyDict_SetItemString(c, "zz", PyLong_FromLong(9));
    PyObject *out = PyTuple_Pack(2, c, d);
    return out;
}

/* dict_merge(d, other, override) -> size after (non-dict other raises) */
static PyObject *c_dict_merge(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *o;
    int ov;
    if (!PyArg_ParseTuple(args, "OOi", &d, &o, &ov)) return 0;
    if (PyDict_Merge(d, o, ov) < 0) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* dict_update(d, other) -> size after (non-mapping other raises) */
static PyObject *c_dict_update(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *o;
    if (!PyArg_ParseTuple(args, "OO", &d, &o)) return 0;
    if (PyDict_Update(d, o) < 0) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* dict_merge_seq2(d, seq, override) -> the dict (bad pairs raise) */
static PyObject *c_dict_merge_seq2(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *s;
    int ov;
    if (!PyArg_ParseTuple(args, "OOi", &d, &s, &ov)) return 0;
    if (PyDict_MergeFromSeq2(d, s, ov) < 0) return 0;
    Py_INCREF(d);
    return d;
}

/* dict_next_pairs(d) -> [[k, v], ...] via the PyDict_Next cursor */
static PyObject *c_dict_next_pairs(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyObject *out = PyList_New(0);
    Py_ssize_t pos = 0;
    PyObject *k, *v;
    while (PyDict_Next(d, &pos, &k, &v)) {
        PyList_Append(out, PyTuple_Pack(2, k, v));
    }
    if (PyErr_Occurred()) return 0;
    return out;
}

/* dict_clear(d) -> size after */
static PyObject *c_dict_clear(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d;
    if (!PyArg_ParseTuple(args, "O", &d)) return 0;
    PyDict_Clear(d);
    if (PyErr_Occurred()) return 0;
    return PyLong_FromSsize_t(PyDict_Size(d));
}

/* ---- wrong-typed receivers: the line-pinned SystemError dialect ---- */

#define BAD1(name, call) \
static PyObject *name(PyObject *self, PyObject *args) { \
    (void)self; PyObject *o; \
    if (!PyArg_ParseTuple(args, "O", &o)) return 0; \
    call; \
    if (PyErr_Occurred()) return 0; \
    Py_RETURN_NONE; \
}

BAD1(c_dict_size_bad,    PyDict_Size(o))
BAD1(c_dict_keys_bad,    PyDict_Keys(o))
BAD1(c_dict_values_bad,  PyDict_Values(o))
BAD1(c_dict_items_bad,   PyDict_Items(o))
BAD1(c_dict_copy_bad,    PyDict_Copy(o))
BAD1(c_list_size_bad,    PyList_Size(o))
BAD1(c_list_sort_bad,    PyList_Sort(o))
BAD1(c_list_reverse_bad, PyList_Reverse(o))
BAD1(c_list_astuple_bad, PyList_AsTuple(o))
BAD1(c_list_clear_bad,   PyList_Clear(o))

#define BAD2(name, call) \
static PyObject *name(PyObject *self, PyObject *args) { \
    (void)self; PyObject *o, *x; \
    if (!PyArg_ParseTuple(args, "OO", &o, &x)) return 0; \
    call; \
    if (PyErr_Occurred()) return 0; \
    Py_RETURN_NONE; \
}

BAD2(c_dict_delitem_bad,      PyDict_DelItem(o, x))
BAD2(c_dict_getitem_bad,      PyDict_GetItemWithError(o, x))
BAD2(c_list_getitem_bad,      (void)PyList_GetItem(o, PyLong_AsLong(x)))
BAD2(c_list_getslice_bad,     (void)PyList_GetSlice(o, 0, 1))
BAD2(c_tuple_getslice_bad,    (void)PyTuple_GetSlice(o, 0, 1))

#define BAD3(name, call) \
static PyObject *name(PyObject *self, PyObject *args) { \
    (void)self; PyObject *o, *x, *y; \
    if (!PyArg_ParseTuple(args, "OOO", &o, &x, &y)) return 0; \
    call; \
    if (PyErr_Occurred()) return 0; \
    Py_RETURN_NONE; \
}

BAD3(c_dict_setitem_bad,    PyDict_SetItem(o, x, y))
BAD3(c_dict_setdefault_bad, PyDict_SetDefault(o, x, y))
BAD3(c_list_setitem_bad,    PyList_SetItem(o, PyLong_AsLong(x), y))
BAD3(c_list_insert_bad,     PyList_Insert(o, PyLong_AsLong(x), y))

/* dict_pop_bad(o, k): Pop has an int return (no -1-on-CPython-docs but
   the measured 3.14 raises SystemError on non-dict) */
static PyObject *c_dict_pop_bad(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *o, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &o, &k)) return 0;
    PyDict_Pop(o, k, &res);
    if (PyErr_Occurred()) return 0;
    Py_RETURN_NONE;
}

static PyObject *c_dict_getitemref_bad(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *o, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &o, &k)) return 0;
    PyDict_GetItemRef(o, k, &res);
    if (PyErr_Occurred()) return 0;
    Py_RETURN_NONE;
}

/* ---- OOB / negative index probes ---- */

static PyObject *c_list_getitem_oob(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *r;
    long i;
    if (!PyArg_ParseTuple(args, "Ol", &l, &i)) return 0;
    r = PyList_GetItem(l, (Py_ssize_t)i);
    if (!r) return 0;
    Py_INCREF(r);
    return r;
}

static PyObject *c_list_setitem_oob(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    long i;
    if (!PyArg_ParseTuple(args, "OlO", &l, &i, &v)) return 0;
    if (PyList_SetItem(l, (Py_ssize_t)i, v) < 0) return 0;
    Py_RETURN_NONE;
}

static PyObject *c_tuple_getitem_oob(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *t, *r;
    long i;
    if (!PyArg_ParseTuple(args, "Ol", &t, &i)) return 0;
    r = PyTuple_GetItem(t, (Py_ssize_t)i);
    if (!r) return 0;
    Py_INCREF(r);
    return r;
}

/* tuple_setitem_dense() -> the tuple: builds (4 items) and fills it
   INSIDE C via New+SetItem (the only shape where SetItem is legal:
   CPython 3.14 refuses SetItem on any tuple whose refcount exceeds 1
   — a Python-passed tuple arrives with rc>1 and the call raises
   SystemError tupleobject.c:123 on BOTH runtimes, so the extension
   must construct-and-fill). */
static PyObject *c_tuple_setitem_dense(PyObject *self, PyObject *args) {
    (void)self; (void)args;
    PyObject *t = PyTuple_New(4);
    if (!t) return 0;
    PyObject *vals[4];
    vals[0] = PyUnicode_FromString("dense");
    vals[1] = PyLong_FromLong(42);
    vals[2] = PyUnicode_FromString("x");
    vals[3] = PyLong_FromLong(-7);
    for (int i = 0; i < 4; i++) {
        if (!vals[i] || PyTuple_SetItem(t, i, vals[i]) < 0) {
            Py_DECREF(t);
            return 0;
        }
    }
    return t;
}

/* ---- unhashable key probes ---- */

#define UNHASHABLE(name, call) \
static PyObject *name(PyObject *self, PyObject *args) { \
    (void)self; PyObject *d, *k; \
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0; \
    call; \
    if (PyErr_Occurred()) return 0; \
    Py_RETURN_NONE; \
}

UNHASHABLE(c_dict_setitem_unhashable,    PyDict_SetItem(d, k, PyLong_FromLong(1)))
UNHASHABLE(c_dict_delitem_unhashable,    PyDict_DelItem(d, k))
UNHASHABLE(c_dict_contains_unhashable,   PyDict_Contains(d, k))
UNHASHABLE(c_dict_setdefault_unhashable, PyDict_SetDefault(d, k, PyLong_FromLong(1)))
UNHASHABLE(c_dict_getitem_we_unhashable, PyDict_GetItemWithError(d, k))

static PyObject *c_dict_getitemref_unhashable(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    int rc = PyDict_GetItemRef(d, k, &res);
    if (rc < 0) return 0;
    return PyLong_FromLong(rc);
}

static PyObject *c_dict_pop_unhashable(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *k, *res = NULL;
    if (!PyArg_ParseTuple(args, "OO", &d, &k)) return 0;
    int rc = PyDict_Pop(d, k, &res);
    if (rc < 0) return 0;
    return PyLong_FromLong(rc);
}

/* ---- Merge/Update/MergeFromSeq2 error shapes ---- */

static PyObject *c_dict_update_pairs(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *d, *o;
    if (!PyArg_ParseTuple(args, "OO", &d, &o)) return 0;
    if (PyDict_Update(d, o) < 0) return 0;
    Py_RETURN_NONE;
}

/* ---- list construction + value paths ---- */

/* make_list() -> [3, 1, 2] via New + SetItem (steal) */
static PyObject *c_make_list(PyObject *self, PyObject *args) {
    (void)self; (void)args;
    PyObject *l = PyList_New(3);
    PyList_SetItem(l, 0, PyLong_FromLong(3));
    PyList_SetItem(l, 1, PyLong_FromLong(1));
    PyList_SetItem(l, 2, PyLong_FromLong(2));
    return l;
}

static PyObject *c_list_size(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l;
    if (!PyArg_ParseTuple(args, "O", &l)) return 0;
    Py_ssize_t n = PyList_Size(l);
    if (n < 0) return 0;
    return PyLong_FromSsize_t(n);
}

static PyObject *c_list_getitem(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *r;
    long i;
    if (!PyArg_ParseTuple(args, "Ol", &l, &i)) return 0;
    r = PyList_GetItem(l, (Py_ssize_t)i);
    if (!r) return 0;
    Py_INCREF(r);
    return r;
}

static PyObject *c_list_getitem_ref(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *r;
    long i;
    if (!PyArg_ParseTuple(args, "Ol", &l, &i)) return 0;
    r = PyList_GetItemRef(l, (Py_ssize_t)i);
    if (!r) return 0;
    return r;
}

/* list_setitem(l, i, v) -> the list (mutated in place via SetItem-steal;
   returning the handle is the source-shim contract: container handles
   are per-call mints, so callers thread the returned object). */
static PyObject *c_list_setitem(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    long i;
    if (!PyArg_ParseTuple(args, "OlO", &l, &i, &v)) return 0;
    if (PyList_SetItem(l, (Py_ssize_t)i, v) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_getslice(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *r;
    long lo, hi;
    if (!PyArg_ParseTuple(args, "Oll", &l, &lo, &hi)) return 0;
    r = PyList_GetSlice(l, (Py_ssize_t)lo, (Py_ssize_t)hi);
    if (!r) return 0;
    return r;
}

static PyObject *c_list_setslice(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    long lo, hi;
    if (!PyArg_ParseTuple(args, "OllO", &l, &lo, &hi, &v)) return 0;
    if (PyList_SetSlice(l, (Py_ssize_t)lo, (Py_ssize_t)hi, v) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_append(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    if (!PyArg_ParseTuple(args, "OO", &l, &v)) return 0;
    if (PyList_Append(l, v) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_insert(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    long i;
    if (!PyArg_ParseTuple(args, "OlO", &l, &i, &v)) return 0;
    if (PyList_Insert(l, (Py_ssize_t)i, v) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_extend(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l, *v;
    if (!PyArg_ParseTuple(args, "OO", &l, &v)) return 0;
    if (PyList_Extend(l, v) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_clear(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l;
    if (!PyArg_ParseTuple(args, "O", &l)) return 0;
    if (PyList_Clear(l) < 0) return 0;
    return PyLong_FromSsize_t(PyList_Size(l));
}

static PyObject *c_list_sort(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l;
    if (!PyArg_ParseTuple(args, "O", &l)) return 0;
    if (PyList_Sort(l) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_reverse(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l;
    if (!PyArg_ParseTuple(args, "O", &l)) return 0;
    if (PyList_Reverse(l) < 0) return 0;
    Py_INCREF(l);
    return l;
}

static PyObject *c_list_as_tuple(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *l;
    if (!PyArg_ParseTuple(args, "O", &l)) return 0;
    PyObject *t = PyList_AsTuple(l);
    if (!t) return 0;
    return t;
}

/* ---- tuple value paths ---- */

/* make_tuple() -> (1, 2, 'x') via New + SetItem */
static PyObject *c_make_tuple(PyObject *self, PyObject *args) {
    (void)self; (void)args;
    PyObject *t = PyTuple_New(3);
    PyTuple_SetItem(t, 0, PyLong_FromLong(1));
    PyTuple_SetItem(t, 1, PyLong_FromLong(2));
    PyTuple_SetItem(t, 2, PyUnicode_FromString("x"));
    return t;
}

static PyObject *c_tuple_size(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *t;
    if (!PyArg_ParseTuple(args, "O", &t)) return 0;
    Py_ssize_t n = PyTuple_Size(t);
    if (n < 0) return 0;
    return PyLong_FromSsize_t(n);
}

static PyObject *c_tuple_getitem(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *t, *r;
    long i;
    if (!PyArg_ParseTuple(args, "Ol", &t, &i)) return 0;
    r = PyTuple_GetItem(t, (Py_ssize_t)i);
    if (!r) return 0;
    Py_INCREF(r);
    return r;
}

static PyObject *c_tuple_getslice(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *t, *r;
    long lo, hi;
    if (!PyArg_ParseTuple(args, "Oll", &t, &lo, &hi)) return 0;
    r = PyTuple_GetSlice(t, (Py_ssize_t)lo, (Py_ssize_t)hi);
    if (!r) return 0;
    return r;
}

static PyObject *c_tuple_pack(PyObject *self, PyObject *args) {
    (void)self;
    PyObject *a, *b;
    if (!PyArg_ParseTuple(args, "OO", &a, &b)) return 0;
    PyObject *t = PyTuple_Pack(3, a, b, a);
    if (!t) return 0;
    return t;
}

/* ---- refcount contract probe (steal vs borrow) ---- */

/* rc_probe() -> [same_ptr, rc_after_minus_before] : SetItem steals
   (no bump), DECREF drops the caller ref, GetItemRef hands a NEW ref
   (+1 observable). Deterministic on BOTH runtimes (str, not a cached
   int). Elements come back as ints; the DRIVER formats them. */
static PyObject *c_rc_probe(PyObject *self, PyObject *args) {
    (void)self; (void)args;
    PyObject *l = PyList_New(1);
    PyList_SetItem(l, 0, Py_None); /* placeholder */
    /* Non-interned 26-char str (CPython immortalizes short/interned
       strings — deltas would read 0 there). PROBE SHAPE: SetItem STEALS
       (no bump), so the caller INCREFs first to stay alive, then DECREFs
       its own ref — the observable steal contract is "rc INVARIANT
       across SetItem+DECREF" (a non-stealing store would read +1).
       GetItem (borrow) leaves rc unchanged; GetItemRef (new ref) reads
       +1. Element 1 = the SetItem+DECREF delta (0 on a faithful steal),
       element 2 = the GetItem delta (0), element 3 = the GetItemRef
       delta (+1). */
    PyObject *s = PyUnicode_FromStringAndSize("AbCdEfGhIjKlMnOpQrStUvWxYz", 26);
    long r0 = Py_Refcnt(s);
    Py_INCREF(s);              /* caller keeps a reference of its own */
    PyList_SetItem(l, 0, s);   /* STEAL (no bump) */
    Py_DECREF(s);              /* drop the caller ref */
    long r1 = Py_Refcnt(s);
    PyObject *g = PyList_GetItem(l, 0); /* borrowed */
    long r2 = Py_Refcnt(g);    /* unchanged by GetItem */
    PyObject *gr = PyList_GetItemRef(l, 0); /* NEW ref: +1 */
    long r3 = Py_Refcnt(gr);
    /* ptr identity holds on CPython; on the shim the borrow mints a fresh
       handle, so identity is asserted via repr equality (value-level). */
    PyObject *rg = PyObject_Repr(g);
    PyObject *rs = PyObject_Repr(s);
    long same = (rg && rs && strcmp(PyUnicode_AsUTF8(rg), PyUnicode_AsUTF8(rs)) == 0) ? 1 : 0;
    Py_XDECREF(rg);
    Py_XDECREF(rs);
    Py_XDECREF(gr);
    PyObject *out = PyList_New(0);
    PyList_Append(out, PyLong_FromLong(same));
    PyList_Append(out, PyLong_FromLong(r1 - r0));   /* 0 = faithful steal */
    PyList_Append(out, PyLong_FromLong(r2 - r0));   /* 0 = borrow */
    PyList_Append(out, PyLong_FromLong(r3 - r1));   /* +1 = new ref */
    Py_DECREF(l);   /* releases the list's own ref on s */
    Py_DECREF(s);   /* drop the caller ref -> s freed here (balanced) */
    return out;
}

/* ---- dict view independence + Copy identity ---- */

static PyMethodDef containers_fns[] = {
    {"make_dict", c_make_dict, 1, "build {'a':1,'b':2} via SetItemString"},
    {"dict_setitem", c_dict_setitem, 1, "PyDict_SetItem -> size"},
    {"dict_getitem_ref", c_dict_getitem_ref, 1, "(rc, v) via GetItemRef"},
    {"dict_getitem_string_ref", c_dict_getitem_string_ref, 1, "(rc, v) via GetItemStringRef"},
    {"dict_getitem_masked", c_dict_getitem_masked, 1, "GetItem masking contract"},
    {"dict_getitem_with_error", c_dict_getitem_with_error, 1, "GetItemWithError"},
    {"dict_contains", c_dict_contains, 1, "Contains"},
    {"dict_contains_string", c_dict_contains_string, 1, "ContainsString"},
    {"dict_delitem", c_dict_delitem, 1, "DelItem -> size"},
    {"dict_delitem_string", c_dict_delitem_string, 1, "DelItemString -> size"},
    {"dict_pop", c_dict_pop, 1, "(rc, v) via Pop"},
    {"dict_pop_string", c_dict_pop_string, 1, "(rc, v) via PopString"},
    {"dict_setdefault", c_dict_setdefault, 1, "SetDefault"},
    {"dict_setdefault_ref", c_dict_setdefault_ref, 1, "(rc, v) via SetDefaultRef"},
    {"dict_size", c_dict_size, 1, "Size"},
    {"dict_keys", c_dict_keys, 1, "Keys"},
    {"dict_values", c_dict_values, 1, "Values"},
    {"dict_items", c_dict_items, 1, "Items"},
    {"dict_copy", c_dict_copy, 1, "Copy"},
    {"dict_copy_then_mut", c_dict_copy_then_mut, 1, "(copy, orig) independence"},
    {"dict_merge", c_dict_merge, 1, "Merge(override) -> size"},
    {"dict_update", c_dict_update, 1, "Update -> size"},
    {"dict_merge_seq2", c_dict_merge_seq2, 1, "MergeFromSeq2 -> dict"},
    {"dict_next_pairs", c_dict_next_pairs, 1, "Next cursor walk"},
    {"dict_clear", c_dict_clear, 1, "Clear -> size"},
    {"dict_size_bad", c_dict_size_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_keys_bad", c_dict_keys_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_values_bad", c_dict_values_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_items_bad", c_dict_items_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_copy_bad", c_dict_copy_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_delitem_bad", c_dict_delitem_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_getitem_bad", c_dict_getitem_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_setitem_bad", c_dict_setitem_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_setdefault_bad", c_dict_setdefault_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_pop_bad", c_dict_pop_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_getitemref_bad", c_dict_getitemref_bad, 1, "non-dict -> SystemError dialect"},
    {"dict_setitem_unhashable", c_dict_setitem_unhashable, 1, "unhashable key dialect"},
    {"dict_delitem_unhashable", c_dict_delitem_unhashable, 1, "unhashable key dialect"},
    {"dict_setdefault_unhashable", c_dict_setdefault_unhashable, 1, "unhashable key dialect"},
    {"dict_getitem_we_unhashable", c_dict_getitem_we_unhashable, 1, "unhashable key dialect"},
    {"dict_getitemref_unhashable", c_dict_getitemref_unhashable, 1, "unhashable key rc=-1"},
    {"dict_pop_unhashable", c_dict_pop_unhashable, 1, "unhashable key rc=-1"},
    {"dict_update_pairs", c_dict_update_pairs, 1, "Update(pairs-list) AttributeError"},
    {"make_list", c_make_list, 1, "build [3,1,2] via New+SetItem"},
    {"list_size", c_list_size, 1, "Size"},
    {"list_getitem", c_list_getitem, 1, "GetItem"},
    {"list_getitem_ref", c_list_getitem_ref, 1, "GetItemRef"},
    {"list_setitem", c_list_setitem, 1, "SetItem (steal)"},
    {"list_getslice", c_list_getslice, 1, "GetSlice"},
    {"list_setslice", c_list_setslice, 1, "SetSlice -> list"},
    {"list_append", c_list_append, 1, "Append -> list"},
    {"list_insert", c_list_insert, 1, "Insert -> list"},
    {"list_extend", c_list_extend, 1, "Extend -> list"},
    {"list_clear", c_list_clear, 1, "Clear -> size"},
    {"list_sort", c_list_sort, 1, "Sort -> list"},
    {"list_reverse", c_list_reverse, 1, "Reverse -> list"},
    {"list_as_tuple", c_list_as_tuple, 1, "AsTuple"},
    {"list_size_bad", c_list_size_bad, 1, "non-list -> SystemError dialect"},
    {"list_sort_bad", c_list_sort_bad, 1, "non-list -> SystemError dialect"},
    {"list_reverse_bad", c_list_reverse_bad, 1, "non-list -> SystemError dialect"},
    {"list_astuple_bad", c_list_astuple_bad, 1, "non-list -> SystemError dialect"},
    {"list_getitem_bad", c_list_getitem_bad, 1, "non-list -> SystemError dialect"},
    {"list_setitem_bad", c_list_setitem_bad, 1, "non-list -> SystemError dialect"},
    {"list_insert_bad", c_list_insert_bad, 1, "non-list -> SystemError dialect"},
    {"list_getslice_bad", c_list_getslice_bad, 1, "non-list -> SystemError dialect"},
    {"tuple_getslice_bad", c_tuple_getslice_bad, 1, "non-tuple -> SystemError dialect"},
    {"list_clear_bad", c_list_clear_bad, 1, "non-list -> SystemError dialect"},
    {"list_getitem_oob", c_list_getitem_oob, 1, "OOB IndexError"},
    {"list_setitem_oob", c_list_setitem_oob, 1, "OOB IndexError"},
    {"make_tuple", c_make_tuple, 1, "build (1,2,'x')"},
    {"tuple_size", c_tuple_size, 1, "Size"},
    {"tuple_getitem", c_tuple_getitem, 1, "GetItem"},
    {"tuple_getslice", c_tuple_getslice, 1, "GetSlice"},
    {"tuple_pack", c_tuple_pack, 1, "Pack(3)"},
    {"tuple_getitem_oob", c_tuple_getitem_oob, 1, "OOB IndexError"},
    {"tuple_setitem_dense", c_tuple_setitem_dense, 1, "dense SetItem write"},
    {"rc_probe", c_rc_probe, 1, "steal/borrow refcount contract"},
    {0, 0, 0, 0},
};

static PyModuleDef containers_module = {
    "containers",
    "leaf 1005a container-band proof",
    -1,
    containers_fns,
};

static PyObject *containers_init(void) {
    return PyModule_Create(&containers_module);
}

PyObject *PyInit_containers(void) {
    apx_register_module_init("containers", containers_init);
    return containers_init();
}

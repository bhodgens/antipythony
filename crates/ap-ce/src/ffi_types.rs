//! Raw FFI types shared by the dispatcher and the exported C-API symbols.

use std::ffi::c_void;

/// `PyObject*` in our ABI: pointer to the refcount-emulated handle.
pub type PyObjPtr = *mut c_void;

/// `PyCFunction`: (self, args tuple) -> PyObject* | NULL on error.
pub type PyCFunctionRaw = unsafe extern "C" fn(slf: PyObjPtr, args: PyObjPtr) -> PyObjPtr;

/// Getter/setter: (self, closure) -> PyObject* | NULL.
pub type GetterRaw = unsafe extern "C" fn(slf: PyObjPtr, closure: PyObjPtr) -> PyObjPtr;
pub type SetterRaw =
    unsafe extern "C" fn(slf: PyObjPtr, value: PyObjPtr, closure: PyObjPtr) -> libc::c_int;

/// tp_init-style: (self, args, kwds) -> int.
pub type InitRaw =
    unsafe extern "C" fn(slf: PyObjPtr, args: PyObjPtr, kwds: PyObjPtr) -> libc::c_int;

/// Module init: returns the module handle (or NULL with error set).
pub type ModuleInitRaw = unsafe extern "C" fn() -> PyObjPtr;

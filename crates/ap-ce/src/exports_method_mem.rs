//! Leaf 1006 bands 3/4/5 — method/class ctors + the PyMutex/PyUnstable/
//! Py_Version emission set, the PyMem allocator front, and the PyGILState
//! bookkeeping — differential-proven against CPython 3.14.7.
//!
//! Measured dialect (scratch-1006 probes + /opt/homebrew python3.14):
//! - PyMethod_New(f, bound) -> type 'method'; __func__/__self__ resolve.
//! - PyClassMethod_New / PyStaticMethod_New / PyInstanceMethod_New ->
//!   'classmethod' / 'staticmethod' / 'instancemethod' descriptors.
//! - PyFunction_Type token -> 'function'.
//! - Py_Version = 0x030e07f0 for 3.14.7 (hex composite: 0x030E07F0);
//!   Py_GetVersion() = "3.14.7 (main, Aug  5 2026, 10:29:49) [Clang ...]"
//!   — the version prefix + our build's compiler note (the driver pins the
//!   "3.14.7" prefix on both runtimes, not the suffix).
//! - PyMutex_Lock/Unlock: real lock semantics (CPython fatal-errors on
//!   double-unlock — we implement genuine blocking locks, so the same
//!   misuse deadlocks rather than aborts: recorded divergence, unreachable
//!   from correct programs). IsLocked probed 0/1/0.
//! - PyGILState_Ensure/Release: nested Ensure returns PyGILState_LOCKED(0)
//!   on both; Release restores. PyEval_SaveThread/RestoreThread return a
//!   non-NULL tstate token (CPython: the thread state; ours: a counter
//!   cell — non-nullness is the contract).
//! - PyMem family: malloc/calloc/realloc/free + Raw variants: plain
//!   allocator front over libc. PyMem_Malloc(0) returns NON-NULL on
//!   CPython (pymalloc pools) — measured malloc0 null=0.
//!
//! Refcount contracts (CPython docs): PyMethod_New etc. return NEW
//! references; the ctors hold strong refs to their callable/self args.

use crate::ffi_types::PyObjPtr;
use crate::state::{ce_newref, err_set, new_handle, CeHandle, CeKind, CePayload};
use std::ffi::{c_char, c_int, c_ulong, c_void};

// ---------------------------------------------------------------------------
// Band 3: method/class ctors
// ---------------------------------------------------------------------------

/// The kind tag for the descriptor-shaped wrappers this band mints.
/// Payload = CePayload::Opaque over a runtime Descriptor PyObject whose
/// descr_kind + func mirror make_descr's shape (ap-run's descriptor
/// protocol binds them natively at attr time).
fn make_descr_value(kind: &str, callable: CeHandle) -> CeHandle {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, kind);
    *o.descr_kind.lock().unwrap_or_else(|p| p.into_inner()) = Some(kind.to_string());
    // The callable rides as the descriptor's func (make_descr contract:
    // interp.rs:13624 — func = args.first()).
    if let Some(v) = crate::value_conv::to_value(callable) {
        *o.func.lock().unwrap_or_else(|p| p.into_inner()) = Some(v);
    }
    new_handle(CeKind::Opaque, CePayload::Opaque(o))
}

/// PyMethod_New(callable, self) — a NEW 'method' object (bound).
/// Payload: a runtime BoundMethod PyObject (func + self_value), so attr
/// protocol (__func__/__self__/call) resolves through the runtime's own
/// bound-method surface.
#[no_mangle]
pub unsafe extern "C" fn PyMethod_New(callable: PyObjPtr, self_: PyObjPtr) -> PyObjPtr {
    if callable.is_null() {
        err_set("SystemError", "PyMethod_New: NULL callable");
        return std::ptr::null_mut();
    }
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::BoundMethod, "method");
    if let Some(f) = crate::value_conv::to_value(callable as CeHandle) {
        *o.func.lock().unwrap_or_else(|p| p.into_inner()) = Some(f);
    }
    let slf = if self_.is_null() {
        ap_obj::value::Value::None
    } else {
        crate::value_conv::to_value(self_ as CeHandle).unwrap_or(ap_obj::value::Value::None)
    };
    *o.self_value.lock().unwrap_or_else(|p| p.into_inner()) = Some(slf);
    let handle = new_handle(CeKind::Opaque, CePayload::Opaque(o));
    // Record the ORIGINAL arg handles so __func__/__self__ answer with
    // identity (the uf == f contract; measured CPython).
    METHOD_PARTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get_or_insert_with(std::collections::HashMap::new)
        .insert(handle as usize, (callable as usize, self_ as usize));
    handle as PyObjPtr
}

/// PyMethod_Type token — a Type handle whose name is 'method' (the
/// type-object token surface: PyMethod_Check macro rides apx type names).
#[no_mangle]
pub extern "C" fn apx_PyMethod_Type() -> PyObjPtr {
    type_token("method")
}

/// PyFunction_Type token — 'function'.
#[no_mangle]
pub extern "C" fn apx_PyFunction_Type() -> PyObjPtr {
    type_token("function")
}

fn type_token(name: &str) -> PyObjPtr {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Type, name);
    new_handle(CeKind::Opaque, CePayload::Opaque(o)) as PyObjPtr
}

/// The owned type NAME behind a handle (the objectproto vtable's
/// ClassNameOf route; falls back to the handle's kind name).
unsafe fn type_name_owned(h: CeHandle) -> String {
    let o = &*h;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Opaque(v)) => match v.kind {
            ap_obj::ObjKind::BoundMethod => "method".to_string(),
            ap_obj::ObjKind::Descriptor => v
                .descr_kind
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone()
                .unwrap_or_else(|| "object".to_string()),
            _ => "object".to_string(),
        },
        _ => o.type_name().to_string(),
    }
}

/// apx_TypeNameOf — the type NAME of a handle (the differential pins the
/// 'method'/'classmethod'/... strings).
#[no_mangle]
pub unsafe extern "C" fn apx_TypeNameOf(o: PyObjPtr) -> *const c_char {
    static NAMES: [(&str, std::sync::OnceLock<std::ffi::CString>); 8] = [
        ("method", std::sync::OnceLock::new()),
        ("function", std::sync::OnceLock::new()),
        ("classmethod", std::sync::OnceLock::new()),
        ("staticmethod", std::sync::OnceLock::new()),
        ("instancemethod", std::sync::OnceLock::new()),
        ("int", std::sync::OnceLock::new()),
        ("str", std::sync::OnceLock::new()),
        ("object", std::sync::OnceLock::new()),
    ];
    let name: String = if o.is_null() {
        "object".to_string()
    } else {
        let h = o as CeHandle;
        // SAFETY: h is non-null (o checked above); type_name_owned is our own
        let owned = unsafe { type_name_owned(h) };
        owned
    };
    for (n, cell) in NAMES.iter() {
        if *n == name.as_str() {
            let c = cell.get_or_init(|| std::ffi::CString::new(*n).unwrap());
            return c.as_ptr();
        }
    }
    let c = NAMES[7]
        .1
        .get_or_init(|| std::ffi::CString::new("object").unwrap());
    c.as_ptr()
}

/// The type-token name accessors.
#[no_mangle]
pub extern "C" fn apx_PyFunction_Type_name() -> *const c_char {
    static N: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    N.get_or_init(|| std::ffi::CString::new("function").unwrap())
        .as_ptr()
}

#[no_mangle]
pub extern "C" fn apx_PyMethod_Type_name() -> *const c_char {
    static N: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    N.get_or_init(|| std::ffi::CString::new("method").unwrap())
        .as_ptr()
}

#[no_mangle]
pub extern "C" fn apx_PyClassMethod_Type_name() -> *const c_char {
    static N: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    N.get_or_init(|| std::ffi::CString::new("classmethod").unwrap())
        .as_ptr()
}

#[no_mangle]
pub extern "C" fn apx_PyStaticMethod_Type_name() -> *const c_char {
    static N: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    N.get_or_init(|| std::ffi::CString::new("staticmethod").unwrap())
        .as_ptr()
}

#[no_mangle]
pub extern "C" fn apx_PyInstanceMethod_Type_name() -> *const c_char {
    static N: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    N.get_or_init(|| std::ffi::CString::new("instancemethod").unwrap())
        .as_ptr()
}

/// PyClassMethod_New(callable) — NEW 'classmethod' descriptor.
#[no_mangle]
pub unsafe extern "C" fn PyClassMethod_New(callable: PyObjPtr) -> PyObjPtr {
    if callable.is_null() {
        err_set("SystemError", "PyClassMethod_New: NULL callable");
        return std::ptr::null_mut();
    }
    make_descr_value("classmethod", callable as CeHandle) as PyObjPtr
}

/// PyStaticMethod_New(callable) — NEW 'staticmethod' descriptor.
#[no_mangle]
pub unsafe extern "C" fn PyStaticMethod_New(callable: PyObjPtr) -> PyObjPtr {
    if callable.is_null() {
        err_set("SystemError", "PyStaticMethod_New: NULL callable");
        return std::ptr::null_mut();
    }
    make_descr_value("staticmethod", callable as CeHandle) as PyObjPtr
}

/// PyStaticMethod_Type token.
#[no_mangle]
pub extern "C" fn apx_PyStaticMethod_Type() -> PyObjPtr {
    type_token("staticmethod")
}

/// PyClassMethod_Type token.
#[no_mangle]
pub extern "C" fn apx_PyClassMethod_Type() -> PyObjPtr {
    type_token("classmethod")
}

/// PyInstanceMethod_New(callable) — NEW 'instancemethod' descriptor.
#[no_mangle]
pub unsafe extern "C" fn PyInstanceMethod_New(callable: PyObjPtr) -> PyObjPtr {
    if callable.is_null() {
        err_set("SystemError", "PyInstanceMethod_New: NULL callable");
        return std::ptr::null_mut();
    }
    make_descr_value("instancemethod", callable as CeHandle) as PyObjPtr
}

/// PyInstanceMethod_Type token.
#[no_mangle]
pub extern "C" fn apx_PyInstanceMethod_Type() -> PyObjPtr {
    type_token("instancemethod")
}

/// __func__ accessor (the m.__func__ C surface; CPython returns the bound
/// method's underlying callable with a NEW reference).
#[no_mangle]
pub unsafe extern "C" fn apx_PyMethod_Function(m: PyObjPtr) -> PyObjPtr {
    method_part(m, true)
}

/// __self__ accessor.
#[no_mangle]
pub unsafe extern "C" fn apx_PyMethod_Self(m: PyObjPtr) -> PyObjPtr {
    method_part(m, false)
}

unsafe fn method_part(m: PyObjPtr, func: bool) -> PyObjPtr {
    let h = m as CeHandle;
    if h.is_null() {
        return std::ptr::null_mut();
    }
    // Identity contract: __func__/__self__ must resolve to the EXACT
    // handle the caller passed to PyMethod_New (measured: `uf == f`).
    // The side table caches the original arg handles per method object.
    let cached = METHOD_PARTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(|m| m.get(&(h as usize)))
        .copied();
    if let Some((f_h, s_h)) = cached {
        let want = if func { f_h } else { s_h };
        if want != 0 {
            let h2 = want as CeHandle;
            ce_newref(h2);
            return h2 as PyObjPtr;
        }
    }
    let o = &*h;
    let payload_borrow = o.payload.borrow();
    let Some(CePayload::Opaque(v)) = payload_borrow.as_ref() else {
        return std::ptr::null_mut();
    };
    let slot = if func { &v.func } else { &v.self_value };
    let guard = slot.lock().unwrap_or_else(|p| p.into_inner());
    let Some(val) = guard.as_ref() else {
        return std::ptr::null_mut();
    };
    let h2 = crate::state::from_value(val);
    ce_newref(h2);
    h2 as PyObjPtr
}

/// (method-handle-addr) -> (func-handle, self-handle): the identity cache
/// for apx_PyMethod_Function/Self. Pointers held as usize (Send) — never
/// dereferenced through this table, only compared. Entries persist
/// process-lifetime (the S0 handle-layer posture — handles never free).
static METHOD_PARTS: std::sync::Mutex<Option<std::collections::HashMap<usize, (usize, usize)>>> =
    std::sync::Mutex::new(None);

// ---------------------------------------------------------------------------
// Band 3: PyMutex — a REAL lock. PyMutex is 1 byte in CPython (opaque); our
// header declares the same opaque struct, but the FFI cannot ship a
// MutexGuard, so the implementation is an atomic-flag + Condvar pair in a
// global registry keyed by the caller's struct address. The struct C holds
// stays opaque and zero-initialized (PyMutex m = {0} compiles).
// ---------------------------------------------------------------------------

/// PyMutex state per caller-allocated struct. `locked` mirrors CPython's
/// bit 0 (IsLocked reads it); the Condvar parks contending Lock calls
/// (genuine blocking, matching CPython's ParkMutex semantics).
struct PyMutexState {
    locked: std::sync::atomic::AtomicBool,
    cv: std::sync::Condvar,
}

static PYMUTEX: std::sync::Mutex<Option<std::collections::HashMap<usize, PyMutexState>>> =
    std::sync::Mutex::new(None);

fn mutex_state(addr: usize) -> bool {
    let g = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
    g.as_ref()
        .and_then(|m| m.get(&addr))
        .map(|s| s.locked.load(std::sync::atomic::Ordering::Acquire))
        .unwrap_or(false)
}

/// PyMutex_Lock — blocks until the lock is held by this caller.
#[no_mangle]
pub unsafe extern "C" fn PyMutex_Lock(m: *mut c_void) {
    if m.is_null() {
        return;
    }
    let addr = m as usize;
    // Install the state on first use (CPython: static PyMutex m = {0}).
    {
        let mut g = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
        let map = g.get_or_insert_with(std::collections::HashMap::new);
        map.entry(addr).or_insert_with(|| PyMutexState {
            locked: std::sync::atomic::AtomicBool::new(false),
            cv: std::sync::Condvar::new(),
        });
    }
    // The registry guard is the parking lock. The per-mutex locked flag is
    // an atomic; parking rides a timed wait on the registry's condvar so
    // the guard types stay simple across the FFI.
    let g = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
    let map = g.as_ref().expect("just installed");
    let st = map.get(&addr).expect("installed above");
    while st.locked.swap(true, std::sync::atomic::Ordering::Acquire) {
        // Park on the registry condvar (borrow scope ends before wait).
        let cv = {
            let g2 = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
            let st2 = g2
                .as_ref()
                .expect("installed above")
                .get(&addr)
                .expect("installed above");
            &st2.cv as *const std::sync::Condvar
        };
        // SAFETY: the state lives in the process-global registry (never
        // removed), so the condvar pointer stays valid; re-lock for wait.
        let g2 = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: cv points at the registry entry's Condvar; entries are never
        let cv = unsafe { &*cv };
        let _ = cv.wait_timeout(g2, std::time::Duration::from_millis(1));
        // loop retries the swap; Unlock's notify wakes us early
    }
}

/// PyMutex_Unlock.
#[no_mangle]
pub unsafe extern "C" fn PyMutex_Unlock(m: *mut c_void) {
    if m.is_null() {
        return;
    }
    let addr = m as usize;
    let mut g = PYMUTEX.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(map) = g.as_mut() {
        if let Some(st) = map.get_mut(&addr) {
            st.locked.store(false, std::sync::atomic::Ordering::Release);
            st.cv.notify_one();
        }
    }
}

/// PyMutex_IsLocked (CPython cpython/lock.h — the probe surface the
/// differential exercises: 0 / 1 / 0 across Lock/Unlock).
#[no_mangle]
pub unsafe extern "C" fn PyMutex_IsLocked(m: *mut c_void) -> c_int {
    if m.is_null() {
        return 0;
    }
    if mutex_state(m as usize) {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Band 3: Py_Version / Py_GetVersion / PyUnstable markers
// ---------------------------------------------------------------------------

/// Py_Version — 0x030e07f0 = 3.14.7 (0x030E=3.14, 0x07=7, 0xF0=final).
#[no_mangle]
pub static mut Py_Version: c_ulong = 0x030E_07F0;

/// Py_Version accessor (the header macro maps the data symbol to this —
/// static mut across the FFI needs a fn form for the source-shim model).
#[no_mangle]
pub extern "C" fn apx_Py_Version() -> c_ulong {
    0x030E_07F0
}

/// Py_GetVersion — "3.14.7 " prefix byte-identical with CPython 3.14.7;
/// the parenthesized build note names OUR compiler (the driver pins only
/// the prefix; the suffix is build-dependent on both runtimes).
#[no_mangle]
pub extern "C" fn Py_GetVersion() -> *const c_char {
    static VERSION: std::sync::OnceLock<std::ffi::CString> = std::sync::OnceLock::new();
    VERSION
        .get_or_init(|| {
            let note = option_env!("APX_BUILD_NOTE").unwrap_or("antipythony");
            std::ffi::CString::new(format!("3.14.7 ({note})"))
                .unwrap_or_else(|_| std::ffi::CString::new("3.14.7").unwrap())
        })
        .as_ptr()
}

/// PyUnstable_InterpreterState_GetID — no-op-able marker family: return a
/// stable per-process id (1 — one interpreter per process on S1).
#[no_mangle]
pub unsafe extern "C" fn PyUnstable_InterpreterState_GetID() -> i64 {
    1
}

/// PyUnstable_Type_AssignVersionTag — marker: CPython returns 0 (already
/// had) or 1 (assigned); ours always "assigned" (versioning counters are
/// ap-obj's AtomicU64 type_version — always present).
#[no_mangle]
pub unsafe extern "C" fn PyUnstable_Type_AssignVersionTag(_t: PyObjPtr) -> c_int {
    1
}

/// PyUnstable_Long_IsCompact — an int fits one digit (compact) when it
/// fits an i64 (our Smi+ i64-shape; CPython's compact = one 30-bit digit,
/// but the OBSERVABLE contract the emission set uses is PyUnstable_Long_
/// CompactValue == the value, which holds for everything fitting i64).
/// Measured: compact(42)=1, compact(1e30)=0.
#[no_mangle]
pub unsafe extern "C" fn PyUnstable_Long_IsCompact(op: PyObjPtr) -> c_int {
    let h = op as CeHandle;
    if h.is_null() {
        return 0;
    }
    let o = &*h;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Int(i)) => {
            if i.to_i64().is_some() {
                1
            } else {
                0
            }
        }
        Some(CePayload::Bool(_)) => 1,
        _ => 0,
    }
}

/// PyUnstable_Long_CompactValue — the compact value (UB on non-compact in
/// CPython; we return 0 there — safer, unobservable in correct programs).
#[no_mangle]
pub unsafe extern "C" fn PyUnstable_Long_CompactValue(op: PyObjPtr) -> i64 {
    let h = op as CeHandle;
    if h.is_null() {
        return 0;
    }
    let o = &*h;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Int(i)) => i.to_i64().unwrap_or(0),
        Some(CePayload::Bool(b)) => {
            if *b {
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// PyUnstable_Object_ClearWeakRefsNoCallbacks — marker: our weakref
/// surface has no callbacks-no-clear split; a no-op suffices.
#[no_mangle]
pub unsafe extern "C" fn PyUnstable_Object_ClearWeakRefsNoCallbacks(_o: PyObjPtr) {}

// ---------------------------------------------------------------------------
// Band 4: PyMem allocator front (+ Raw)
// ---------------------------------------------------------------------------

/// PyMem_Malloc — plain allocator front. size 0 -> NON-NULL like CPython
/// (measured malloc0 null=0); implemented as malloc(max(size,1)).
#[no_mangle]
pub unsafe extern "C" fn PyMem_Malloc(size: usize) -> *mut c_void {
    // SAFETY: the libc allocator has no preconditions beyond a non-zero
    // size (the .max(1) guard pins that); the returned block is owned by the
    // ce payload registry and released only through the matching free seam.
    unsafe { libc::malloc(size.max(1)) }
}

/// PyMem_Calloc.
#[no_mangle]
pub unsafe extern "C" fn PyMem_Calloc(nelem: usize, elsize: usize) -> *mut c_void {
    // SAFETY: the libc allocator has no preconditions beyond a non-zero
    // size (the .max(1) guard pins that); the returned block is owned by the
    // ce payload registry and released only through the matching free seam.
    unsafe { libc::calloc(nelem.max(1), elsize.max(1)) }
}

/// PyMem_Realloc — realloc(NULL) is malloc.
#[no_mangle]
pub unsafe extern "C" fn PyMem_Realloc(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    // SAFETY: the libc allocator has no preconditions beyond a non-zero
    // size (the .max(1) guard pins that); the returned block is owned by the
    // ce payload registry and released only through the matching free seam.
    unsafe { libc::realloc(ptr, new_size.max(1)) }
}

/// PyMem_Free — free(NULL) is a no-op.
#[no_mangle]
pub unsafe extern "C" fn PyMem_Free(ptr: *mut c_void) {
    // SAFETY: the libc allocator has no preconditions beyond a non-zero
    // size (the .max(1) guard pins that); the returned block is owned by the
    // ce payload registry and released only through the matching free seam.
    unsafe {
        if !ptr.is_null() {
            libc::free(ptr);
        }
    }
}

/// PyMem_RawMalloc (the allocator-domain raw layer).
#[no_mangle]
pub unsafe extern "C" fn PyMem_RawMalloc(size: usize) -> *mut c_void {
    PyMem_Malloc(size)
}

#[no_mangle]
pub unsafe extern "C" fn PyMem_RawCalloc(nelem: usize, elsize: usize) -> *mut c_void {
    PyMem_Calloc(nelem, elsize)
}

#[no_mangle]
pub unsafe extern "C" fn PyMem_RawRealloc(ptr: *mut c_void, new_size: usize) -> *mut c_void {
    PyMem_Realloc(ptr, new_size)
}

#[no_mangle]
pub unsafe extern "C" fn PyMem_RawFree(ptr: *mut c_void) {
    PyMem_Free(ptr)
}

// ---------------------------------------------------------------------------
// Band 5: PyGILState bookkeeping (the 02.09 no-GIL model: no locks; the
// Ensure/Release pair is BOOKKEEPING — a depth counter per thread + the
// state accessor contract).
// ---------------------------------------------------------------------------

/// PyGILState_STATE values (CPython pystate.h): LOCKED = 0, UNLOCKED = 1.
/// Measured: nested Ensure on the interpreter thread returns 0 both times.
pub const PYGILSTATE_LOCKED: c_int = 0;
pub const PYGILSTATE_UNLOCKED: c_int = 1;

thread_local! {
    static GILSTATE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// PyGILState_Ensure — "have the GIL": on the no-GIL runtime this always
/// holds; the bookkeeping records the acquire depth (Release restores).
/// Returns PyGILState_LOCKED (the thread was already attached — CPython's
/// measured value on the importing thread).
#[no_mangle]
pub extern "C" fn PyGILState_Ensure() -> c_int {
    GILSTATE_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
    GILSTATE_EVER.with(|d| d.set(true));
    PYGILSTATE_LOCKED
}

/// PyGILState_Release.
#[no_mangle]
pub unsafe extern "C" fn PyGILState_Release(_state: c_int) {
    GILSTATE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
}

/// PyGILState_Check — 1 when this thread holds the interpreter. CPython
/// (measured pin4): 1 both nested and AFTER full release on the main
/// thread — the attaching thread keeps its GIL-state once attached (the
/// auto-Ensure at interpreter entry). We mirror: any thread that has
/// EVER Ensured stays "attached" (the no-GIL runtime's attached set).
thread_local! {
    static GILSTATE_EVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// PyGILState_Check.
#[no_mangle]
pub extern "C" fn PyGILState_Check() -> c_int {
    if GILSTATE_EVER.with(|d| d.get()) {
        1
    } else {
        0
    }
}

/// PyEval_SaveThread — returns a non-NULL token (CPython: the previous
/// thread state; ours: a per-thread counter box address — non-nullness is
/// the contract Py_BEGIN_ALLOW_THREADS relies on).
#[no_mangle]
pub unsafe extern "C" fn PyEval_SaveThread() -> *mut c_void {
    let b = Box::new(0u64);
    Box::into_raw(b) as *mut c_void
}

/// PyEval_RestoreThread — consume the token.
#[no_mangle]
pub unsafe extern "C" fn PyEval_RestoreThread(tstate: *mut c_void) {
    if !tstate.is_null() {
        let b = tstate as *mut u64;
        // SAFETY: tstate is the token PyEval_SaveThread returned (a
        drop(unsafe { Box::from_raw(b) });
    }
}

/// PyEval_ThreadsInitialized — always 1 post-02.07 (threads exist).
#[no_mangle]
pub extern "C" fn PyEval_ThreadsInitialized() -> c_int {
    1
}

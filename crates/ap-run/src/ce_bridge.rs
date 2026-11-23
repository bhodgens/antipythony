//! The ap-run <-> ap-ce bridge (leaf 10.02 spike): routes C-extension
//! dispatch into the ap-ce registries.
//!
//! Three seams:
//! 1. `call_ce_marker` — a `__ap_ce__.<spec>#<entry>` marker callable
//!    (tp_new/tp_init/methods/getsets from PyType_FromSpec). Parses the
//!    spec index, converts args to C handles, calls the C fn pointer,
//!    converts the result back / raises the C error state.
//! 2. `call_ce_module_fn` — dotted `mod.name` markers for registered
//!    PyMethodDef functions (PyModule_AddFunctions route).
//! 3. `native_ce_module` — the import seam: `import spike` finds a
//!    registered extension module, runs its registered PyInit fn (which
//!    registers methods/types through the header shim), and materializes
//!    the module Value from the ns.

use crate::exc_hier::ExceptionValue;
use crate::Flow;
use ap_ce::ffi_types::PyObjPtr;
use ap_ce::state::{ce_newref, err_is_set, err_take, from_value, CeHandle, CeKind, CePayload};
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

// ---------------------------------------------------------------------------
// Leaf 10.04: the call-back registration. Installed once (first ce import);
// PyObject_Call family in ap-ce dispatches back through this pointer. The
// interpreter handle rides the runtime's existing thread-local
// (interp::current_interp_handle — the 02.09 per-worker model).
// ---------------------------------------------------------------------------

static PYCALL_INSTALL: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// C ABI: (callable, args tuple, kwargs dict-or-NULL) -> result handle.
unsafe extern "C" fn runtime_pycall(
    callable: PyObjPtr,
    args: PyObjPtr,
    kwds: PyObjPtr,
) -> PyObjPtr {
    // 1. Handles -> runtime values (faithful: Opaque arms carry clones).
    let cv = ap_ce::value_conv::to_value(callable as CeHandle);
    let Some(cv) = cv else {
        ap_ce::state::err_set("SystemError", "PyObject_Call: bad callable handle");
        return std::ptr::null_mut();
    };
    let arg_vals: Vec<Value> = match (&*(args as CeHandle)).payload.borrow().as_ref() {
        Some(CePayload::Tuple(items)) => items
            .iter()
            .filter_map(|h| ap_ce::value_conv::to_value(*h))
            .collect(),
        _ => Vec::new(),
    };
    // kwargs dict-or-NULL (CPython PyObject_Call shape).
    let kw_pairs: Vec<(String, Value)> = if kwds.is_null() {
        Vec::new()
    } else {
        match ap_ce::value_conv::to_value(kwds as CeHandle) {
            Some(Value::Dict(d)) => d
                .items()
                .iter()
                .filter_map(|(k, v)| match k {
                    Value::Str(s) => Some((s.as_str().to_string(), v.clone())),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    // 2. Call into the runtime (the interpreter is the thread-local; this
    //    is re-entrant: C fn -> here -> call_value -> Python -> C fn ...).
    let interp = crate::interp::current_interp_handle();
    match crate::interp::call_value_pub(&interp, &cv, &arg_vals, &kw_pairs) {
        Ok(out) => {
            let h = from_value(&out);
            ce_newref(h);
            h as PyObjPtr
        }
        Err(flow) => {
            flow_to_err(&flow);
            std::ptr::null_mut()
        }
    }
}

/// Install the pycall pointer into ap-ce (idempotent).
fn ensure_pycall_installed() {
    PYCALL_INSTALL.get_or_init(|| {
        let _ = ap_ce::runtime_ctx::install_pycall(runtime_pycall);
        // Leaf 1005c: the error/traceband seams ride the SAME install point
        // (every C entry already funnels through ensure_pycall_installed).
        let _ = ap_ce::exports_errtb::install_frame_query(runtime_frame_query);
        let _ = ap_ce::exports_errtb::install_matches(runtime_exc_matches);
        let _ = ap_ce::exports_errtb::install_newexc(runtime_new_exception);
        let _ = ap_ce::runtime_ctx::install_errtb(runtime_errtb_dispatch);
    });
    // Leaf 1005b: the object-protocol band installs alongside (an
    // extension may hit PyObject_GetAttr before any PyObject_Call).
    crate::ce_objproto::ensure_objproto_installed();
}

/// Leaf 10.04: tp_new payload allocation — allocate the C-visible instance
/// struct (spec tp_basicsize) + register the runtime Value so C handles
/// minted for this instance resolve back faithfully.
fn alloc_ce_instance(spec_index: usize) -> Result<Value, Flow> {
    let snap = ap_ce::slots::record_clone(spec_index)
        .ok_or_else(|| raise("SystemError", "spec lost its record"))?;
    let t = ap_ce::slots::type_ref_for_spec(spec_index)
        .ok_or_else(|| raise("SystemError", "spec lost its type payload"))?;
    // Runtime-side instance FIRST (its Arc address is the payload key —
    // stable for the object's lifetime; Gc<T> = Arc).
    let inst = crate::classes::alloc_instance(&t);
    let key = if snap.basicsize > 0 {
        let v = Value::Obj(match &inst {
            Value::Obj(o) => o.clone(),
            _ => unreachable!("alloc_instance returns Value::Obj"),
        });
        let key = ap_obj::Gc::as_ptr(match &v {
            Value::Obj(o) => o,
            _ => unreachable!(),
        }) as *const std::ffi::c_void as usize;
        let block = ap_ce::payloads::payload_alloc(key, spec_index, snap.basicsize);
        if block.is_null() {
            return Err(raise("SystemError", "tp_basicsize payload alloc failed"));
        }
        ap_ce::value_map::set_runtime_value(key, v);
        key
    } else {
        let v = inst.clone();
        let key = match &v {
            Value::Obj(o) => ap_obj::Gc::as_ptr(o) as *const std::ffi::c_void as usize,
            _ => unreachable!(),
        };
        ap_ce::value_map::set_runtime_value(key, v);
        key
    };
    let _ = key; // key ride documented; handle carries it below
    Ok(inst)
}

fn ce_instance_handle(spec_index: usize, inst: &Value) -> CeHandle {
    let key = match inst {
        Value::Obj(o) => ap_obj::Gc::as_ptr(o) as *const std::ffi::c_void as usize,
        _ => 0,
    };
    let h = ap_ce::state::new_ce_instance(spec_index, key);
    ce_newref(h);
    h
}

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

/// Convert Flow -> C error state (the C fn sees "error set" on return NULL
/// via the wrapper below; used when converting incoming args fails).
fn flow_to_err(f: &Flow) {
    if let Flow::Raise(e) = f {
        ap_ce::state::err_set(
            &e.class,
            e.args
                .first()
                .map(|a| match a {
                    Value::Str(s) => s.as_str().to_string(),
                    other => ap_obj::ops::repr(other).unwrap_or_default(),
                })
                .unwrap_or_default(),
        );
    } else {
        ap_ce::state::err_set("SystemError", "bridge flow");
    }
}

/// Tuple Value -> C args tuple handle (borrowed: INCREF per CPython's
/// borrowed-arg convention so DECREF imbalance cannot free).
fn args_to_handle(args: &[Value]) -> CeHandle {
    let items: Vec<CeHandle> = args.iter().map(from_value).collect();
    let t = ap_ce::state::new_handle(CeKind::Tuple, CePayload::Tuple(items));
    ce_newref(t);
    t
}

/// Handle -> Result<Value, Flow>: NULL => take the error state => raise.
fn handle_to_result(h: *mut ap_ce::state::CeObject) -> Result<Value, Flow> {
    if h.is_null() {
        if let Some((class, msg)) = err_take() {
            return Err(raise(&class, msg));
        }
        return Err(raise(
            "SystemError",
            "C extension returned NULL without an error",
        ));
    }
    let _ = err_is_set(); // read to keep the fn total
    Ok(ap_ce::value_conv::to_value(h).unwrap_or(Value::None))
}

/// Route 1: a `__ap_ce__.<spec>#<entry>` marker call.
/// `args` are the PYTHON-level args (self already bound by the caller —
/// BoundMethod markers arrive with the receiver riding args[0]; plain
/// module fn markers don't).
pub fn call_ce_marker(full: &str, args: &[Value]) -> Option<Result<Value, Flow>> {
    // Leaf 10.04: make sure the call-back pointer is registered before ANY
    // C code runs (tp_new/methods may call PyObject_Call immediately).
    ensure_pycall_installed();
    if std::env::var_os("APX_CE_TRACE").is_some() {
        eprintln!("[ap-ce] marker ENTER {full}");
    }
    let (spec_index, entry) = ap_ce::slots::parse_marker(full)?;
    let snap = ap_ce::slots::record_clone(spec_index)?;
    Some((|| {
        // tp_new: (cls, args) — cls comes from the call site as args[0].
        // tp_init: (self, args) — self rides args[0] via BoundMethod.
        // methods: (self, args) — same binding convention.
        // getters: (self,) — property fget call shape.
        let (slf, rest): (Value, &[Value]) = match entry.as_str() {
            "__object_new__" => {
                // No tp_new: allocate a plain instance (object.__new__).
                let cls = args.first().cloned().unwrap_or(Value::None);
                return object_new_for_spec(spec_index, &cls);
            }
            _ => (
                args.first().cloned().unwrap_or(Value::None),
                &args[1.min(args.len())..],
            ),
        };
        // Build the C-side (self, args_tuple) pair.
        // Leaf 10.04 (payload model): the S0 from_value mint for a
        // UserInstance would produce an Opaque handle with NO payload
        // linkage — the C side needs a CeInstance handle carrying the
        // payload key so apx_InstancePayload(self) resolves. mint_slf
        // attaches a fresh CeInstance handle for the receiver when it is
        // an instance of THIS spec (methods/getsets/tp_init paths).
        let slf_h = mint_slf(spec_index, &slf);
        let args_h = args_to_handle(rest);
        let f: ap_ce::slots::SlotFn = match entry.as_str() {
            "__new__" => snap
                .tp_new
                .ok_or_else(|| raise("SystemError", format!("spec {spec_index} has no tp_new")))?,
            "__init__" => snap
                .tp_init
                .ok_or_else(|| raise("SystemError", format!("spec {spec_index} has no tp_init")))?,
            "g" => unreachable!("getset entries carry an index"),
            e if e.starts_with('g') && e[1..].parse::<usize>().is_ok() => {
                let gi: usize = e[1..].parse().expect("checked");
                snap.getter_at(gi).ok_or_else(|| {
                    raise(
                        "AttributeError",
                        format!("attribute has no getter (spec {spec_index})"),
                    )
                })?
            }
            e if e.starts_with('m') => {
                let mi: usize = e[1..]
                    .parse()
                    .map_err(|_| raise("SystemError", "bad method marker"))?;
                snap.method_at(mi).ok_or_else(|| {
                    raise("SystemError", format!("method index {mi} out of range"))
                })?
            }
            _ => return Err(raise("SystemError", format!("unknown ce slot '{entry}'"))),
        };
        if std::env::var_os("APX_CE_TRACE").is_some() {
            eprintln!("[ap-ce] marker {full} entry={entry} nargs={}", args.len());
        }
        let is_new = entry == "__new__";
        let is_init = entry == "__init__";
        let r = unsafe { (f.0)(slf_h as PyObjPtr, args_h as PyObjPtr) };
        if is_init {
            // tp_init has CPython `int` return semantics (0 ok, -1 error,
            // transmuted through the pointer ABI; the code rides the
            // integer register — NULL == 0 == SUCCESS here).
            let rc = r as isize;
            if rc != 0 {
                if let Some((class, msg)) = err_take() {
                    return Err(raise(&class, msg));
                }
                return Err(raise(
                    "SystemError",
                    format!("tp_init failed ({rc}) without setting an error"),
                ));
            }
            // __init__ returns None to the Python layer (check_init_return
            // contract); the instance is already materialized.
            return Ok(Value::None);
        }
        if r.is_null() && std::env::var_os("APX_CE_TRACE").is_some() {
            eprintln!("[ap-ce] marker {full} returned NULL");
        }
        if r.is_null() {
            if let Some((class, msg)) = err_take() {
                return Err(raise(&class, msg));
            }
            return Err(raise(
                "SystemError",
                "C function returned NULL without setting an error",
            ));
        }
        let _ = ce_newref(r as CeHandle);
        if is_new {
            // S1 (leaf 10.04): the C fn VALIDATES the ctor args; the runtime
            // allocates the instance AND its tp_basicsize C payload block,
            // keyed so every later C handle for this object round-trips
            // faithfully (payload + identity, not a repr pin).
            return alloc_ce_instance(spec_index);
        }
        handle_to_result(r as CeHandle)
    })())
}

/// object.__new__ arm for specs without tp_new: allocate a UserInstance of
/// the registered type (arity-0: extra args raise like CPython).
fn object_new_for_spec(spec_index: usize, cls: &Value) -> Result<Value, Flow> {
    if args_beyond_first(cls) {
        // CPython: object.__new__() with args + no tp_new override errors
        // only when __init__ is also not overridden; S0 spike surface
        // keeps it simple: reject non-empty construction.
        let Some(t) = ap_ce::slots::type_ref_for_spec(spec_index) else {
            return Err(raise("SystemError", "spec lost its type payload"));
        };
        return Err(raise(
            "TypeError",
            format!("{}() takes no arguments", t.name),
        ));
    }
    let Some(t) = ap_ce::slots::type_ref_for_spec(spec_index) else {
        return Err(raise("SystemError", "spec lost its type payload"));
    };
    Ok(crate::classes::alloc_instance(&t))
}

fn args_beyond_first(_cls: &Value) -> bool {
    // The marker call carries only cls (args[0]) when called as C(x) with
    // no args; the caller (call_value on the class) routes the FULL ctor
    // args through tp_new. This fn sees the marker-level args: len>1.
    false
}

/// Route 2: dotted `mod.name` for registered module functions.
pub fn call_ce_module_fn(module: &str, fname: &str, args: &[Value]) -> Option<Result<Value, Flow>> {
    // Leaf 10.04: call-back registration (module fns may call back too).
    ensure_pycall_installed();
    if std::env::var_os("APX_CE_TRACE").is_some() {
        eprintln!("[ap-ce] module_fn {module}.{fname} nargs={}", args.len());
    }
    let (flags, f) = ap_ce::api::find_method(module, fname)?;
    let _ = flags; // METH_* collapse to the tuple form on S0
    Some((|| {
        // pydantic-probe band: pass the REAL module handle as self (the
        // CPython PyCFunction contract — ml_self is the module for
        // module-level defs; pydantic-core's module fns attr-read the
        // module ns through it).
        let self_v = ap_ce::api::module_ns(module)
            .map(|ns| {
                let m = ap_obj::PyObject::new(ap_obj::ObjKind::Module, module);
                *m.module_ns.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(std::sync::Arc::new(ns.clone()));
                Value::Obj(m)
            })
            .unwrap_or(Value::None);
        let self_h = from_value(&self_v);
        let args_h = args_to_handle(args);
        let r = unsafe { (f)(self_h as PyObjPtr, args_h as PyObjPtr) };
        if r.is_null() {
            if let Some((class, msg)) = err_take() {
                return Err(raise(&class, msg));
            }
            return Err(raise(
                "SystemError",
                "C function returned NULL without setting an error",
            ));
        }
        handle_to_result(r as CeHandle)
    })())
}

/// Route 3: the import seam. Returns the module Value for a registered
/// extension module, running its PyInit (which fills the registries).
pub fn native_ce_module(
    _interp: &std::sync::Arc<crate::frame::Interpreter>,
    name: &str,
) -> Option<Result<Value, Flow>> {
    // 1005c: install the runtime seams BEFORE the dlopen/PyInit (the C init
    // may call PyErr_NewException / the frame probes immediately).
    ensure_pycall_installed();
    // Registered already? (module registry filled by a prior import or by
    // an embedded init at startup)
    if !ap_ce::api::module_registered(name) {
        // Try dlopen: lib<name>.so / <name>.so on sys.path-rooted dirs and
        // cwd. Undefined Py* symbols resolve against THIS process (the
        // binary exports them via -export_dynamic — the source-shim model).
        if !ensure_loaded(name) {
            return None;
        }
    }
    if ap_ce::api::module_registered(name) {
        ensure_pycall_installed();
    }
    if let Some(ns) = ap_ce::api::module_ns(name) {
        return Some(Ok(finish_module(name, &ns)));
    }
    // Run the C init (registered via apx_register_module_init).
    extern "C" {
        fn apx_run_module_init(name: *const std::ffi::c_char) -> PyObjPtr;
    }
    let cname = std::ffi::CString::new(name).ok()?;
    let h = unsafe { apx_run_module_init(cname.as_ptr()) };
    if h.is_null() {
        if let Some((class, msg)) = err_take() {
            return Some(Err(raise(&class, msg)));
        }
        return Some(Err(raise(
            "ImportError",
            format!("extension module '{name}' init failed"),
        )));
    }
    let _ = h;
    match ap_ce::api::module_ns(name) {
        Some(ns) => Some(Ok(finish_module(name, &ns))),
        None => Some(Err(raise(
            "ImportError",
            format!("extension module '{name}' registered no namespace"),
        ))),
    }
}

/// Materialize a Module PyObject Value from the ce ns (the importer seam).
fn finish_module(name: &str, ns: &ap_obj::dict::PyDict) -> Value {
    if std::env::var_os("APX_CE_TRACE").is_some() {
        eprintln!(
            "[ap-ce] finish_module({name}) ns_items={:?}",
            ns.items()
                .iter()
                .map(|(k, _)| match k {
                    Value::Str(s) => s.as_str().to_string(),
                    _ => "?".to_string(),
                })
                .collect::<Vec<_>>()
        );
    }
    let m = ap_obj::PyObject::new(ap_obj::ObjKind::Module, name);
    let d = ap_obj::dict::PyDict::new();
    let _ = d.set(
        Value::Str(PyString::new("__name__")),
        Value::Str(PyString::new(name)),
    );
    let _ = d.set(Value::Str(PyString::new("__file__")), Value::None);
    for (k, v) in ns.items() {
        let _ = d.set(k.clone(), v.clone());
    }
    let _ = d.set(Value::Str(PyString::new("__spec__")), Value::None);
    *m.module_ns.lock().unwrap_or_else(|p| p.into_inner()) =
        Some(std::sync::Arc::new((*d).clone()));
    Value::Obj(m)
}

/// Leaf 10.04: the receiver handle for slot dispatch. A UserInstance of
/// this spec gets a CeInstance handle (payload key = its Arc address —
/// matching alloc_ce_instance's keying) so C payload access resolves;
/// anything else (cls for tp_new, module self) mints normally.
fn mint_slf(spec_index: usize, slf: &Value) -> CeHandle {
    if let Value::Obj(o) = slf {
        if o.kind == ap_obj::ObjKind::UserInstance {
            if let Some(t) = ap_ce::slots::type_ref_for_spec(spec_index) {
                if o.name == t.name {
                    let key = ap_obj::Gc::as_ptr(o) as *const std::ffi::c_void as usize;
                    let h = ap_ce::state::new_ce_instance(spec_index, key);
                    ce_newref(h);
                    return h;
                }
            }
        }
    }
    from_value(slf)
}

/// Int helper used by tests.
#[allow(dead_code)]
fn i(n: i64) -> Value {
    Value::Int(Int::from_i64(n))
}

#[allow(dead_code)]
fn l(v: Vec<Value>) -> Value {
    Value::List(PyList::from_vec(v))
}

#[allow(dead_code)]
fn t(v: Vec<Value>) -> Value {
    Value::Tuple(Gc::new(Tuple(v)))
}

/// Test hook: convert a Flow error to the C error state (call sites that
/// round-trip into C and back).
pub fn flow_into_err_pub(f: &Flow) {
    flow_to_err(f);
}

// ---------------------------------------------------------------------------
// dlopen seam
// ---------------------------------------------------------------------------

unsafe fn dlopen_fullname(libc_name: *const std::ffi::c_char) -> *mut std::ffi::c_void {
    unsafe { libc::dlopen(libc_name, libc::RTLD_NOW | libc::RTLD_LOCAL) }
}

/// Is a loadable extension library visible for `name`?
fn ext_lib_candidates(name: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut dirs: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from(".")];
    if let Ok(d) = std::env::var("APX_EXT_DIR") {
        dirs.push(std::path::PathBuf::from(d));
    }
    // The RUNNING SCRIPT's directory (the interpreter's argv[0] analog):
    // ap-run records it at startup via set_script_dir.
    if let Some(d) = script_dir() {
        dirs.push(d);
    }
    for d in dirs {
        out.push(d.join(format!("lib{name}.so")));
        out.push(d.join(format!("{name}.so")));
    }
    out
}

static SCRIPT_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Record the script dir (ap-run's run_module calls this at startup).
pub fn set_script_dir(d: std::path::PathBuf) {
    let _ = SCRIPT_DIR.set(d);
}

fn script_dir() -> Option<std::path::PathBuf> {
    SCRIPT_DIR.get().cloned()
}

/// The dlopen probe: does an extension .so for `name` exist in any of the
/// candidate dirs? (1005c: the dotted-walk import path answers before the
/// ModuleNotFoundError — import_file_module's finder only knows .py.)
pub fn ext_lib_exists(name: &str) -> bool {
    ext_lib_candidates(name).into_iter().any(|p| p.is_file())
}

/// dlopen the extension lib + run PyInit_<name>. Returns true when the
/// module is now registered.
fn ensure_loaded(name: &str) -> bool {
    let lib = ext_lib_candidates(name).into_iter().find(|p| p.is_file());
    let Some(lib) = lib else {
        if std::env::var_os("APX_CE_TRACE").is_some() {
            eprintln!("[ap-ce] ensure_loaded({name}): no candidate .so found");
        }
        return false;
    };
    if std::env::var_os("APX_CE_TRACE").is_some() {
        eprintln!("[ap-ce] ensure_loaded({name}): dlopen {}", lib.display());
    }
    let Some(cpath) = std::ffi::CString::new(lib.to_string_lossy().as_bytes()).ok() else {
        return false;
    };
    unsafe {
        let handle = dlopen_fullname(cpath.as_ptr());
        if handle.is_null() {
            let dlerr = std::ffi::CStr::from_ptr(libc::dlerror()).to_string_lossy();
            if std::env::var_os("APX_CE_TRACE").is_some() {
                eprintln!("[ap-ce] ensure_loaded({name}): dlopen FAILED: {dlerr}");
            }
            ap_ce::state::err_set(
                "ImportError",
                format!("dlopen {}: {}", lib.display(), dlerr),
            );
            return false;
        }
        let sym = format!("PyInit_{name}");
        let csym = std::ffi::CString::new(sym).unwrap();
        let init = libc::dlsym(handle, csym.as_ptr());
        if init.is_null() {
            ap_ce::state::err_set(
                "ImportError",
                format!("extension '{}' has no PyInit symbol", lib.display()),
            );
            return false;
        }
        let f: unsafe extern "C" fn() -> PyObjPtr = std::mem::transmute(init);
        let r = f();
        if r.is_null() {
            // error state set by the init
            return false;
        }
        true
    }
}

// ===========================================================================
// Leaf 1005c: the error/traceback runtime seams (installed alongside the
// pycall pointer). See ap-ce/src/exports_errtb.rs for the C-side contract.
// ===========================================================================

/// The live Python frame chain as (qualname, filename, lineno, f_lasti)
/// rows, OUTERMOST FIRST (interp.py_frames order). C ABI: fills the
/// caller's FrameBuf; each row is written by value into the buffer.
unsafe extern "C" fn runtime_frame_query(
    buf: *mut ap_ce::exports_errtb::FrameBuf,
) -> std::ffi::c_int {
    let interp = crate::interp::current_interp_handle();
    let rows: Vec<ap_ce::exports_errtb::FrameRow> = {
        let st = interp.py_frames.lock().unwrap_or_else(|p| p.into_inner());
        st.iter()
            .map(|f| {
                let file = f
                    .globals
                    .get(&Value::Str(PyString::new("__file__")))
                    .and_then(|v| match v {
                        Value::Str(sv) => Some(sv.as_str().to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| interp.script_path.clone());
                (
                    f.qualname.clone(),
                    file,
                    f.last_line.get().max(1) as i64,
                    -1i64, // f_lasti: statement granularity (documented)
                )
            })
            .collect()
    };
    let out = &mut *buf;
    let n = rows.len().min(out.cap);
    for (i, r) in rows.into_iter().take(n).enumerate() {
        *out.rows.add(i) = r;
    }
    out.len = n;
    0
}

/// The runtime's exception-subclass predicate: builtin tree walk
/// (exc_hier::is_subclass) PLUS the real class payloads (user classes
/// derive through their MRO — the except-clause semantics).
unsafe extern "C" fn runtime_exc_matches(
    raised: *const std::ffi::c_char,
    target: *const std::ffi::c_char,
) -> std::ffi::c_int {
    if raised.is_null() || target.is_null() {
        return 0;
    }
    let r = std::ffi::CStr::from_ptr(raised)
        .to_string_lossy()
        .into_owned();
    let t = std::ffi::CStr::from_ptr(target)
        .to_string_lossy()
        .into_owned();
    if r == t {
        return 1;
    }
    // 1. The builtin tree (names).
    if crate::exc_hier::is_subclass(&r, &t) {
        return 1;
    }
    // 2. Real class payloads: a USER class named `raised` deriving from a
    //    builtin (or another user class) matches through its MRO.
    let interp = crate::interp::current_interp_handle();
    if let Some(rc) = crate::classes::exc_class(&interp, &r) {
        if let Some(tc) = crate::classes::exc_class(&interp, &t) {
            if rc.is_subclass(&tc) {
                return 1;
            }
        } else if crate::exc_hier::is_subclass(&t, "BaseException")
            && rc
                .mro
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .any(|m| m.name == t)
        {
            return 1;
        }
    }
    // 3. user_subclasses edges (class statements deriving from exceptions).
    {
        let edges = interp
            .user_subclasses
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // Transitive walk over user edges (child -> parent), rooted at r.
        let mut cur = r.clone();
        for _ in 0..16 {
            if cur == t {
                return 1;
            }
            let parent = edges
                .iter()
                .find(|(c, _)| *c == cur)
                .map(|(_, p)| p.clone());
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
    }
    0
}

/// PyErr_NewException backend: build a REAL exception class via
/// dynamic_type (base Exception defaults; is_exception flows from the
/// base). The class Value parks in the ce->runtime value map under the
/// returned key (usize::MAX on failure; error state set).
unsafe extern "C" fn runtime_new_exception(
    name: *const std::ffi::c_char,
    base: *const std::ffi::c_char,
    doc: *const std::ffi::c_char,
) -> usize {
    if name.is_null() {
        ap_ce::state::err_set("SystemError", "PyErr_NewException: NULL name");
        return usize::MAX;
    }
    let full = std::ffi::CStr::from_ptr(name)
        .to_string_lossy()
        .into_owned();
    // CPython: __name__ is the BARE tail ("custom"); the dotted name only
    // rides the repr. Our class payload carries ONE identity string, and
    // type(e).__name__ reads it — so the BARE tail wins here (the
    // __bases__ repr divergence is recorded as a leaf finding).
    let bare = full.rsplit('.').next().unwrap_or(&full).to_string();
    let base_name = if base.is_null() {
        "Exception".to_string()
    } else {
        std::ffi::CStr::from_ptr(base)
            .to_string_lossy()
            .into_owned()
    };
    let interp = crate::interp::current_interp_handle();
    // The base VALUE: the interned exception class (exc_classes carries
    // the real BaseException tree); PyErr_NewException-created classes
    // register under their DOTTED name in user_class_values (below), so
    // a chained NewException base resolves through that table too.
    let base_v = crate::classes::exc_class_value(&interp, &base_name)
        .or_else(|| {
            interp
                .user_class_values
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&base_name)
                .cloned()
        })
        .unwrap_or_else(|| {
            Value::Obj(ap_obj::PyObject::new(
                ap_obj::ObjKind::Type,
                base_name.clone(),
            ))
        });
    let ns = ap_obj::dict::PyDict::new();
    let _ = ns.set(
        Value::Str(PyString::new("__module__")),
        Value::Str(PyString::new("errtb")),
    );
    // PyErr_NewExceptionWithDoc: the doc lands in the class dict (CPython
    // help()/__doc__ read it there).
    if !doc.is_null() {
        let d = std::ffi::CStr::from_ptr(doc).to_string_lossy().into_owned();
        if !d.is_empty() {
            let _ = ns.set(
                Value::Str(PyString::new("__doc__")),
                Value::Str(PyString::new(d)),
            );
        }
    }
    // CPython class identity: the dotted NAME is the class's repr shape
    // (<class 'errtb.custom'>) — dynamic_type's t.name carries it.
    match crate::classes::dynamic_type(&interp, &bare, &[base_v], ns, &[]) {
        Ok(class_v) => {
            // PyErr_NewExceptionWithDoc: also park the doc on the class
            // VALUE's attrs (the class-attr reader's dict arm answers None
            // for __doc__ on object-rooted classes — the attrs path is
            // what the isinstance/attr walkers consult first).
            if !doc.is_null() {
                let d = std::ffi::CStr::from_ptr(doc).to_string_lossy().into_owned();
                if !d.is_empty() {
                    if let Value::Obj(co) = &class_v {
                        let _ = co.attrs.lock().unwrap_or_else(|p| p.into_inner()).set(
                            Value::Str(PyString::new("__doc__")),
                            Value::Str(PyString::new(d)),
                        );
                    }
                }
            }
            let key = ap_obj::Gc::as_ptr(match &class_v {
                Value::Obj(o) => o,
                _ => unreachable!("dynamic_type returns a class Value::Obj"),
            }) as *const std::ffi::c_void as usize;
            ap_ce::value_map::set_runtime_value(key, class_v.clone());
            // Register under the dotted name so chained NewException bases
            // resolve to the REAL class (is_exception propagates through
            // the MRO — the bare name-Type fallback broke custom_sub).
            interp
                .user_class_values
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(bare.clone(), class_v.clone());
            if std::env::var_os("APX_CE_TRACE").is_some() {
                eprintln!("[ap-ce] NewException({bare}) key={key:#x}");
            }
            key
        }
        Err(f) => {
            if let Flow::Raise(e) = &f {
                if std::env::var_os("APX_CE_TRACE").is_some() {
                    eprintln!(
                        "[ap-ce] NewException({bare}) FAILED: {} {}",
                        e.class,
                        e.py_str().unwrap_or_default()
                    );
                }
                ap_ce::state::err_set(&e.class, e.py_str().unwrap_or_default());
            } else {
                ap_ce::state::err_set("SystemError", "PyErr_NewException failed");
            }
            usize::MAX
        }
    }
}

/// The value-level ErrTbOp interpreter (ap-ce runtime_ctx::ErrTbOp).
/// Every op is total: Some -> ErrTbOut::Value, None -> None_/NoIdx.
unsafe extern "C" fn runtime_errtb_dispatch(
    op: *const ap_ce::runtime_ctx::ErrTbOp,
    out: *mut ap_ce::runtime_ctx::ErrTbOut,
) -> std::ffi::c_int {
    let Some(op) = op.as_ref() else {
        return -1;
    };
    let out = &mut *out;
    match op {
        ap_ce::runtime_ctx::ErrTbOp::MakeFrame(idx) => {
            let interp = crate::interp::current_interp_handle();
            match crate::builtins::py_frame_stub(&interp, *idx) {
                v @ Value::Obj(_) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(v),
                _ => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::MakeCode(name, file, lineno) => {
            // The s016 `<code>` Namespace stub shape (co_name/co_filename/
            // co_firstlineno), byte-compatible with py_frame_stub's code.
            let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
            let a = code.attrs.lock().unwrap_or_else(|p| p.into_inner());
            let _ = a.set(
                Value::Str(PyString::new("co_name")),
                Value::Str(PyString::new(name.clone())),
            );
            let _ = a.set(
                Value::Str(PyString::new("co_filename")),
                Value::Str(PyString::new(file.clone())),
            );
            let _ = a.set(
                Value::Str(PyString::new("co_firstlineno")),
                Value::Int(Int::from_i64(*lineno)),
            );
            drop(a);
            *out = ap_ce::runtime_ctx::ErrTbOut::Value(Value::Obj(code));
        }
        ap_ce::runtime_ctx::ErrTbOp::MakeTb(rows) => {
            // The `<tb>` Namespace snapshot shape the traceback walkers
            // (traceback.py's StackSummary over __ap_tb__ tuples) consume.
            let tb = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<traceback>");
            let entries: Vec<Value> = rows
                .iter()
                .map(|(q, f, l)| {
                    Value::Tuple(Gc::new(Tuple(vec![
                        Value::Str(PyString::new(q.clone())),
                        Value::Int(Int::from_i64(*l as i64)),
                        Value::Str(PyString::new(f.clone())),
                    ])))
                })
                .collect();
            let a = tb.attrs.lock().unwrap_or_else(|p| p.into_inner());
            let _ = a.set(
                Value::Str(PyString::new("__ap_tb__")),
                Value::List(PyList::from_vec(entries)),
            );
            drop(a);
            *out = ap_ce::runtime_ctx::ErrTbOut::Value(Value::Obj(tb));
        }
        ap_ce::runtime_ctx::ErrTbOp::FrameAttr(v, attr) => {
            // Route through the interpreter's own attr surface: `<frame>`
            // values answer f_locals/f_globals/f_lineno (f_lineno LIVE via
            // the s016-fix2 arm; f_locals mirrors the snapshot shape).
            let interp = crate::interp::current_interp_handle();
            match crate::interp::attr_get_public(&interp, v, attr) {
                Ok(Some(av)) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(av),
                _ => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::FrameGetVar(v, name) => {
            let interp = crate::interp::current_interp_handle();
            match crate::interp::attr_get_public(&interp, v, "f_locals") {
                Ok(Some(Value::Dict(d))) => match d.get(&Value::Str(PyString::new(name.clone()))) {
                    Some(av) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(av.clone()),
                    None => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
                },
                _ => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::GeneratorCode(v) => {
            // gi_code on a generator value (interp.rs Generator arm
            // synthesizes a `<code>` Namespace stub).
            let interp = crate::interp::current_interp_handle();
            match crate::interp::attr_get_public(&interp, v, "gi_code") {
                Ok(Some(cv)) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(cv),
                _ => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::CodeAttr(v, attr) => {
            // The PyCode_Get* projection: the runtime's code stubs carry
            // the real attrs when present (py_frame_stub's <code>); a miss
            // answers the CPython-documented empty shape (co_code b"",
            // co_varnames/cellvars/freevars the empty tuple) instead of
            // NULL — CPython's own getters always return the attr.
            let interp = crate::interp::current_interp_handle();
            let hit = crate::interp::attr_get_public(&interp, v, attr)
                .ok()
                .flatten();
            let value = hit.or_else(|| match attr.as_str() {
                "co_code" => Some(Value::Bytes(Gc::new(Vec::new()))),
                "co_varnames" | "co_cellvars" | "co_freevars" | "co_names" | "co_consts" => {
                    Some(Value::Tuple(Gc::new(Tuple(Vec::new()))))
                }
                _ => None,
            });
            match value {
                Some(cv) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(cv),
                None => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::FrameIndex(v) => {
            // The `<frame>` stub carries its py_frames index in
            // slice_parts[0] (builtins::py_frame_stub contract) and also
            // as the __ap_pf_index__ attr.
            let slice_idx = match v {
                Value::Obj(o) => o
                    .slice_parts
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .first()
                    .and_then(|x| x.as_int())
                    .and_then(|i| i.to_i64()),
                _ => None,
            };
            let idx = match slice_idx {
                Some(i) if i >= 0 => Some(i as usize),
                _ => {
                    // Attr fallback (the s016 __ap_pf_index__ carrier).
                    let interp = crate::interp::current_interp_handle();
                    match crate::interp::attr_get_public(&interp, v, "__ap_pf_index__") {
                        Ok(Some(av)) => av
                            .as_int()
                            .and_then(|i| i.to_i64())
                            .filter(|i| *i >= 0)
                            .map(|i| i as usize),
                        _ => None,
                    }
                }
            };
            match idx {
                Some(i) => *out = ap_ce::runtime_ctx::ErrTbOut::Idx(i),
                None => *out = ap_ce::runtime_ctx::ErrTbOut::NoIdx,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::ExcDescribe(v) => {
            // (class, str(exc)) as a 2-tuple; the runtime's own py_str
            // dialect (KeyError repr etc.) via ExceptionValue.
            let ev = exception_value_of(v);
            let class = match v {
                Value::Obj(o) => o.name.clone(),
                other => other.type_name().to_string(),
            };
            let text = ev
                .map(|e| e.py_str().unwrap_or_default())
                .unwrap_or_else(|| ap_obj::ops::to_str(v).unwrap_or_default());
            *out = ap_ce::runtime_ctx::ErrTbOut::Value(Value::Tuple(Gc::new(Tuple(vec![
                Value::Str(PyString::new(class)),
                Value::Str(PyString::new(text)),
            ]))));
        }
        ap_ce::runtime_ctx::ErrTbOp::MakeExceptionInstance(class, text) => {
            let interp = crate::interp::current_interp_handle();
            // Materialize the SAME shape Python's `except` binding sees:
            // exception_instance over a class+msg ExceptionValue.
            let ev = crate::exc_hier::ExceptionValue::msg(class, text.clone());
            let inst = crate::interp::exception_instance_value(&ev);
            let _ = interp;
            *out = ap_ce::runtime_ctx::ErrTbOut::Value(inst);
        }
        ap_ce::runtime_ctx::ErrTbOp::ClassNameOf(v) => {
            // The CLASS name behind a class-or-instance value: exception
            // instances report their class via class_of_value's payload;
            // classes report their own name.
            let interp = crate::interp::current_interp_handle();
            let name = match crate::classes::class_of_value(&interp, v) {
                Some(t) => Some(t.name.clone()),
                None => match v {
                    Value::Obj(o)
                        if o.kind == ap_obj::ObjKind::UserClass
                            || o.kind == ap_obj::ObjKind::Type =>
                    {
                        Some(o.name.clone())
                    }
                    _ => None,
                },
            };
            match name {
                Some(n) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(Value::Str(PyString::new(n))),
                None => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
        ap_ce::runtime_ctx::ErrTbOp::GetAttr(v, attr) => {
            let interp = crate::interp::current_interp_handle();
            match crate::interp::attr_get_public(&interp, v, attr) {
                Ok(Some(av)) => *out = ap_ce::runtime_ctx::ErrTbOut::Value(av),
                _ => *out = ap_ce::runtime_ctx::ErrTbOut::None_,
            }
        }
    }
    0
}

/// The ExceptionValue behind an exception-instance value (None when the
/// value is not an exception instance). Uses the runtime's own
/// instance->EV conversion so user-class instances keep their args.
fn exception_value_of(v: &Value) -> Option<crate::exc_hier::ExceptionValue> {
    match crate::interp::exception_value_of_instance_pub(v) {
        Ok(ev) => Some(ev),
        Err(_) => None,
    }
}

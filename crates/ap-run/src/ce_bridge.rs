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
        let slf_h = from_value(&slf);
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
            // S0 tp_new contract: the C fn VALIDATES the ctor args; the
            // runtime allocates (alloc_instance) and tp_init receives it.
            // (CPython's tp_new allocates the C payload; there is no C-side
            // instance payload on S0 — documented divergence.)
            let t = ap_ce::slots::type_ref_for_spec(spec_index)
                .ok_or_else(|| raise("SystemError", "spec lost its type payload"))?;
            return Ok(crate::classes::alloc_instance(&t));
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
    if std::env::var_os("APX_CE_TRACE").is_some() {
        eprintln!("[ap-ce] module_fn {module}.{fname} nargs={}", args.len());
    }
    let (flags, f) = ap_ce::api::find_method(module, fname)?;
    let _ = flags; // METH_* collapse to the tuple form on S0
    Some((|| {
        let self_h = from_value(&Value::None); // module-level fns: self = NULL-ish
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
        eprintln!("[ap-ce] finish_module({name}) ns_items={:?}", ns.items().iter().map(|(k,_)| match k { Value::Str(s) => s.as_str().to_string(), _ => "?".to_string() }).collect::<Vec<_>>());
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
    let mut dirs: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from("."),
    ];
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

/// dlopen the extension lib + run PyInit_<name>. Returns true when the
/// module is now registered.
fn ensure_loaded(name: &str) -> bool {
    let lib = ext_lib_candidates(name).into_iter().find(|p| p.is_file());
    let Some(lib) = lib else {
        return false;
    };
    let Some(cpath) = std::ffi::CString::new(lib.to_string_lossy().as_bytes()).ok() else {
        return false;
    };
    unsafe {
        let handle = dlopen_fullname(cpath.as_ptr());
        if handle.is_null() {
            ap_ce::state::err_set(
                "ImportError",
                format!(
                    "dlopen {}: {}",
                    lib.display(),
                    std::ffi::CStr::from_ptr(libc::dlerror()).to_string_lossy()
                ),
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

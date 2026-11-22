//! PyModule_Create + PyType_FromSpec entry points, and the registries the
//! ap-run bridge reads. These are the Rust functions the exported `#[no_mangle]`
//! C symbols in `exports.rs` thin-wrap.

use crate::ffi_types::{GetterRaw, InitRaw, PyCFunctionRaw, PyObjPtr, SetterRaw};
use crate::slots::{from_spec, SlotFn, SpecSnapshot};
use crate::state::{err_set, new_handle, slot_registry, CeHandle, CeKind, CePayload};
use ap_obj::dict::PyDict;
use std::ffi::CStr;
use std::sync::{Mutex, OnceLock};

/// One PyMethodDef normalized.
#[derive(Clone)]
pub struct MethodRecord {
    pub module: String,
    pub name: String,
    pub flags: i32,
    pub f: PyCFunctionRaw,
}

/// One registered module (handle stored as usize to stay Send).
pub struct ModuleRecord {
    pub name: String,
    /// The module handle PyModule_Create returned (ns lives in the payload).
    pub handle: usize,
}

/// API-method flags we honor (subset; METH_VARARGS | METH_KEYWORDS collapse
/// to the tuple form on S0 — kwargs-bearing PyCFunctions are S1).
pub const METH_VARARGS: i32 = 0x0001;
pub const METH_NOARGS: i32 = 0x0004;
pub const METH_O: i32 = 0x0008;
pub const METH_KEYWORDS: i32 = 0x0002;
pub const METH_STATIC: i32 = 0x0010;
pub const METH_CLASS: i32 = 0x0020;

/// (module, name) -> C fn pointer + flags. Filled by PyModule_Create /
/// the header shim's table walk; read by ap-run's ce_bridge.
pub fn method_registry() -> &'static Mutex<Vec<MethodRecord>> {
    static R: OnceLock<Mutex<Vec<MethodRecord>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

/// name -> module record. Read by the import seam (bridge::native_ce_module).
pub fn module_registry() -> &'static Mutex<Vec<ModuleRecord>> {
    static R: OnceLock<Mutex<Vec<ModuleRecord>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

/// CStr -> &str (lossy on non-UTF8; extension names are ASCII).
unsafe fn cstr<'a>(p: *const std::ffi::c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    std::str::from_utf8(CStr::from_ptr(p).to_bytes()).unwrap_or("")
}

// ---------------------------------------------------------------------------
// PyModule_Create2
// ---------------------------------------------------------------------------

/// Register a module def. `def_base` points at the C `PyModuleDef` whose
/// first field is `m_name` (char*); the layout contract lives in the
/// header shim. Returns the module handle (new ref).
pub unsafe fn module_create(def_base: PyObjPtr) -> CeHandle {
    // PyModuleDef layout (our header): m_name at offset 0 (char*).
    let name_ptr = *(def_base as *const *const std::ffi::c_char);
    let name = cstr(name_ptr).to_string();
    let ns = PyDict::new();
    let _ = ns.set(
        ap_obj::value::Value::Str(ap_obj::value::PyString::new("__name__")),
        ap_obj::value::Value::Str(ap_obj::value::PyString::new(name.clone())),
    );
    let _ = ns.set(
        ap_obj::value::Value::Str(ap_obj::value::PyString::new("__doc__")),
        ap_obj::value::Value::None,
    );
    let h = new_handle(
        CeKind::Module,
        CePayload::Module {
            name: name.clone(),
            ns,
        },
    );
    module_registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(ModuleRecord { name, handle: h as usize });
    h
}

/// Add a function to a registered module (the PyModule_AddFunctions route;
/// the header shim walks the C method table and calls this per entry).
pub unsafe fn module_add_function(module: CeHandle, name: &str, flags: i32, f: PyCFunctionRaw) {
    // Find the module name from the handle.
    let mod_name = {
        let o = &*module;
        match o.payload.borrow().as_ref() {
            Some(CePayload::Module { name, ns }) => {
                let n = name.clone();
                // Install the dotted marker callable in the ns.
                let _ = ns.set(
                    ap_obj::value::Value::Str(ap_obj::value::PyString::new(name.to_string())),
                    crate::state::marker_for(&n, name),
                );
                n
            }
            _ => {
                err_set("SystemError", "PyModule_AddFunctions: not a module");
                return;
            }
        }
    };
    method_registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(MethodRecord {
            module: mod_name,
            name: name.to_string(),
            flags,
            f,
        });
}

/// Lookup: the dotted dispatch target for `mod.name` markers.
pub fn find_method(module: &str, name: &str) -> Option<(i32, PyCFunctionRaw)> {
    let tab = method_registry().lock().unwrap_or_else(|p| p.into_inner());
    tab.iter()
        .find(|m| m.module == module && m.name == name)
        .map(|m| (m.flags, m.f))
}

/// The module ns dict for the import seam (builds the native module Value).
pub fn module_ns(name: &str) -> Option<ap_obj::dict::PyDict> {
    let tab = module_registry().lock().unwrap_or_else(|p| p.into_inner());
    let rec = tab.iter().find(|m| m.name == name)?;
    unsafe {
        match (*(rec.handle as CeHandle)).payload.borrow().as_ref() {
            Some(CePayload::Module { ns, .. }) => Some(ns.as_ref().clone()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PyType_FromSpec
// ---------------------------------------------------------------------------

/// `spec` points at the C `PyType_Spec`; the header shim has already walked
/// it into normalized Rust forms (name/slots/methods/getsets/bases) — this
/// is the entry the shim's walker calls.
#[allow(clippy::too_many_arguments)]
pub fn type_from_spec_normalized(
    name: &str,
    tp_new: Option<PyCFunctionRaw>,
    tp_init: Option<InitRaw>,
    tp_dealloc: Option<PyCFunctionRaw>,
    methods: Vec<(String, i32, PyCFunctionRaw)>,
    getsets: Vec<(String, Option<GetterRaw>, Option<SetterRaw>)>,
    bases: Vec<ap_obj::value::Value>,
) -> Result<usize, String> {
    // tp_new is PyCFunction-shaped on S0 (PyType_Slot stores void*; the
    // generic new signature reduces to (cls, args) with kwds NULL — the
    // caller's C code was written against our headers).
    from_spec(
        name,
        tp_new.map(SlotFn),
        tp_init.map(|f| SlotFn(unsafe { std::mem::transmute::<InitRaw, PyCFunctionRaw>(f) })),
        tp_dealloc.map(SlotFn),
        methods
            .into_iter()
            .map(|(n, fl, f)| (n, fl, SlotFn(f)))
            .collect(),
        getsets
            .into_iter()
            .map(|(n, g, s)| {
                (
                    n,
                    g.map(|f| {
                        SlotFn(unsafe { std::mem::transmute::<GetterRaw, PyCFunctionRaw>(f) })
                    }),
                    s.map(|f| {
                        SlotFn(unsafe { std::mem::transmute::<SetterRaw, PyCFunctionRaw>(f) })
                    }),
                )
            })
            .collect(),
        bases,
    )
}

pub fn spec_snapshot(spec_index: usize) -> Option<SpecSnapshot> {
    crate::slots::record_clone(spec_index)
}

/// Call a registered slot fn: (self_or_cls, args tuple handle) -> handle.
/// NULL return + err_is_set => Python exception in flight.
pub unsafe fn call_slot(
    f: SlotFn,
    slf: PyObjPtr,
    args: PyObjPtr,
) -> Result<CeHandle, (String, String)> {
    let r = (f.0)(slf, args);
    if r.is_null() {
        Err(crate::state::err_take().unwrap_or_else(|| {
            (
                "SystemError".to_string(),
                "<C function returned NULL without setting an error>".to_string(),
            )
        }))
    } else {
        Ok(r as CeHandle)
    }
}

/// Registry stats for the bridge/tests.
pub fn registry_stats() -> (usize, usize, usize) {
    (
        method_registry()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len(),
        slot_registry()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len(),
        module_registry()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len(),
    )
}

/// Test helper: is `name` a registered module?
pub fn module_registered(name: &str) -> bool {
    module_registry()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .any(|m| m.name == name)
}

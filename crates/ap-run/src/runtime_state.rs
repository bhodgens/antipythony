// 02.09a P3 (SPLIT-02.09.md; census punch-list P3): interpreter confinement
// of the (c)-class registries. THIS FILE IS THE MECHANISM.
//
// Design (constrained by the split's file ownership — interp.rs's
// run_module body belongs to 02.09-b, so 02.09a cannot hang the tables off
// new Interpreter FIELDS; instead the tables live HERE, keyed by the
// interpreter payload address, and the per-interp semantics are carried by
// the key):
//
//   * One process-global table per registry family (Mutex + HashMap).
//   * Key = Arc::as_ptr(interp) payload address, the identity the GC and
//     the registry walk already use everywhere (value_addr/gcmod.rs,
//     WORKER_INTERP, THREADS by_addr).
//   * run_module SEEDS the active handle via set_active_interp (the two
//     allowed interp.rs declaration sites are the CELL_BACKING /
//     MODULE_FRAME_CELLS moves; the seeding call lives in
//     gcsupport::install, which 02.09a owns and which already runs at
//     run_module entry before any module allocation).
//   * Registry accessors take &Interpreter (explicit interp passing — the
//     census's named fix for the dual-handle CURRENT_INTERP/builtins-
//     CURRENT bug) and route through the ACTIVE handle for the hot call
//     sites that cannot thread the interp parameter (native-marker
//     dispatch deep in builtins_modules).
//
// Behavior under GIL v1: byte-identical. One live Interpreter per process
// (a second in-process run REPLACES the active handle at its run_module
// entry — exactly the semantics of the thread_locals these tables replace,
// minus the thread-keyed aliasing that broke worker visibility).
//
// 02.09-b contract: when interpreters become per-thread, the active
// handle becomes per-worker (installed from worker_bootstrap through the
// WORKER_INTERP table); the table shape here does not change.

use std::cell::RefCell as TlsRefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use ap_obj::int::Gc;
use ap_obj::value::PyList;
use ap_obj::{PyDict, Value};

use crate::frame::{CellSlot, Interpreter};

type InterpKey = usize;

#[inline]
fn interp_key(interp: &Arc<Interpreter>) -> InterpKey {
    Arc::as_ptr(interp).addr()
}

// ---------------------------------------------------------------------------
// The per-registry families. Each is `Mutex<HashMap<InterpKey, T>>` — the
// Mutex satisfies Sync for the static; under the GIL it is uncontended.
// ---------------------------------------------------------------------------

macro_rules! registry_table {
    ($slot:ident, $acc:ident, $ty:ty, $doc:literal) => {
        static $slot: OnceLock<Mutex<HashMap<InterpKey, $ty>>> = OnceLock::new();
        #[allow(non_snake_case)]
        pub(crate) fn $acc() -> &'static Mutex<HashMap<InterpKey, $ty>> {
            $slot.get_or_init(|| Mutex::new(HashMap::new()))
        }
    };
}

registry_table!(
    SYS_MODULES_T,
    SYS_MODULES_T_acc,
    HashMap<String, Value>,
    "the import cache (importer.rs thread_local)"
);
registry_table!(
    SYS_PATH_T,
    SYS_PATH_T_acc,
    Vec<String>,
    "bootstrap search path (importer.rs thread_local)"
);
registry_table!(
    SYS_PATH_VIEW_T,
    SYS_PATH_VIEW_T_acc,
    Option<Gc<PyList>>,
    "live sys.path PyList (importer.rs thread_local)"
);
registry_table!(
    SYS_MODULES_VIEW_T,
    SYS_MODULES_VIEW_T_acc,
    Option<Arc<PyDict>>,
    "python-visible sys.modules dict (importer.rs thread_local)"
);
registry_table!(
    BOOTSTRAP_SEEDED_T,
    BOOTSTRAP_SEEDED_T_acc,
    bool,
    "once-flag for _bootstrap seeding (importer.rs thread_local)"
);
registry_table!(
    MODULE_CACHE_T,
    MODULE_CACHE_T_acc,
    HashMap<String, Value>,
    "native-registry modules (builtins_modules.rs thread_local)"
);
registry_table!(
    NAMED_CALLABLES_T,
    NAMED_CALLABLES_T_acc,
    HashMap<String, Value>,
    "marker-name -> callable (builtins_modules.rs thread_local)"
);
registry_table!(
    ATEXIT_FNS_T,
    ATEXIT_FNS_T_acc,
    Vec<(String, Vec<Value>)>,
    "atexit registry (builtins_modules.rs thread_local)"
);
registry_table!(
    INTERNED_T,
    INTERNED_T_acc,
    HashMap<String, Value>,
    "sys.intern canonical table (builtins.rs fn-local thread_local)"
);
registry_table!(
    SD_REGISTRY_T,
    SD_REGISTRY_T_acc,
    HashMap<String, crate::builtins_modules::SdRegistry>,
    "singledispatch registries (builtins_modules.rs thread_local)"
);
registry_table!(
    LRU_PENDING_T,
    LRU_PENDING_T_acc,
    Option<(Option<i64>, bool)>,
    "lru_cache two-stage parking (builtins_modules.rs thread_local)"
);
registry_table!(
    LRU_STATE_T,
    LRU_STATE_T_acc,
    HashMap<String, crate::builtins_modules::LruState>,
    "lru_cache wrapper state (builtins_modules.rs thread_local)"
);
registry_table!(
    DOCS_T,
    DOCS_T_acc,
    crate::lxml::DocRegistry,
    "lxml parsed-tree arena (lxml.rs process global, moved per-interp)"
);
registry_table!(
    XPATH_OBJECTS_T,
    XPATH_OBJECTS_T_acc,
    HashMap<u64, crate::lxml::XPathRow>,
    "compiled XPath registry (lxml.rs process global, moved per-interp)"
);
registry_table!(
    RE_CACHE_T,
    RE_CACHE_T_acc,
    (HashMap<(String, u32), Gc<ap_obj::PyObject>>, Vec<(String, u32)>),
    "module-level re.compile cache + order ledger (sre.rs thread_local)"
);
registry_table!(
    PAT_TABLE_T,
    PAT_TABLE_T_acc,
    (HashMap<usize, crate::sre::PatRc>, Vec<usize>),
    "compiled-pattern address table + ledger (sre.rs thread_local)"
);
registry_table!(
    MATCH_TABLE_T,
    MATCH_TABLE_T_acc,
    (HashMap<usize, Arc<crate::sre::MatchBox>>, Vec<usize>),
    "match-context address table + ledger (sre.rs thread_local)"
);
registry_table!(
    CELL_BACKING_T,
    CELL_BACKING_T_acc,
    HashMap<usize, CellSlot>,
    "__closure__ cell storage (interp.rs thread_local DECLARATION MOVED HERE)"
);
registry_table!(
    MODULE_FRAME_CELLS_T,
    MODULE_FRAME_CELLS_T_acc,
    HashMap<(u64, String), CellSlot>,
    "module-frame cells (interp.rs thread_local DECLARATION MOVED HERE)"
);
registry_table!(
    CLASS_KWARGS_T,
    CLASS_KWARGS_T_acc,
    Vec<(String, Value)>,
    "class-body keyword staging (classes.rs thread_local)"
);
registry_table!(
    CURRENT_CLASS_NS_T,
    CURRENT_CLASS_NS_T_acc,
    Option<Gc<PyDict>>,
    "exec-in-class namespace (classes.rs thread_local)"
);
registry_table!(
    BRIDGE_SOURCE_T,
    BRIDGE_SOURCE_T_acc,
    String,
    "t-string raw-span source (ast_bridge.rs thread_local)"
);

// ---------------------------------------------------------------------------
// Active handle (the seed point; replaced per in-process run).
// ---------------------------------------------------------------------------

static ACTIVE_INTERP: OnceLock<Mutex<Option<(InterpKey, Arc<Interpreter>)>>> = OnceLock::new();

fn active_slot() -> &'static Mutex<Option<(InterpKey, Arc<Interpreter>)>> {
    ACTIVE_INTERP.get_or_init(|| Mutex::new(None))
}

/// Seed/replace the active interpreter handle (run_module entry via
/// gcsupport::install). A second in-process run takes the table over —
/// the historical semantics of the thread_locals these tables replace.
pub fn set_active_interp(interp: &Arc<Interpreter>) {
    let key = interp_key(interp);
    let mut slot = active_slot().lock().unwrap();
    *slot = Some((key, interp.clone()));
}

/// The active interpreter key + handle. Panics outside a run (matches the
/// old thread_local `.expect("interpreter set for the run")` behavior).
pub fn active_interp() -> (InterpKey, Arc<Interpreter>) {
    active_slot()
        .lock()
        .unwrap()
        .clone()
        .expect("interpreter set for the run")
}

pub fn active_interp_opt() -> Option<(InterpKey, Arc<Interpreter>)> {
    active_slot().lock().unwrap().clone()
}

/// Explicit-interp variant: the key for a SPECIFIC interpreter (the census's
/// dual-handle fix — call sites that hold &Interpreter never guess).
pub fn interp_state_key(interp: &Arc<Interpreter>) -> InterpKey {
    interp_key(interp)
}

/// Run `f` against the active interpreter's slot value (clone-in/clone-out
/// for Copy-sized state; callers needing mutation use with_* variants).
pub fn with_active<T: Default, R>(
    table: &Mutex<HashMap<InterpKey, T>>,
    f: impl FnOnce(&T) -> R,
) -> R {
    let key = active_interp().0;
    let guard = table.lock().unwrap();
    let default_slot: T = T::default();
    f(guard.get(&key).unwrap_or(&default_slot))
}

pub fn with_active_mut<T: Default, R>(
    table: &Mutex<HashMap<InterpKey, T>>,
    f: impl FnOnce(&mut T) -> R,
) -> R {
    let key = active_interp().0;
    let mut guard = table.lock().unwrap();
    f(guard.entry(key).or_default())
}

/// Drop the active interpreter's rows in every registry (run teardown —
/// the tables are keyed by payload address, which RECYCLES; leaving rows
/// behind would let a future run inherit this run's sys.modules, the
/// exact cross-run hazard the census named for SYS_PATH). Called from
/// gcsupport::uninstall at run_module exit.
pub fn retire_runtime_state(interp: &Arc<Interpreter>) {
    let key = interp_key(interp);
    let _ = SYS_MODULES_T_acc().lock().unwrap().remove(&key);
    let _ = SYS_PATH_T_acc().lock().unwrap().remove(&key);
    let _ = SYS_PATH_VIEW_T_acc().lock().unwrap().remove(&key);
    let _ = SYS_MODULES_VIEW_T_acc().lock().unwrap().remove(&key);
    let _ = BOOTSTRAP_SEEDED_T_acc().lock().unwrap().remove(&key);
    let _ = MODULE_CACHE_T_acc().lock().unwrap().remove(&key);
    let _ = NAMED_CALLABLES_T_acc().lock().unwrap().remove(&key);
    let _ = ATEXIT_FNS_T_acc().lock().unwrap().remove(&key);
    let _ = INTERNED_T_acc().lock().unwrap().remove(&key);
    let _ = SD_REGISTRY_T_acc().lock().unwrap().remove(&key);
    let _ = LRU_PENDING_T_acc().lock().unwrap().remove(&key);
    let _ = LRU_STATE_T_acc().lock().unwrap().remove(&key);
    let _ = DOCS_T_acc().lock().unwrap().remove(&key);
    let _ = XPATH_OBJECTS_T_acc().lock().unwrap().remove(&key);
    let _ = RE_CACHE_T_acc().lock().unwrap().remove(&key);
    let _ = PAT_TABLE_T_acc().lock().unwrap().remove(&key);
    let _ = MATCH_TABLE_T_acc().lock().unwrap().remove(&key);
    let _ = CELL_BACKING_T_acc().lock().unwrap().remove(&key);
    let _ = MODULE_FRAME_CELLS_T_acc().lock().unwrap().remove(&key);
    let _ = CLASS_KWARGS_T_acc().lock().unwrap().remove(&key);
    let _ = CURRENT_CLASS_NS_T_acc().lock().unwrap().remove(&key);
    let _ = BRIDGE_SOURCE_T_acc().lock().unwrap().remove(&key);
    let mut slot = active_slot().lock().unwrap();
    if let Some((k, _)) = slot.as_ref() {
        if *k == key {
            *slot = None;
        }
    }
}

// The TlsRefCell import is retained for the accessor shims that still need
// thread-local scratch (LAST_MNF_NAME diagnostics — census class (a), not
// in the P3 list).
#[allow(dead_code)]
fn _tls_type_witness(_: &TlsRefCell<usize>) {}

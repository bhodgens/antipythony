//! File-based import machinery (leaf 02.04a).
//!
//! Resolution order per import statement (docs/02-04a-import-machinery-design.md):
//! 1. sys.modules cache (module identity: re-import returns the SAME object)
//! 2. native registry (02.08-lite `builtins_modules::get_or_make_module`)
//! 3. file finder over sys.path: `p/<name>/__init__.py` (package), `p/<name>.py`
//!    (module), `p/<name>/` without `__init__` (namespace package, `__path__` set)
//! 4. ModuleNotFoundError "No module named 'x'" (byte-exact CPython 3.14)
//!
//! Execution model: compile (parse -> lower), execute in a fresh namespace
//! dict with `__name__`/`__file__`/`__package__` set; sys.modules insertion
//! happens BEFORE body execution (circular-import partial visibility).
//! Packages get `__path__ = [pkg_dir]`; relative imports resolve against
//! `__package__` per CPython.
//!
//! v0 scoping (documented): module-level defs of imported modules register
//! into the extended FnRegistry; nested defs/lambdas go through the existing
//! nested-function table. Traceback source echo for imported frames reuses
//! the main module's source table (documented divergence).
//!
//! sys.path bootstrap: [script_dir, vendored_stdlib] + ANTIPTYHONY_PATH env
//! entries. `--path` on ap-run-eval appends extra entries.

use std::sync::Arc;

use ap_obj::PyObject;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::{PyString, Value};

use crate::exc_hier::ExceptionValue;
use crate::frame::{FnEntry, Frame, Interpreter};
use crate::interp::{FnRegistry, exec_block};
use crate::{Flow, builtins, builtins_modules};

type EResult = Result<Value, Flow>;

// T1 probe: temporary import-path tracing (set AP_IMPORT_TRACE=1).
pub(crate) fn ap_trace(msg: impl std::fmt::Display) {
    if std::env::var_os("AP_IMPORT_TRACE").is_some() {
        eprintln!("[ap-import] {msg}");
    }
}

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

thread_local! {
    /// 02.08-c: the module name of the most recent ModuleNotFoundError raised
    /// by the importer (importlib._bootstrap._handle_fromlist reads
    /// `exc.name` for its backwards-compat fromlist path).
    static LAST_MNF_NAME: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Record + raise a ModuleNotFoundError carrying the module name.
fn raise_mnf(name: &str, msg: String) -> Flow {
    LAST_MNF_NAME.with(|c| *c.borrow_mut() = Some(name.to_string()));
    Flow::Raise(ExceptionValue::msg("ModuleNotFoundError", msg))
}

/// The module name of the most recent importer ModuleNotFoundError.
pub fn last_mnf_name() -> Option<String> {
    LAST_MNF_NAME.with(|c| c.borrow().clone())
}

fn s(x: impl Into<String>) -> Value {
    Value::Str(PyString::new(x.into()))
}

// ---------------------------------------------------------------------------
// sys.modules / sys.path state (thread-local, per-run like MODULE_CACHE)
// ---------------------------------------------------------------------------

// 02.09a P3: SYS_MODULES / SYS_PATH / SYS_PATH_VIEW / SYS_MODULES_VIEW /
// BOOTSTRAP_SEEDED moved to the per-interp RuntimeState tables
// (runtime_state.rs). Accessors below route through the ACTIVE interp key
// (process-global Mutex<HashMap> tables) — worker threads now see the SAME
// sys.modules/sys.path as main (the audit's named (c) bug).

/// sys.path bootstrap: [script_dir] + vendored_stdlib + **fetched
/// site-packages** + ANTIPTYHONY_PATH (04.01/04.02 integration: the uv fetch
/// dir joins AFTER vendored_stdlib so a fetched package can never shadow a
/// stdlib module, and BEFORE env/extra entries — env wins for dev overrides).
/// Idempotent (re-running clears and rebuilds — tests spawn fresh processes).
pub fn bootstrap_sys_path(script_dir: &str, extra: &[String]) {
    let path = build_bootstrap_path(script_dir, extra);
    {
        use crate::runtime_state as rs;
        let table = rs::SYS_PATH_T_acc();
        let mut guard = table.lock().unwrap();
        let key = rs::active_interp_opt().map(|(k, _)| k).unwrap_or(0);
        *guard.entry(key).or_default() = path.clone();
    }
    // LIVE-list wiring: if a sys.path PyList view already exists (same-process
    // re-bootstrap), sync the fresh bootstrap content into it so the view
    // stays the single source of truth. If not, DROP any stale view so the
    // next sys_path_value() rebuilds from this bootstrap.
    let existing = {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_PATH_VIEW_T_acc(), |v| v.clone())
    };
    if let Some(l) = existing {
        let items: Vec<Value> = path.iter().map(|e| s(e.clone())).collect();
        l.clear();
        for it in items {
            l.append(it);
        }
    } else {
        // drop the stale view
        use crate::runtime_state as rs;
        rs::with_active_mut(rs::SYS_PATH_VIEW_T_acc(), |v| *v = None);
    }
}

/// sys.path bootstrap path builder (extracted; the storage moved to the
/// RuntimeState table but the ORDERED resolution is unchanged).
fn build_bootstrap_path(script_dir: &str, extra: &[String]) -> Vec<String> {
    let mut path = vec![script_dir.to_string()];
    if let Some(vd) = vendored_stdlib_dir() {
        path.push(vd);
    }
    // 04.01: fetched pure-python packages (uv `--target` install). Discovered
    // from the project lockfile walking up from the script dir (single source
    // of truth with the CLI); falls back to the cache default when a lockfile
    // records any dependency. See importer::site_packages_for_script.
    if let Some(sp) = site_packages_for_script(script_dir) {
        path.push(sp);
    }
    if let Ok(env) = std::env::var("ANTIPTYHONY_PATH") {
        for e in env.split(':') {
            if !e.is_empty() {
                path.push(e.to_string());
            }
        }
    }
    path.extend(extra.iter().cloned());
    path
}

/// The site-packages dir to join sys.path for a script in `script_dir`
/// (04.02 `run` integration; used by BOTH ap-cli `run` and ap-run-eval).
///
/// Resolution: walk up from script_dir for a project manifest. Found AND
/// recording at least one dependency (antipythony.toml `[[dependency]]`, or
/// a non-empty antipythony.lock) -> the fetched site-packages dir
/// (`$ANTIPTYHONY_CACHE/site-packages`, default
/// `~/.cache/antipythony/site-packages`). No manifest / no deps -> None (a
/// dep-free run must not put a random cache on sys.path — cache content is
/// only meaningful when the project fetched something).
pub fn site_packages_for_script(script_dir: &str) -> Option<String> {
    let dir = ap_pack::manifest::find_project_dir(std::path::Path::new(script_dir))?;
    let has_deps = match ap_pack::manifest::read_manifest(&dir) {
        Ok(Some(m)) => !m.dependencies.is_empty(),
        Ok(None) => false,
        Err(_) => false, // a malformed manifest must not crash import bootstrap
    };
    if !has_deps {
        // Lockfile fallback (manifest w/o deps recorded but lock written).
        let Ok(Some(lock)) = ap_pack::manifest::read_lock(&dir) else {
            return None;
        };
        if lock.packages.is_empty() {
            return None;
        }
    }
    let sp = ap_pack::cache::site_packages_dir()?;
    if !sp.is_dir() {
        return None;
    }
    Some(sp.to_string_lossy().into_owned())
}

/// The vendored_stdlib/ dir at the repo root (resolved relative to this
/// crate's manifest; also overridable via ANTIPTYHONY_STDLIB for installs).
pub fn vendored_stdlib_dir() -> Option<String> {
    if let Ok(v) = std::env::var("ANTIPTYHONY_STDLIB") {
        if std::path::Path::new(&v).is_dir() {
            return Some(v);
        }
    }
    // CARGO_MANIFEST_DIR points at crates/ap-run; the vendored tree sits at
    // the workspace root. Static binaries keep the build-machine path — v0
    // accepts this (Plan 04 wheel/packaging replaces the discovery).
    let cand = concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendored_stdlib");
    let p = std::path::Path::new(cand).canonicalize().ok()?;
    p.to_str().map(|s| s.to_string())
}

/// Read access for the sys module surface (`sys.path` list value). The FIRST
/// call creates the PyList, seeds it from the bootstrap Vec and installs it
/// as the LIVE view (SYS_PATH_VIEW); every later call returns that same list
/// object, so user mutations of `sys.path` persist and the importer sees them.
pub fn sys_path_value() -> Value {
    let existing = {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_PATH_VIEW_T_acc(), |v| v.clone())
    };
    if let Some(l) = existing {
        return Value::List(l);
    }
    let items: Vec<Value> = {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_PATH_T_acc(), |p| {
            p.iter().map(|e| s(e.clone())).collect()
        })
    };
    let l = ap_obj::value::PyList::from_vec(items);
    use crate::runtime_state as rs;
    rs::with_active_mut(rs::SYS_PATH_VIEW_T_acc(), |v| *v = Some(l.clone()));
    Value::List(l)
}

/// T1 (scrapy fix pass): raw sys.path entries for native package builders
/// that need to point __path__ at their vendored subpackage dirs.
///
/// LIVE-list wiring: once a `sys.path` list object exists (SYS_PATH_VIEW,
/// created by the first sys_path_value/attach_sys_extras), it IS the source
/// of truth — user code mutating sys.path at runtime (sys.path.insert/
/// .append) mutates that list, and this read (used by find_on_path and every
/// other importer lookup) sees the mutation. Falls back to the bootstrap Vec
/// for reads before sys is ever built.
pub(crate) fn sys_path_entries() -> Vec<String> {
    let view = {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_PATH_VIEW_T_acc(), |v| v.clone())
    };
    if let Some(l) = view {
        return l
            .items()
            .iter()
            .filter_map(|v| match v {
                Value::Str(sp) => Some(sp.as_str().to_string()),
                _ => None,
            })
            .collect();
    }
    use crate::runtime_state as rs;
    rs::with_active(rs::SYS_PATH_T_acc(), |p| p.clone())
}

/// `sys.modules` dict snapshot (fresh dict each read — v0).
pub fn sys_modules_value() -> Value {
    let d = PyDict::new();
    {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_MODULES_T_acc(), |m| {
            for (k, v) in m.iter() {
                let _ = d.set(s(k.clone()), v.clone());
            }
        });
    }
    Value::Dict(d)
}

/// Test seam: clear the cache (one Interpreter per process normally).
/// The live sys.path view is dropped too: the next sys_path_value() rebuilds
/// it from SYS_PATH (bootstrap_sys_path), mirroring the fresh-process runs
/// the test suite spawns. (SYS_PATH itself intentionally persists like the
/// bootstrap Vec did — reset_import_state never cleared it.)
pub fn reset_import_state() {
    use crate::runtime_state as rs;
    rs::with_active_mut(rs::SYS_MODULES_T_acc(), |m| m.clear());
    rs::with_active_mut(rs::SYS_PATH_VIEW_T_acc(), |v| *v = None);
}

/// Public read for the Import binding (top-package fetch) and the ap-gc
/// root scanner (sys.modules VALUES are collection roots).
pub fn sys_modules_values_pub() -> Vec<Value> {
    use crate::runtime_state as rs;
    rs::with_active(rs::SYS_MODULES_T_acc(), |m| m.values().cloned().collect())
}

pub fn sys_modules_get(name: &str) -> Option<Value> {
    use crate::runtime_state as rs;
    rs::with_active(rs::SYS_MODULES_T_acc(), |m| m.get(name).cloned())
}

fn sys_modules_insert(name: &str, v: &Value) {
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg)
        && name.contains("_py_warnings")
    {
        if let Value::Obj(o) = v {
            eprintln!("[INSDBG] insert {name} obj={:p}", std::sync::Arc::as_ptr(o));
        }
    }
    {
        use crate::runtime_state as rs;
        rs::with_active_mut(rs::SYS_MODULES_T_acc(), |m| {
            m.insert(name.to_string(), v.clone())
        });
    }
    // 02.04a fix pass (BUG-C): keep the PYTHON-VISIBLE sys.modules dict in
    // sync — it was a snapshot taken when sys was built, so
    // `sys.modules[__name__]` inside a freshly-imported module (warnings.py
    // line 96) raised KeyError for every module imported after sys.
    {
        use crate::runtime_state as rs;
        rs::with_active(rs::SYS_MODULES_VIEW_T_acc(), |m| {
            if let Some(d) = m.as_ref() {
                let _ = d.set(s(name.to_string()), v.clone());
            }
        });
    }
}

// ---------------------------------------------------------------------------
// File finder
// ---------------------------------------------------------------------------

/// What the file finder found on sys.path.
enum Found {
    /// Directory package with __init__.py.
    Package { dir: String, init: String },
    /// Directory WITHOUT __init__.py (namespace package, design v1).
    Namespace { dir: String },
    /// Single-file module.
    Module { file: String },
}

/// Robustness-ci leaf 02.02 (review finding #16): a module-name segment is
/// safe to join onto a sys.path directory only if it is neither empty, nor
/// `.`/`..`, nor contains a path separator. Pre-guard, `find_on_path("")`
/// joined the empty segment and returned the sys.path BASE DIRECTORY as a
/// namespace package (`__import__('')` "succeeded" with `__path__ = [base]`)
/// — a filesystem-traversal surface for computed names.
///
/// The parser cannot currently produce such names; this guard is the
/// defense for the `__import__`-with-a-computed-string path (builtins.rs
/// routes arbitrary str args straight into `import_module`).
///
/// The byte-exact CPython 3.14 error texts for the rejected shapes
/// (`ValueError: Empty module name`, `ModuleNotFoundError: No module named
/// '.'`, the `; 'os' is not a package` suffix form, etc.) are raised
/// upstream in `import_file_module` where the dotted-prefix context is
/// known; the find functions themselves simply refuse the join.
/// Probes pinned in `tests/import_guard_differential.rs`.
fn segment_is_safe(seg: &str) -> bool {
    !seg.is_empty() && seg != "." && seg != ".." && !seg.contains('/') && !seg.contains('\\')
}

/// Python `repr()` for the module name embedded in CPython's
/// ModuleNotFoundError text (leaf 02.02 probe-pinned: `__import__('a\\b')`
/// prints `No module named 'a\\b'` — doubled backslash — and `__import__("a'q")`
/// switches to double quotes). CPython's repr rules for short printable
/// strings: single quotes unless the string contains `'` and no `"`; escape
/// `\`, the active quote, and \\n \\r \\t. Names the parser can produce
/// never hit an escape, so existing goldens are unaffected.
fn py_name_repr(s: &str) -> String {
    let has_sq = s.contains('\'');
    let has_dq = s.contains('"');
    let q = if has_sq && !has_dq { '"' } else { '\'' };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            // CPython repr escapes remaining ASCII control chars as \xNN
            // (probe: __import__('a\x08b') -> "No module named 'a\\x08b'").
            c if (c < '\x20' || c == '\x7f') => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn find_on_path(name: &str) -> Option<Found> {
    if !segment_is_safe(name) {
        return None;
    }
    // LIVE sys.path (see sys_path_entries): runtime insert/append visible.
    let entries = sys_path_entries();
    // 02.08-c: the zope.interface stub must win even when the gate probe
    // inserts site-packages at sys.path[0] (the real zope's ro.py descriptor
    // identity-scan is out of v1 scope — docs/lxml-shim.md). Search the
    // vendored dir FIRST for the zope top-level package.
    if name == "zope" {
        if let Some(vd) = vendored_stdlib_dir() {
            let dir = std::path::Path::new(&vd).join("zope");
            let init = dir.join("__init__.py");
            if init.is_file() {
                return Some(Found::Package {
                    dir: dir.to_string_lossy().into_owned(),
                    init: init.to_string_lossy().into_owned(),
                });
            }
        }
    }
    for base in entries {
        let bp = std::path::Path::new(&base);
        let dir = bp.join(name);
        if dir.is_dir() {
            let init = dir.join("__init__.py");
            if init.is_file() {
                return Some(Found::Package {
                    dir: dir.to_string_lossy().into_owned(),
                    init: init.to_string_lossy().into_owned(),
                });
            }
            return Some(Found::Namespace {
                dir: dir.to_string_lossy().into_owned(),
            });
        }
        let file = bp.join(format!("{name}.py"));
        if file.is_file() {
            return Some(Found::Module {
                file: file.to_string_lossy().into_owned(),
            });
        }
    }
    None
}

fn find_submodule(parent_dir: &str, name: &str) -> Option<Found> {
    // Robustness-ci 02.02: same join guard as find_on_path (see there).
    if !segment_is_safe(name) {
        return None;
    }
    let bp = std::path::Path::new(parent_dir);
    let dir = bp.join(name);
    if dir.is_dir() {
        let init = dir.join("__init__.py");
        if init.is_file() {
            return Some(Found::Package {
                dir: dir.to_string_lossy().into_owned(),
                init: init.to_string_lossy().into_owned(),
            });
        }
        return Some(Found::Namespace {
            dir: dir.to_string_lossy().into_owned(),
        });
    }
    let file = bp.join(format!("{name}.py"));
    if file.is_file() {
        return Some(Found::Module {
            file: file.to_string_lossy().into_owned(),
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Module execution
// ---------------------------------------------------------------------------

/// Per-import compile result: the lowered module + scopes for registry extend.
struct Compiled {
    module: ap_ir::Module,
    scopes: ap_parse::ScopeInfoModule,
    is_package: bool,
    dir: String,
}

fn compile_file(path: &str, is_package: bool, dir: String) -> Result<Compiled, Flow> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| raise("ImportError", format!("cannot read {path}: {e}")))?;
    let parsed = ap_parse::parse(&source).map_err(|e| {
        // SyntaxError inside an imported module: CPython surfaces the
        // SyntaxError directly (no wrapping ImportError).
        raise(
            "SyntaxError",
            format!("{} (line {}): {}", path, e.line, e.message),
        )
    })?;
    let (module, scopes) =
        ap_parse::lower(&parsed).map_err(|e| raise("SyntaxError", format!("{e}")))?;
    Ok(Compiled {
        module,
        scopes,
        is_package,
        dir,
    })
}

/// A namespace created for an in-flight module execution: dict + frame +
/// registry share the module's globals dict; the dict IS the module ns.
struct Loaded {
    ns: Gc<PyDict>,
    frame: Arc<Frame>,
    reg: Arc<FnRegistry>,
}

fn load_into(
    interp: &Arc<Interpreter>,
    compiled: &Compiled,
    name: &str,
    file: &str,
    package: &str,
) -> Result<Loaded, Flow> {
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s(name));
    let _ = ns.set(s("__file__"), s(file));
    let _ = ns.set(s("__package__"), s(package));
    let _ = ns.set(s("__doc__"), Value::None);
    if compiled.is_package {
        // __path__ = [pkg_dir] (CPython: a list of str).
        let _ = ns.set(
            s("__path__"),
            Value::List(ap_obj::value::PyList::from_vec(vec![s(compiled
                .dir
                .clone())])),
        );
    }
    // Extend the FnRegistry with this module's defs, bound to THIS module's
    // namespace as their globals.
    let reg = FnRegistry::extend_current(
        interp,
        &compiled.module.items,
        compiled.scopes.clone(),
        ns.clone(),
    );
    // Module frames keep locals empty; stores land in the globals dict (= ns).
    let frame = Arc::new(Frame::new_module(ns.clone()));
    // B5 (memperf audit): the module frame carries THIS module's scope
    // table so nested-def classification (and the importer's cell seeding
    // below) resolves exactly, never through another module's same-path
    // shadow (the twisted deprecate.py NameError collision class).
    frame.scope_table.set(reg.last_table());
    // Bind module-level defs through the EXTENDED registry so their fn ids
    // resolve (mirror of interp::exec_module's FnDef arm).
    let mut stmts: Vec<ap_ir::Stmt> = Vec::new();
    for item in &compiled.module.items {
        match item {
            ap_ir::Item::Stmt(st) => stmts.push(st.clone()),
            ap_ir::Item::FnDef(f) => {
                // T1-d fix (ast.py `from _ast import *` then `def parse`):
                // ALL module-level defs now defer to source position (the
                // old eager binding ran BEFORE body exec, so a star import
                // clobbered same-named defs). Register the defining frame
                // eagerly so eval_default resolves module globals.
                let id = reg.by_path_lookup(&f.name).unwrap_or(0);
                interp
                    .defining_frames
                    .borrow_mut()
                    .insert(id, frame.clone());
                stmts.push(ap_ir::Stmt::LocalDef(Box::new(f.clone())));
            }
            ap_ir::Item::ClassDef(c) => {
                stmts.push(ap_ir::Stmt::LocalClass(Box::new(c.clone())));
            }
        }
    }
    Ok(Loaded { ns, frame, reg })
}

/// Execute an already-inserted module namespace (sys.modules holds the
/// partial module for circular imports).
fn run_loaded(
    interp: &Arc<Interpreter>,
    loaded: &Loaded,
    stmts: &[ap_ir::Stmt],
) -> Result<(), Flow> {
    // s022: builtins globals()/locals() must see the MODULE ns while its body
    // runs (token.py builds tok_name from globals().items()).
    let saved = builtins_modules::CURRENT_MODULE_GLOBALS
        .with(|c| c.borrow_mut().replace(loaded.frame.globals.clone()));
    let r = exec_block(interp, &loaded.frame, stmts, &loaded.reg);
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg) {
        if let Err(ref _e) = r {
            eprintln!("[MODDBG] module exec failed (see traceback above)");
        }
    }
    builtins_modules::CURRENT_MODULE_GLOBALS.with(|c| *c.borrow_mut() = saved);
    r
}

// ---------------------------------------------------------------------------
// The import entry points (called from interp.rs Import/ImportFrom arms)
// ---------------------------------------------------------------------------

/// Import a module by absolute dotted name; returns the module object (the
/// TOP package for dotted names — the caller binds per CPython rules).
/// `interp` is needed for registry extension + builtins dispatch.
pub fn import_module(interp: &Arc<Interpreter>, name: &str) -> EResult {
    ap_trace(format!("import_module({name})"));
    builtins_modules::set_current_interp(interp);
    // T2 (pytest PytestPluginManager.__init__: nested `from _pytest.assertion
    // import ...` between the class entry and its zero-arg super()): a module
    // exec'd during the import can leave the LIFO super_stack imbalanced
    // (generator sinks / early returns inside the imported body), which
    // emptied the CALLER's staged __init__ context. Snapshot and restore the
    // stack around the whole import so the enclosing method's context survives
    // anything the imported module does.
    let saved_super: Vec<(std::sync::Arc<ap_obj::classes::TypeRef>, Value, bool)> =
        interp.super_stack.borrow().iter().cloned().collect();
    let _super_guard = SuperStackGuard {
        interp: std::sync::Arc::downgrade(interp),
        saved: saved_super,
    };
    // 02.03b-fix: seed the bootstrap builtins ONCE — importlib/_bootstrap's
    // _setup skips _builtin_from_name for names already in sys.modules
    // (CPython: real builtins are always present there).
    // 02.09a P3: the once-flag lives in the RuntimeState table (was a
    // thread_local Cell — a worker re-seeded bootstrap modules into ITS
    // private SYS_MODULES, breaking `is` identity across threads).
    {
        use crate::runtime_state as rs;
        let seeded = rs::with_active_mut(rs::BOOTSTRAP_SEEDED_T_acc(), |c| {
            if !*c {
                *c = true;
                true
            } else {
                false
            }
        });
        if seeded {
            for n in ["_thread", "_warnings", "_weakref", "_imp"] {
                if sys_modules_get(n).is_none() {
                    if let Ok(m) = import_module_inner(interp, n) {
                        sys_modules_insert(n, &m);
                    }
                }
            }
            // 02.03b-fix: __main__ in sys.modules (CPython always has it;
            // importlib/_bootstrap and globals-echo paths read it).
            if sys_modules_get("__main__").is_none() {
                let m = ap_obj::PyObject::new(ap_obj::ObjKind::Module, "__main__");
                *m.module_ns.borrow_mut() = Some(interp.globals.clone());
                let _ = interp.globals.set(
                    Value::Str(PyString::new("__name__")),
                    Value::Str(PyString::new("__main__")),
                );
                sys_modules_insert("__main__", &Value::Obj(m));
            }
        }
    }
    import_module_inner(interp, name)
}

/// T2: RAII restore of the zero-arg-super stack across a module import (the
/// imported body may imbalanced-pop the caller's staged method context).
struct SuperStackGuard {
    interp: std::sync::Weak<Interpreter>,
    saved: Vec<(std::sync::Arc<ap_obj::classes::TypeRef>, Value, bool)>,
}
impl Drop for SuperStackGuard {
    fn drop(&mut self) {
        if let Some(rc) = self.interp.upgrade() {
            let mut st = rc.super_stack.borrow_mut();
            if st.len() != self.saved.len() {
                st.clear();
                for entry in self.saved.drain(..) {
                    st.push(entry);
                }
            }
        }
    }
}

pub fn import_module_inner(interp: &Arc<Interpreter>, name: &str) -> EResult {
    ap_trace(format!("import_module_inner({name})"));
    // 1. sys.modules cache.
    if let Some(m) = sys_modules_get(name) {
        link_parent_attr(name, &m);
        return Ok(m);
    }
    // 2. Native registry (02.08-lite; verified quality wins per decision).
    // For the five names whose vendored CPython sources are also present,
    // AUGMENT the native module with any names only the file defines
    // (private helpers like posixpath._splitext that the vendored import
    // chain calls). Identity is preserved — user code still sees the native
    // module object.
    if matches!(
        name,
        "posixpath" | "genericpath" | "stat" | "keyword" | "linecache" | "collections"
        // T1-d (packaging._elffile etc read functools._unwrap_partialmethod
        // — private CPython-stdlib names the native surface lacks).
        | "functools"
        // Issue #13: the mbc MIXIN classes (MultibyteIncrementalEncoder/
        // Decoder, StreamReader/Writer) live in the vendored _multibytecodec.py
        // over the native engine bridges; the vendored file must exec INTO the
        // native module so wrapper class bases resolve.
        | "_multibytecodec"
    ) {
        if let Some(m) = builtins_modules::get_or_make_module(interp, name) {
            augment_with_vendored(interp, name, &m);
            sys_modules_insert(name, &m);
            return Ok(m);
        }
    }
    // W114: _ast with REAL classes - exec the vendored _ast.py
    // (class hierarchy + from_native converter over the ast_bridge
    // generic dicts). compile()'s ONLY_AST arm calls
    // _ast.from_native(ast_bridge::native_parse_tree(..)).
    if name == "_ast" {
        for base in sys_path_entries() {
            let f = std::path::Path::new(&base).join("_ast.py");
            if f.is_file() {
                if let Ok(compiled) = compile_file(&f.to_string_lossy(), false, String::new()) {
                    if let Ok(loaded) =
                        load_into(interp, &compiled, name, &f.to_string_lossy(), name)
                    {
                        // W117 leak fix: CPython circular-import semantics —
                        // the module object enters sys.modules BEFORE its
                        // body runs (same contract as exec_compiled). The
                        // W114 _ast.py self-imports (`import _ast as
                        // _ast_mod`); with the old insert-AFTER-exec order
                        // the self-import missed sys.modules, re-entered
                        // this branch, and re-ran the whole ~100-class
                        // module body one level deeper — unbounded
                        // recursion, each level cloning the FnRegistry and
                        // Box::leak-ing a merged scope table (GC-immune
                        // ~150MB/level, fenced at 8.6GB). Pre-inserting the
                        // partial module makes the self-import a
                        // sys.modules hit; body writes are visible because
                        // module_ns SHARES loaded.ns (not a snapshot).
                        let m = PyObject::new(ap_obj::ObjKind::Module, name);
                        let mv = Value::Obj(m.clone());
                        *m.module_ns.borrow_mut() = Some(loaded.ns.clone());
                        sys_modules_insert(name, &mv);
                        let stmts = module_stmts(interp, &compiled, &loaded.reg, &loaded.frame);
                        let outcome = run_loaded(interp, &loaded, &stmts);
                        // Refresh the ns view post-body (share, like
                        // exec_compiled) and drop the partial module on a
                        // failed body so the native/next-base fallback
                        // re-imports cleanly.
                        *m.module_ns.borrow_mut() = Some(loaded.ns.clone());
                        if outcome.is_ok() {
                            return Ok(mv);
                        }
                        {
                            use crate::runtime_state as rs;
                            rs::with_active_mut(rs::SYS_MODULES_T_acc(), |sm| {
                                sm.remove(name);
                            });
                            rs::with_active(rs::SYS_MODULES_VIEW_T_acc(), |v| {
                                if let Some(d) = v.as_ref() {
                                    let _ = d.pop(&s(name.to_string()));
                                }
                            });
                        }
                    }
                }
            }
        }
    }
    // 3. Native registry (02.08-lite continued).
    // T1 (scrapy fix pass): collections.abc — exec the vendored
    // _collections_abc.py INTO a "collections.abc" module (CPython's
    // collections/abc.py is exactly `from _collections_abc import *`; the
    // vendored tree has the flat _collections_abc.py). Without this the
    // native-parent descent returns an empty module and typing's
    // Hashable/Mapping probes die.
    if name == "collections.abc" {
        // T1: the parent `collections` must be importable + bound first
        // (CPython walks packages; `import collections.abc` makes a later
        // bare `import collections` a sys.modules hit and binds the top pkg).
        if sys_modules_get("collections").is_none() {
            if let Some(m) = builtins_modules::get_or_make_module(interp, "collections") {
                sys_modules_insert("collections", &m);
            }
        }
        for base in sys_path_entries() {
            let f = std::path::Path::new(&base).join("_collections_abc.py");
            if f.is_file() {
                if let Ok(compiled) = compile_file(&f.to_string_lossy(), false, String::new()) {
                    if let Ok(loaded) =
                        load_into(interp, &compiled, name, &f.to_string_lossy(), "collections")
                    {
                        let stmts = module_stmts(interp, &compiled, &loaded.reg, &loaded.frame);
                        if run_loaded(interp, &loaded, &stmts).is_ok() {
                            // Build the module object fresh from the loaded
                            // ns (a stale empty entry may exist in
                            // sys.modules - overwrite it).
                            let (m, mns) = builtins_modules::new_module(name);
                            let _ = mns.set(s("__name__"), s(name));
                            for (k, v) in loaded.ns.items() {
                                let _ = mns.set(k, v);
                            }
                            *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*mns).clone()));
                            let mv = Value::Obj(m);
                            link_parent_attr(name, &mv);
                            sys_modules_insert(name, &mv);
                            return Ok(mv);
                        }
                    }
                }
            }
        }
    }
    // T1 (scrapy fix pass): DOTTED names try the file finder FIRST when the
    // parent package exposes __path__ (native importlib now points at the
    // vendored dir), so `import importlib.resources` resolves the vendored
    // subpackage instead of the native-parent early return.
    // 02.08-c: EXCEPT registered native stubs (zope.interface.*) — the gate
    // probe inserts site-packages at sys.path[0] and the REAL zope package
    // would shadow the shim; the native registry must win (docs/lxml-shim.md).
    if name.contains('.') && !builtins_modules::is_native_module(name) {
        match import_file_module(interp, name) {
            Ok(m) => {
                ap_trace(format!("dotted {name}: file finder OK"));
                sys_modules_insert(name, &m);
                return Ok(m);
            }
            Err(f) => {
                ap_trace(format!("dotted {name}: file finder FAILED (flow raised)"));
                return Err(f);
            }
        }
    }
    if let Some(m) = builtins_modules::get_or_make_module(interp, name) {
        // T1 fix: native modules with a vendored package dir (importlib)
        // carry __path__ so later dotted children resolve through the file
        // finder (CPython _find_spec walks parent.__path__).
        attach_native_vendored_path(&m);
        // T1-d: a dotted NATIVE import must have its ancestor prefixes in
        // sys.modules too (inspect's `import importlib.machinery` then
        // binds top `importlib` in the importer namespace; CPython inserts
        // every prefix during the walk).
        let parts: Vec<&str> = name.split('.').collect();
        if parts.len() > 1 {
            for i in 1..parts.len() {
                let anc = parts[..i].join(".");
                if sys_modules_get(&anc).is_none() {
                    if let Some(am) = builtins_modules::get_or_make_module(interp, &anc) {
                        attach_native_vendored_path(&am);
                        sys_modules_insert(&anc, &am);
                        link_parent_attr(&anc, &am);
                    }
                }
            }
        }
        link_parent_attr(name, &m);
        sys_modules_insert(name, &m);
        return Ok(m);
    }
    // 02.08-c: importlib.abc probes `import _frozen_importlib[ _external]`
    // and compares `exc.name`; CPython has these frozen. Alias them to the
    // vendored importlib._bootstrap[ _external] modules.
    if name == "_frozen_importlib" || name == "_frozen_importlib_external" {
        let alias_target = if name == "_frozen_importlib" {
            "importlib._bootstrap"
        } else {
            "importlib._bootstrap_external"
        };
        if let Ok(m) = import_module_inner(interp, alias_target) {
            sys_modules_insert(name, &m);
            // T2: CPython runs _bootstrap_external._install(...) at
            // startup — it registers FileFinder in sys.path_hooks and
            // PathFinder in sys.meta_path. Without it, sys.path_hooks is
            // empty and importlib.machinery.PathFinder/importlib.metadata
            // find nothing on sys.path.
            if name == "_frozen_importlib_external" {
                let frozen = sys_modules_get("_frozen_importlib");
                let ext = sys_modules_get(name);
                if let (Some(Value::Obj(frozen_m)), Some(Value::Obj(ext_m))) = (frozen, ext) {
                    let install = ext_m
                        .module_ns
                        .borrow()
                        .as_ref()
                        .and_then(|ns| ns.get(&s("_install")));
                    if let Some(install) = install {
                        let frozen_v = Value::Obj(frozen_m);
                        let _ = crate::interp::call_value_pub3(&install, &[frozen_v]);
                    }
                }
            }
            return Ok(m);
        }
    }
    if name == "sys" {
        let m = builtins::get_or_make_sys(interp);
        // sys.path/sys.modules visible on the module surface.
        attach_sys_extras(interp, &m);
        sys_modules_insert("sys", &m);
        return Ok(m);
    }
    if name == "builtins" {
        let m = builtins_module(interp);
        sys_modules_insert("builtins", &m);
        return Ok(m);
    }
    if name == "_thread" {
        let m = thread_stub_module(interp);
        sys_modules_insert("_thread", &m);
        return Ok(m);
    }
    if name == "_weakref" {
        let m = weakref_stub_module();
        sys_modules_insert("_weakref", &m);
        return Ok(m);
    }
    if name == "_typing" {
        let m = typing_stub_module(interp);
        sys_modules_insert("_typing", &m);
        return Ok(m);
    }
    // 3. File finder, descending packages ("a.b.c" resolves through a, a.b).
    import_file_module(interp, name)
}

pub(crate) fn import_file_module(interp: &Arc<Interpreter>, name: &str) -> EResult {
    ap_trace(format!("import_file_module({name})"));
    // --- robustness-ci leaf 02.02: module-name path-segment guard ----------
    // (review finding #16). No unsafe segment ('', '.', '..', or one
    // containing '/' or '\') may reach a sys.path join in find_on_path /
    // find_submodule. The error texts below are byte-pinned against live
    // CPython 3.14 (`__import__(name)`, level=0); probes reproduced in
    // crates/ap-run/tests/import_guard_differential.rs.
    if name.is_empty() {
        // CPython _sanity_check: ValueError('Empty module name') BEFORE any
        // path lookup or finder join.
        return Err(raise("ValueError", "Empty module name"));
    }
    if name.starts_with('.') {
        // A leading-dot name is never absolute-importable. Fresh-process
        // CPython behavior (pinned):
        // - '..', '...', '..a', '../evil' -> ModuleNotFoundError: No module
        //   named '.' (the rpartition parent of the leading '..' is '.').
        // - '.x' where the first segment x resolves on the absolute path
        //   ('.os', '.json', '.keyword', '.os.path') -> ValueError: Empty
        //   module name (CPython loads x's source under the alias, then
        //   __import__ re-imports name.partition('.')[0] == "").
        // - '.x' where x does not resolve ('.a', '.a.b', '.nonexistentpkg',
        //   '.os/', '.1') -> ModuleNotFoundError: No module named '.x' up to
        //   the first dot (the failing parent-prefix report).
        if name.starts_with("..") {
            return Err(raise_mnf(".", "No module named '.'".to_string()));
        }
        let head = name.strip_prefix('.').unwrap_or(name);
        let first = head.split('.').next().unwrap_or("");
        // CPython's '.x' quirk fires when the TAIL after the last dot (== x
        // for a single leading dot) matches a finder spec (os.py on
        // sys.path, native json, ...) — sys.modules CACHE membership does
        // not count (probe: '.sys' -> No module named '.sys' even though
        // sys is always cached).
        let resolves = segment_is_safe(first)
            && (find_on_path(first).is_some()
                || builtins_modules::get_or_make_module(interp, first).is_some());
        if resolves {
            return Err(raise("ValueError", "Empty module name"));
        }
        let reported = match head.find('.') {
            Some(i) => format!(".{}", &head[..i]),
            None => name.to_string(),
        };
        return Err(raise_mnf(
            &reported,
            format!("No module named {}", py_name_repr(&reported)),
        ));
    }
    let parts: Vec<&str> = name.split('.').collect();
    // Walk the dotted path, importing each parent package first.
    let mut prefix: Vec<&str> = Vec::new();
    let mut parent_dir: Option<String> = None;
    for part in &parts {
        prefix.push(part);
        let full = prefix.join(".");
        if let Some(m) = sys_modules_get(&full) {
            // Already loaded (or partial from a circular import): continue
            // descent using its __path__ when present. Ensure the parent
            // link exists (the cached module may have loaded through
            // import_from without the attribute ever being set).
            if let (Some((parent_name, child_name)), Value::Obj(_po)) = (full.rsplit_once('.'), &m)
            {
                if let Some(Value::Obj(pp)) = sys_modules_get(parent_name) {
                    if let Some(pns) = &*pp.module_ns.borrow_mut() {
                        let _ = pns.set(s(child_name), m.clone());
                    }
                }
            }
            parent_dir = module_path_dir(&m);
            continue;
        }
        // Native registry dotted names (os.path under native os): the parent
        // resolved natively (no __path__) — delegate the whole remainder.
        // Native-registry precedence stays intact (importlib.util,
        // importlib.machinery, os.path resolve native, never vendored); only
        // names with NO native surface fall through to the file finder.
        if parent_dir.is_none() && !prefix.is_empty() {
            if let Some(m) = builtins_modules::get_or_make_module(interp, &full) {
                // T1: CPython puts EVERY walked prefix in sys.modules
                // (import collections.abc makes 'collections' importable and
                // binds the top package). Without this a later bare
                // `import collections` re-runs the vendored collections.py
                // exec and dies on native-only surfaces.
                attach_native_vendored_path(&m);
                // T1-d: ancestors FIRST (importlib.machinery requires
                // 'importlib' in sys.modules + linked before the leaf).
                for i in 1..prefix.len() {
                    let anc = prefix[..i].join(".");
                    if sys_modules_get(&anc).is_none() {
                        if let Some(am) = builtins_modules::get_or_make_module(interp, &anc) {
                            attach_native_vendored_path(&am);
                            sys_modules_insert(&anc, &am);
                            link_parent_attr(&anc, &am);
                        }
                    }
                }
                sys_modules_insert(&full, &m);
                link_parent_attr(&full, &m);
                continue;
            }
        }
        let found = match &parent_dir {
            Some(dir) => find_submodule(dir, part),
            // T1 fix: native parent with no dotted registry entry — CPython's
            // _find_spec walks the PARENT's __path__ for the child segment
            // (never a fresh top-level search). Native parents carry __path__
            // pointing at their vendored package dir (attach helper), so the
            // file finder must search there.
            None => {
                let parent_name = if prefix.len() >= 2 {
                    Some(prefix[..prefix.len() - 1].join("."))
                } else {
                    None
                };
                match parent_name
                    .and_then(|p| sys_modules_get(&p))
                    .as_ref()
                    .and_then(module_path_dir)
                {
                    // (a) parent has __path__ (native w/ vendored dir or a
                    // file-finder package that lost the race): search it.
                    Some(dir) => find_submodule(&dir, part),
                    // (b) parent is native, on sys.modules, no __path__: not
                    // a package — CPython message is the is-not-a-package
                    // form, but the pre-existing fallthrough to find_on_path
                    // stays for native json.decoder / os.path fidelity (see
                    // the leaf 02.02 note below).
                    None => find_on_path(part),
                }
            }
        };
        let found = match found {
            Some(f) => Some(f),
            None => {
                // 04.01 (CPython _handle_fromlist semantics): a submodule the
                // file finder can't see may exist as a PRE-BUILT module
                // ATTRIBUTE of the parent — six.py registers six.moves as a
                // _LazyModule instance on the six module (its meta-path
                // importer is out of our v0 scope). When the parent module
                // object carries a module-ns attr named `part`, adopt it as
                // the submodule (sys.modules identity from here on).
                let mut adopted = false;
                if let Some((pname, _)) = full.rsplit_once('.') {
                    if let Some(Value::Obj(pp)) = sys_modules_get(pname) {
                        let attr = pp
                            .module_ns
                            .borrow()
                            .as_ref()
                            .and_then(|ns| ns.get(&s(part.to_string())));
                        if let Some(Value::Obj(am)) = attr {
                            let is_module = am.kind == ap_obj::ObjKind::Module
                                || am
                                    .type_ref
                                    .borrow()
                                    .as_ref()
                                    .and_then(|tr| tr.upgrade())
                                    .map(|tr| tr.mro.borrow().iter().any(|m| m.name == "module"))
                                    .unwrap_or(false);
                            if is_module {
                                let mv = Value::Obj(am);
                                sys_modules_insert(&full, &mv);
                                parent_dir = module_path_dir(&mv);
                                link_parent_attr(&full, &mv);
                                adopted = true;
                            }
                        }
                    }
                }
                if adopted {
                    continue;
                }
                None
            }
        };
        let found = match found {
            Some(f) => f,
            None => {
                // 02.02: CPython embeds repr(name) ("No module named
                // 'a\\b'"), so segments the finder refused for containing
                // backslashes print doubled, byte-exact.
                return Err(raise_mnf(
                    &full,
                    format!("No module named {}", py_name_repr(&full)),
                ));
            }
        };
        exec_found(interp, &full, &found)?;
        let m = sys_modules_get(&full).expect("exec_found inserted");
        ap_trace(format!("walk: exec {full} OK"));
        // CPython: the submodule becomes an attribute of the parent package
        // (import a.b makes a.b reachable as a.b after `import a.b`).
        if let (Some((parent_name, child_name)), Value::Obj(_po)) = (full.rsplit_once('.'), &m) {
            if let Some(Value::Obj(pp)) = sys_modules_get(parent_name) {
                // W112: try_borrow_mut — a re-entrant import (module body
                // importing a sibling while the parent link is already
                // held) panics the whole run on a plain borrow_mut; the
                // link is idempotent so skipping a contended write is
                // safe (the winner sets the same value).
                if let Ok(pns_ref) = pp.module_ns.try_borrow_mut() {
                    if let Some(pns) = &*pns_ref {
                        let _ = pns.set(s(child_name), m.clone());
                    }
                }
            }
        }
        parent_dir = module_path_dir(&m);
    }
    sys_modules_get(name)
        .ok_or_else(|| raise_mnf(name, format!("No module named {}", py_name_repr(name))))
}

/// T1 fix: file-loaded packages get a minimal `__spec__` (name/origin/
/// submodule_search_locations/loader). importlib.resources.files() reads
/// package.__spec__ (wrap_spec → SpecLoaderAdapter → get_resource_reader);
/// CPython source loaders always provide one. `loader` stays unset — the
/// vendored resources chain suppress()es the AttributeError and falls back
/// to CompatibilityFiles.files() (a path-backed Traversable), matching the
/// CPython behavior our probe pins (files(...) is a PosixPath, not callable).
fn attach_module_spec(name: &str) {
    if let Some(Value::Obj(m)) = sys_modules_get(name) {
        if let Some(ns) = &*m.module_ns.borrow_mut() {
            if ns.get(&s("__spec__")).is_some() {
                return;
            }
        }
        let spec = PyObject::new(ap_obj::ObjKind::Namespace, "ModuleSpec");
        {
            let attrs = spec.attrs.borrow_mut();
            let _ = attrs.set(s("name"), s(name));
            let _ = attrs.set(s("origin"), Value::None);
        }
        // T1: CPython's SourceFileLoader implements get_resource_reader, so
        // importlib.resources.files() returns a pathlib.Path for on-disk
        // packages. Model the loader + reader as native namespaces; the
        // reader's files() marker dispatches to pathlib.Path(pkg_dir).
        {
            let loader = PyObject::new(ap_obj::ObjKind::Namespace, "SourceFileLoader");
            let reader = PyObject::new(ap_obj::ObjKind::Namespace, "ResourceReader");
            if let Some(Value::List(pl)) = {
                let b = m.module_ns.borrow();
                b.as_ref().and_then(|ns| ns.get(&s("__path__")))
            } {
                if let Some(Value::Str(pdir)) = pl.items().first() {
                    let files_fn =
                        PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_resreader__.files");
                    *files_fn.slice_parts.borrow_mut() = vec![s(pdir.as_str().to_string())];
                    let _ = reader
                        .attrs
                        .borrow_mut()
                        .set(s("files"), Value::Obj(files_fn));
                    let get_rr = PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "__ap_loader__.get_resource_reader",
                    );
                    *get_rr.slice_parts.borrow_mut() = vec![Value::Obj(reader.clone())];
                    let _ = loader
                        .attrs
                        .borrow_mut()
                        .set(s("get_resource_reader"), Value::Obj(get_rr));
                    let _ = spec.attrs.borrow_mut().set(s("loader"), Value::Obj(loader));
                }
            }
        }
        let sv = Value::Obj(spec.clone());
        if let Some(ns) = &*m.module_ns.borrow_mut() {
            if let Some(Value::List(pl)) = ns.get(&s("__path__")) {
                if let Some(Value::Str(p)) = pl.items().first() {
                    let _ = spec.attrs.borrow_mut().set(
                        s("submodule_search_locations"),
                        Value::List(ap_obj::value::PyList::from_vec(vec![s(p
                            .as_str()
                            .to_string())])),
                    );
                }
            }
        }
        if let Some(ns) = &*m.module_ns.borrow_mut() {
            let _ = ns.set(s("__spec__"), sv);
        }
    }
}

/// T1 fix: give a native module a `__path__` when its top-level name has a
/// vendored package directory (`vendored_stdlib/<top>/`). CPython's
/// _find_spec resolves dotted children by walking parent.__path__; native
/// modules with vendored subpackages (importlib — importlib.resources) need
/// the same surface or the child search degenerates to a top-level lookup.
/// Idempotent: never overwrites an existing __path__ (the importlib builder
/// already sets one; this generalizes to any future native top-level pkg).
fn attach_native_vendored_path(m: &Value) {
    let Value::Obj(o) = m else { return };
    if o.kind != ap_obj::ObjKind::Module {
        return;
    }
    let Some(ns) = &*o.module_ns.borrow() else {
        return;
    };
    let Some(Value::Str(name)) = ns.get(&s("__name__")) else {
        return;
    };
    // Only top-level native names can have a vendored dir.
    let top = match name.as_str().split_once('.') {
        Some((t, _)) => t,
        None => name.as_str(),
    };
    if ns.get(&s("__path__")).is_some() {
        return;
    }
    for base in sys_path_entries() {
        let dir = std::path::Path::new(&base).join(top);
        if dir.join("__init__.py").is_file() {
            let _ = ns.set(
                s("__path__"),
                Value::List(ap_obj::value::PyList::from_vec(vec![s(dir
                    .to_string_lossy()
                    .into_owned())])),
            );
            return;
        }
    }
}

/// Extract `[pkg_dir]` from a module value's `__path__` (packages only).
fn module_path_dir(m: &Value) -> Option<String> {
    if let Value::Obj(o) = m {
        let ns = o.module_ns.borrow();
        if let Some(ns) = &*ns {
            let pv = ns.get(&s("__path__"))?;
            if let Value::List(l) = pv {
                let items = l.items();
                if let Some(Value::Str(sp)) = items.first() {
                    return Some(sp.as_str().to_string());
                }
            }
        }
    }
    None
}

fn exec_found(interp: &Arc<Interpreter>, name: &str, found: &Found) -> Result<(), Flow> {
    match found {
        Found::Module { file } => {
            let compiled = compile_file(file, false, String::new())?;
            // A module inside a package gets the parent name as __package__.
            let package = name.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
            exec_compiled(interp, name, file, package, &compiled)
        }
        Found::Package { dir, init } => {
            let compiled = compile_file(init, true, dir.clone())?;
            exec_compiled(interp, name, init, name, &compiled)?;
            attach_module_spec(name);
            Ok(())
        }
        Found::Namespace { dir } => {
            // Namespace package: empty namespace + __path__, no body to run.
            let m = PyObject::new(ap_obj::ObjKind::Module, name);
            let ns = PyDict::new();
            let _ = ns.set(s("__name__"), s(name));
            let _ = ns.set(s("__package__"), s(name));
            let _ = ns.set(
                s("__path__"),
                Value::List(ap_obj::value::PyList::from_vec(vec![s(dir.clone())])),
            );
            let _ = ns.set(s("__doc__"), Value::None);
            let v = Value::Obj(m.clone());
            *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*ns).clone()));
            sys_modules_insert(name, &v);
            Ok(())
        }
    }
}

/// Create the module object, insert into sys.modules (PARTIAL visibility —
/// circular imports observe the half-built namespace), run the body.
fn exec_compiled(
    interp: &Arc<Interpreter>,
    name: &str,
    file: &str,
    package: &str,
    compiled: &Compiled,
) -> Result<(), Flow> {
    let loaded = load_into(interp, compiled, name, file, package)?;
    let m = PyObject::new(ap_obj::ObjKind::Module, name);
    let v = Value::Obj(m.clone());
    // SHARE the ns dict (not a snapshot): the module object must observe
    // body-execution writes (interp bug #10 — the pre-execution clone made
    // every module attr invisible after import).
    // 02.04a fix pass (BUG-C): both the pre-body and post-body assignments
    // must SHARE loaded.ns — the pre-body `.clone()` gave circular-import /
    // self-referential reads (warnings.py: `_wm = sys.modules[__name__]`,
    // then `_wm.filterwarnings`) an EMPTY snapshot; sys is a PyDict clone so
    // the clone never sees body writes.
    *m.module_ns.borrow_mut() = Some(loaded.ns.clone());
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg) {
        eprintln!(
            "[EXECDBG] {} ns_ptr={:p} frame_g={:p}",
            name,
            std::sync::Arc::as_ptr(&loaded.ns),
            std::sync::Arc::as_ptr(&loaded.frame.globals)
        );
    }
    // BEFORE body execution (design: CPython circular-import semantics).
    sys_modules_insert(name, &v);
    let stmts = module_stmts(interp, compiled, &loaded.reg, &loaded.frame);
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::StmtDbg)
        && name.contains("http")
    {
        for (i, st) in stmts.iter().enumerate() {
            eprintln!("[STMTDBG] {} #{} {:?}", name, i, st);
        }
    }
    let outcome = run_loaded(interp, &loaded, &stmts);
    // Refresh the module's namespace view after body execution (the module
    // stores a by-value PyDict; body writes landed in `loaded.ns`).
    *m.module_ns.borrow_mut() = Some(loaded.ns.clone());
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg)
        && name == "_py_warnings"
    {
        eprintln!(
            "[EXECDBG-post] {} ns items={} has_DEP={}",
            name,
            loaded.ns.items().len(),
            loaded
                .ns
                .get(&Value::Str(PyString::new("_DEPRECATED_MSG")))
                .is_some()
        );
    }
    match outcome {
        Ok(()) => Ok(()),
        Err(f) => {
            // Failed body: CPython REMOVES the module from sys.modules.
            // M2-adj (memperf audit 2026-11-06): the module frame dies with
            // the failed body — drop its MODULE_FRAME_CELLS entries (the
            // table is keyed by the monotone frame id, so this is exact).
            crate::interp::retire_module_frame_cells(loaded.frame.frame_id);
            {
                use crate::runtime_state as rs;
                rs::with_active_mut(rs::SYS_MODULES_T_acc(), |s| {
                    s.remove(name);
                });
                rs::with_active(rs::SYS_MODULES_VIEW_T_acc(), |v| {
                    if let Some(d) = v.as_ref() {
                        let _ = d.pop(&s(name.to_string()));
                    }
                });
            }
            Err(f)
        }
    }
}

fn module_stmts(
    interp: &Arc<Interpreter>,
    compiled: &Compiled,
    reg: &Arc<FnRegistry>,
    frame: &Arc<Frame>,
) -> Vec<ap_ir::Stmt> {
    let mut stmts: Vec<ap_ir::Stmt> = Vec::new();
    for item in &compiled.module.items {
        match item {
            ap_ir::Item::Stmt(st) => stmts.push(st.clone()),
            ap_ir::Item::FnDef(f) => {
                // T1-d fix (ast.py star-import + later def parse): ALL
                // module-level defs defer to source position (eager
                // binding ran before body exec, so a star import clobbered
                // same-named defs). Register the defining frame so
                // default-arg expressions resolve against THIS module's
                // globals (`def f(v=_sentinel)` — vendored traceback.py).
                let id = reg.by_path_lookup(&f.name).unwrap_or(0);
                interp
                    .defining_frames
                    .borrow_mut()
                    .insert(id, frame.clone());
                // T1-d: pre-create the fn's CELL storage on the module
                // frame — nested defs wire their free vars from
                // defining.my_cells at call time; module frames never run
                // call_function's cell setup, so without this the cells
                // are missing (typing._tp_cache 'typed' NameError).
                // B5: resolve against this module's own table (exact).
                if let Some(scope) = reg.scope_of_in(frame.scope_table.get(), &f.name) {
                    // T1-d: cells keyed by (module frame ID, name) —
                    // nested-def wiring consults this when the defining
                    // frame is a MODULE frame (which never builds my_cells
                    // via call_function). M2-adj (memperf audit): the
                    // monotone frame_id replaces the recycled Arc address.
                    // 02.09a P3: storage in the RuntimeState table.
                    let key = frame.frame_id;
                    frame.has_frame_cells.set(true);
                    {
                        use crate::runtime_state as rs;
                        rs::with_active_mut(rs::MODULE_FRAME_CELLS_T_acc(), |m| {
                            for cname in &scope.cells {
                                m.entry((key, cname.clone()))
                                    .or_insert_with(|| crate::cell_slot::CellSlotHandle::new(None));
                            }
                        });
                    }
                }
                stmts.push(ap_ir::Stmt::LocalDef(Box::new(f.clone())));
            }
            ap_ir::Item::ClassDef(c) => {
                stmts.push(ap_ir::Stmt::LocalClass(Box::new(c.clone())));
            }
        }
    }
    stmts
}

/// Attach `sys.path` / `sys.modules` onto the sys module namespace.
pub fn attach_sys_extras(_interp: &Arc<Interpreter>, sys_m: &Value) {
    if let Value::Obj(o) = sys_m {
        let mut ns_ref = o.module_ns.borrow_mut();
        if let Some(ns) = &mut *ns_ref {
            let _ = ns.set(s("path"), sys_path_value());
            let view = match ns.get(&s("modules")) {
                Some(Value::Dict(d)) => d,
                _ => {
                    let d = PyDict::new();
                    let _ = ns.set(s("modules"), Value::Dict(d.clone()));
                    d
                }
            };
            // Pre-seed with the current registry, then keep the view live.
            {
                use crate::runtime_state as rs;
                rs::with_active(rs::SYS_MODULES_T_acc(), |m| {
                    for (k, v) in m.iter() {
                        let _ = view.set(s(k.clone()), v.clone());
                    }
                });
                rs::with_active_mut(rs::SYS_MODULES_VIEW_T_acc(), |slot| {
                    *slot = Some(view);
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Relative-import support (from . import x / from .mod import y)
// ---------------------------------------------------------------------------

/// Resolve a relative ImportFrom's target package. `level` = number of dots
/// (1 = current package). Mirrors CPython's _calc___package__:
/// - level 1, no module: `__package__`
/// - level >1 or with module: strip (level-1) trailing components from the
///   package, then join `module`.
///   Returns Err for "attempted relative import with no known parent package"
///   (byte-exact) when no parent package is known.
pub fn resolve_relative_base(
    interp: &Arc<Interpreter>,
    frame: &Arc<Frame>,
    level: usize,
    module: &str,
) -> Result<String, Flow> {
    ap_trace(format!(
        "resolve_relative_base(level={level}, module={module})"
    ));
    builtins_modules::set_current_interp(interp);
    // The __package__ of the frame's module: read the module global the
    // frame chain lives in (module frames keep globals = module ns).
    let pkg = frame
        .globals
        .get(&s("__package__"))
        .or_else(|| frame.globals.get(&s("__name__")));
    let pkg = match pkg {
        Some(Value::Str(sp)) => sp.as_str().to_string(),
        _ => String::new(),
    };
    if pkg.is_empty() {
        return Err(raise(
            "ImportError",
            "attempted relative import with no known parent package",
        ));
    }
    // CPython: for `from .. import x` in module a.b.c, base = a.b
    // (strip level-1 trailing parts from the module name).
    let mut parts: Vec<&str> = pkg.split('.').collect();
    let strip = level.saturating_sub(1);
    if strip > 0 {
        if strip >= parts.len() {
            return Err(raise(
                "ImportError",
                "attempted relative import beyond top-level package",
            ));
        }
        parts.truncate(parts.len() - strip);
    }
    let mut base = parts.join(".");
    if !module.is_empty() {
        base = if base.is_empty() {
            module.to_string()
        } else {
            format!("{base}.{module}")
        };
    }
    Ok(base)
}

/// `from <module> import names` where <module> resolved to a file/native
/// module Value: bind each name (with submodule fallback for packages).
pub fn import_from_module(
    interp: &Arc<Interpreter>,
    frame: &Arc<Frame>,
    module: &str,
    mv: &Value,
    names: &[(String, String)], // (source, bind)
) -> Result<(), Flow> {
    for (src, bind) in names {
        let got = attr_on_module(interp, mv, src);
        let got = match got {
            Some(v) => Some(v),
            // 04.01: module objects backed by a real class (six.moves is a
            // _LazyModule instance) keep their moved-attribute DESCRIPTORS
            // on the class — fall through the full attr protocol (which
            // invokes __get__) before the submodule lookup.
            None => {
                let rc = crate::builtins_modules::current_interp();
                if let Some(i) = rc {
                    match crate::interp::attr_get_public(&i, mv, src) {
                        Ok(Some(v)) => Some(v),
                        _ => None,
                    }
                } else {
                    None
                }
            }
        };
        let got = match got {
            Some(v) => Some(v),
            None => {
                // Package submodule fallback: `from pkg import sub` where
                // sub is not an attr of __init__ but a module on pkg.__path__.
                let dir = module_path_dir(mv);
                if let Some(dir) = dir {
                    if let Some(found) = find_submodule(&dir, src) {
                        let full = format!("{module}.{src}");
                        // T1-d fix (asyncio tasks↔timeouts cycle): if the
                        // submodule is ALREADY in sys.modules (partial from a
                        // circular import), do NOT re-execute its file —
                        // CPython returns the half-built module. Re-running
                        // the body here restarts the cycle forever.
                        // Skip re-exec only for an ALREADY-REGISTERED module
                        // (a re-entrant/circular import); non-module `Found`
                        // kinds (e.g. namespace packages) always re-exec.
                        let already_registered = sys_modules_get(&full).is_some();
                        if !(already_registered && matches!(found, Found::Module { .. })) {
                            exec_found(interp, &full, &found)?;
                        }
                        // Re-read the (possibly refreshed) module attr.
                        if let Some(fresh) = sys_modules_get(&full) {
                            // Bind the submodule itself.
                            frame.store_global(bind, fresh);
                            continue;
                        }
                    }
                }
                None
            }
        };
        match got {
            Some(v) => frame.store_global(bind, v),
            None => {
                if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg) {
                    if let Value::Obj(o) = mv {
                        let n = o
                            .module_ns
                            .borrow()
                            .as_ref()
                            .map(|ns| ns.items().len())
                            .unwrap_or(usize::MAX);
                        let keys = o
                            .module_ns
                            .borrow()
                            .as_ref()
                            .map(|ns| {
                                ns.items()
                                    .iter()
                                    .filter_map(|(k, _)| match k {
                                        Value::Str(sv) => Some(sv.as_str().to_string()),
                                        _ => None,
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        eprintln!("[FROMDBG] module_ptr={:p}", std::sync::Arc::as_ptr(o));
                        eprintln!(
                            "[FROMDBG] miss '{src}' in {module}: ns has {} items: {:?}",
                            n, keys
                        );
                    }
                }
                let file = module_file_repr(mv).unwrap_or_else(|| module.to_string());
                return Err(raise(
                    "ImportError",
                    format!("cannot import name '{src}' from '{module}' ({file})"),
                ));
            }
        }
    }
    Ok(())
}

/// Attribute read on a module value (module_ns dict), native modules included.
fn attr_on_module(_interp: &Arc<Interpreter>, mv: &Value, name: &str) -> Option<Value> {
    if let Value::Obj(o) = mv {
        let ns = o.module_ns.borrow();
        if let Some(ns) = &*ns {
            return ns.get(&s(name));
        }
    }
    None
}

/// `__file__` of a module value (CPython error messages include the file).
fn module_file_repr(mv: &Value) -> Option<String> {
    if let Value::Obj(o) = mv {
        let ns = o.module_ns.borrow();
        if let Some(ns) = &*ns {
            if let Some(Value::Str(f)) = ns.get(&s("__file__")) {
                return Some(f.as_str().to_string());
            }
            return None;
        }
    }
    None
}

/// `from <abs-or-rel-module> import names` resolving through the machinery.
/// `base` is the already-qualified absolute module name (relative part
/// resolved by the caller via resolve_relative_base).
pub fn import_from(
    interp: &Arc<Interpreter>,
    frame: &Arc<Frame>,
    base: &str,
    names: &[(String, String)],
) -> Result<(), Flow> {
    let mv = import_module(interp, base)?;
    import_from_module(interp, frame, base, &mv, names)
}

// Silence the unused-import lint for FnEntry (kept for parity with the
// interp-side def binding if this module grows it).
#[allow(unused)]
fn _fn_entry_parity(_e: FnEntry) {}

/// `from M import *` (02.04a): bind every public name of the module's
/// namespace into the frame's globals. v0: no `__all__` support, no
/// module-level `_` filtering for native marker names.
pub fn import_star(_interp: &Arc<Interpreter>, frame: &Arc<Frame>, mv: &Value) -> Result<(), Flow> {
    let names: Vec<String> = match mv {
        Value::Obj(o) => {
            let ns = o.module_ns.borrow();
            match &*ns {
                Some(ns) => ns
                    .keys()
                    .into_iter()
                    .filter_map(|k| match k {
                        Value::Str(sp) => Some(sp.as_str().to_string()),
                        _ => None,
                    })
                    .collect::<Vec<String>>(),
                None => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    for name in names {
        if name.starts_with('_') {
            continue;
        }
        if let Some(v) = attr_on_module(_interp, mv, &name) {
            frame.store_global(&name, v);
        }
    }
    Ok(())
}

/// The `builtins` module: namespace snapshots interp.builtins (reprlib etc
/// do `import builtins` then read names lazily — a COPY is fine for v0
/// because interpreter builtins don't change after install).
fn builtins_module(interp: &Arc<Interpreter>) -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "builtins");
    let ns = PyDict::new();
    for (k, v) in interp.builtins.items() {
        let _ = ns.set(k, v);
    }
    let _ = ns.set(s("__name__"), s("builtins"));
    let _ = ns.set(s("__doc__"), Value::None);
    *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*ns).clone()));
    Value::Obj(m)
}

/// `_thread` stub: reprlib (and traceback filtering) call `_thread.get_ident()`.
/// Single-threaded v0: a monotonic counter keyed per call site is enough.
fn thread_stub_module(interp: &crate::frame::Interpreter) -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "_thread");
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s("_thread"));
    let _ = ns.set(s("__doc__"), Value::None);
    // get_ident: T2 fix - bind a REAL callable. It was a plain int (v0
    // "identity comparisons only"), but reprlib.recursive_repr CALLS
    // get_ident(); dataclasses wraps every generated __repr__ in
    // @recursive_repr, so repr() of any dataclass instance raised
    // TypeError: int object is not callable (pytest Skip dataclass,
    // outcome reports). threading_call.rs:191 already dispatches the
    // "get_ident" marker name to get_ident_value(); a BuiltinFn marker
    // routes there.
    let _ = ns.set(
        s("get_ident"),
        crate::builtins_modules::marker("threading.get_ident"),
    );
    let _ = ns.set(s("allocate_lock"), Value::None);
    // 02.04a fix pass (BUG-C): RLock — _py_warnings does `with _lock:`.
    // v0: a no-op context manager (single-threaded runtime; real locking is
    // a Plan-04 surface).
    let lock_type = ap_obj::classes::TypeObject::create(
        "RLock".to_string(),
        "RLock".to_string(),
        vec![crate::classes::object_class(interp)],
        PyDict::new(),
        ap_obj::classes::TypeFlags::default(),
        None,
    )
    .expect("RLock mro");
    let enter_fn = PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_rlock_enter__");
    let exit_fn = PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_rlock_exit__");
    let _ = lock_type.dict.set(s("__enter__"), Value::Obj(enter_fn));
    let _ = lock_type.dict.set(s("__exit__"), Value::Obj(exit_fn));
    let lock_class_value = ap_obj::classes::make_class_value(lock_type);
    let _ = ns.set(s("RLock"), lock_class_value.clone());
    let _ = ns.set(s("allocate_lock"), lock_class_value);
    *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*ns).clone()));
    Value::Obj(m)
}

/// `_weakref` stub: weakref.py hard-imports ref/proxy/type names. v0: markers
/// only (no GC-hook semantics; documented gap — corpus cases that call ref()
/// will surface a fail-loud marker error rather than import failure).
fn weakref_stub_module() -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "_weakref");
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s("_weakref"));
    let _ = ns.set(s("__doc__"), Value::None);
    for n in [
        "getweakrefcount",
        "getweakrefs",
        "proxy",
        "_remove_dead_weakref",
    ] {
        let _ = ns.set(s(n), marker_fn(&format!("_weakref.{n}")));
    }
    // 02.03b-fix: ref must be a TYPE (importlib._bootstrap does
    // `class KeyedRef(_weakref.ref)` — a function base broke class
    // creation). Calls route through call_type_ctor's "ref" arm, which
    // returns the opaque token (v0: nothing dereferences it).
    let _ = ns.set(
        s("ref"),
        Value::Obj(PyObject::new(ap_obj::ObjKind::Type, "ref")),
    );
    for t in ["CallableProxyType", "ProxyType", "ReferenceType"] {
        let _ = ns.set(s(t), s(t));
    }
    *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*ns).clone()));
    Value::Obj(m)
}

/// `_typing` stub: typing.py hard-imports the Caccelerators at module scope.
/// v0: callable markers + interned types; the runtime gates never exercise
/// real TypeVar substitution (documented gap).
fn typing_stub_module(interp: &crate::frame::Interpreter) -> Value {
    let m = PyObject::new(ap_obj::ObjKind::Module, "_typing");
    let ns = PyDict::new();
    let _ = ns.set(s("__name__"), s("_typing"));
    let _ = ns.set(s("__doc__"), Value::None);
    for n in ["TypeVar", "ParamSpec", "TypeVarTuple", "Union"] {
        let mv = marker_fn(&format!("_typing.{n}"));
        // T2 gate finding 3: these markers ARE classes in CPython — repr
        // is <class 'typing.X'> (union __origin__ renders byte-exact).
        if let Value::Obj(mo) = &mv {
            *mo.repr_text.borrow_mut() = Some(format!("<class 'typing.{n}'>"));
        }
        let _ = ns.set(s(n), mv);
    }
    // T1-d: _idfunc is the identity function — typing.NewType.__call__
    // delegates to it, so NewType handles (twisted _SystemEventID tuples)
    // return their argument and stay subscriptable.
    {
        let f = PyObject::new(ap_obj::ObjKind::BuiltinFn, "_typing._idfunc");
        let _ = ns.set(s("_idfunc"), Value::Obj(f));
    }
    for t in [
        "ParamSpecArgs",
        "ParamSpecKwargs",
        "TypeAliasType",
        "NoDefault",
    ] {
        let _ = ns.set(s(t), crate::builtins::interned_type(interp, t));
    }
    // Generic must be a class-derivable base: a real (object-derived)
    // TypeObject so `class Protocol(Generic)` and subclasses synthesize MROs.
    let generic = ap_obj::classes::TypeObject::create(
        "Generic".to_string(),
        "Generic".to_string(),
        vec![crate::classes::object_class(interp)],
        PyDict::new(),
        ap_obj::classes::TypeFlags::default(),
        None,
    )
    .expect("Generic mro");
    let _ = ns.set(s("Generic"), ap_obj::classes::make_class_value(generic));
    *m.module_ns.borrow_mut() = Some(std::sync::Arc::new((*ns).clone()));
    Value::Obj(m)
}

fn marker_fn(name: &str) -> Value {
    Value::Obj(PyObject::new(ap_obj::ObjKind::BuiltinFn, name))
}

/// Ensure `parent.<child>` is an attribute of the parent package (CPython
/// sets it on submodule import).
fn link_parent_attr(full: &str, m: &Value) {
    let Some((parent_name, child_name)) = full.rsplit_once('.') else {
        return;
    };
    if let Some(Value::Obj(pp)) = sys_modules_get(parent_name) {
        if let Some(pns) = &*pp.module_ns.borrow_mut() {
            let _ = pns.set(s(child_name), m.clone());
        }
    }
}

/// Copy names the native module lacks from the vendored CPython file's
/// executed namespace into it (02.04a augmentation — keeps native identity,
/// adds private helpers like posixpath._splitext).
pub(crate) fn augment_with_vendored(interp: &Arc<Interpreter>, name: &str, m: &Value) {
    // Already augmented?
    if let Value::Obj(o) = m {
        if let Some(ns) = &*o.module_ns.borrow() {
            if ns.get(&s("_ap_vendored_augmented")).is_some() {
                return;
            }
        }
    }
    // Locate the vendored file.
    let entries = sys_path_entries();
    for base in entries {
        let file = std::path::Path::new(&base).join(format!("{name}.py"));
        if !file.is_file() {
            continue;
        }
        let path = file.to_string_lossy().into_owned();
        let Ok(compiled) = compile_file(&path, false, String::new()) else {
            return;
        };
        let loaded = match load_into(interp, &compiled, name, &path, name) {
            Ok(l) => l,
            Err(_) => return,
        };
        let stmts = module_stmts(interp, &compiled, &loaded.reg, &loaded.frame);
        // Best-effort body run (vendored namesakes may pull imports we
        // don't serve — ignore failures and copy what landed).
        let outcome = run_loaded(interp, &loaded, &stmts);
        if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::ModDbg) {
            let n_items = loaded.ns.items().len();
            eprintln!(
                "[AUGDBG] augment {name}: run_loaded ok={} ns_items={}",
                outcome.is_ok(),
                n_items
            );
        }
        // Copy names the native namespace lacks. T1 (typing): the vendored
        // defaultdict is a REAL dict subclass (dict-backing + __missing__);
        // the native interned Type stub constructs plain dicts and breaks
        // typing._overload_registry. Vendored definitions WIN for names on
        // this override list (native identity for everything else).
        const VENDORED_WINS: &[&str] = &[
            "defaultdict",
            // T1-d (scrapy.utils.datatypes CaseInsensitiveDict): the native
            // UserDict interned Type has no data-dict behavior; the vendored
            // Python class (dict subclass) makes super().__setitem__ work.
            "UserDict",
            "UserList",
            "UserString",
            // W110 (T2 pytest): inspect.py calls
            // new_params.move_to_end(...) on OrderedDict; the native
            // interned Type stub lacks it. The vendored class is a real
            // dict subclass with the full surface.
            "OrderedDict",
        ];
        // W110 (T2): functools augmentation must be wholesale — the native
        // WRAPPER_ASSIGNMENTS stub lacks '__wrapped__' so a name-gated copy
        // would keep the broken stub. Vendored names overwrite all.
        let wholesale = name == "functools";
        if let Value::Obj(mo) = m {
            if let Some(cns) = &*mo.module_ns.borrow_mut() {
                for (k, v) in loaded.ns.items() {
                    let wins = wholesale
                        || matches!(&k, Value::Str(kv) if VENDORED_WINS.contains(&kv.as_str()));
                    if wins || cns.get(&k).is_none() {
                        let _ = cns.set(k, v);
                    }
                }
                let _ = cns.set(s("_ap_vendored_augmented"), Value::Bool(true));
            }
        }
        return;
    }
}

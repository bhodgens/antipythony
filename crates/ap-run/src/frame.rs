//! Frames + name resolution (task brief §1): a global dict, per-frame
//! locals, cells/frees through shared `RefCell` slots (02.04 revisits the
//! representation), builtin fallback chain, and the interpreter state
//! (function table, iterator table, generator registry, output buffer).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::exc_hier::ExceptionValue;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::Value;

/// One shared cell: closure variable storage (S-02-12/13).
/// A cell handle: Arc<RefCell<Option<Value>>> with a staged Send+Sync
/// marker (02.09a P4/P5 — see ap_obj::markers + int.rs contract: the cell
/// is touched under the GIL/table-lock protocol; refcount traffic is Arc).
pub type CellSlot = crate::cell_slot::CellSlotHandle;

/// A6: an Arc-shared declared-names set. Resolved ONCE when a FnEntry is
/// registered (from the scope classification) and shared by every frame
/// built from it — building a function frame is an Arc clone, not a
/// HashSet<String> rebuild, per call.
pub type DeclaredNames = std::sync::Arc<std::collections::HashSet<String>>;

pub fn empty_declared() -> DeclaredNames {
    std::sync::Arc::new(std::collections::HashSet::new())
}

thread_local! {
    /// E8/M2-adj: monotone counter minting Frame::frame_id (unique per
    /// thread; frame ids are never reused, so keyed side tables can't
    /// alias a recycled Arc address to a fresh frame's entries).
    static NEXT_FRAME_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

pub fn next_frame_id() -> u64 {
    NEXT_FRAME_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    })
}

/// A single execution frame.
pub struct Frame {
    /// Locals (module frames keep this empty and use `globals` directly).
    pub locals: RefCell<HashMap<String, Loc>>,
    /// A6 (memperf audit 2026-11-06): the ENCLOSING frame for
    /// comprehension/genexpr child scopes. The old run_comp snapshotted
    /// ALL of the parent's bound locals (one Value clone per name) into
    /// the comp frame on every comprehension execution. With a parent
    /// link, `lookup` falls through the parent's live locals/cells instead
    /// — zero per-run allocations, and reads see the CURRENT values
    /// (closer to CPython 3 cell-based comprehension semantics).
    pub parent: Option<Arc<Frame>>,
    /// E8/M2-adj (memperf audit): monotone frame identity, unique across a
    /// thread's lifetime (Arc addresses get recycled once a module frame
    /// drops — the old (addr, name) keys on MODULE_FRAME_CELLS could then
    /// alias a fresh frame's cells).
    pub frame_id: u64,
    /// M2-adj: set when the importer seeded MODULE_FRAME_CELLS entries for
    /// this module frame; gates the Drop-time retire so transient module
    /// frames (eval_default's fallback frame per default-expr eval) don't
    /// scan the table on every drop.
    pub has_frame_cells: std::cell::Cell<bool>,
    /// The module-level dict this frame reads globals from.
    pub globals: Gc<PyDict>,
    /// Scope path for classification lookup ("" for module).
    pub scope_path: String,
    /// Cells THIS function owns (its `cells` classification).
    pub my_cells: HashMap<String, CellSlot>,
    /// Cells captured from enclosing scopes (its `frees`).
    pub free_cells: HashMap<String, CellSlot>,
    /// B5 (memperf audit): the DEFINING module's registered scope table,
    /// plumbed from FnEntry.scope_table. Nested-def/lambda scope-path
    /// classification prefers THIS table (exact, no cross-module
    /// richest-wins collision), falling back to the global scan. Cell
    /// because the call site sets it after the Arc<Frame> exists.
    pub scope_table: std::cell::Cell<Option<&'static ap_parse::ScopeInfoModule>>,
    /// Names declared `global` in this scope (S-02-3): stores go to globals.
    /// A6 (memperf audit 2026-11-06): `DeclaredNames` is an Arc-shared set —
    /// the registry caches one per scope path so per-call frames no longer
    /// rebuild two fresh HashSet<String> (2× |names| String allocs per call).
    pub declared_globals: DeclaredNames,
    /// Names declared `nonlocal` (S-02-4).
    pub declared_nonlocals: DeclaredNames,
    /// Qualname for traceback rendering.
    pub qualname: String,
    /// Wave-58: the last executed statement's source line IN THIS
    /// frame. The old frame_stack-of-cells approach let a parent
    /// frame's touch_line overwrite a child's cell mid-unwind (shared
    /// fs.last() writes), so traceback entries recorded the caller's
    /// line. Per-frame cell is written only by this frame's exec_stmt.
    pub last_line: Cell<usize>,
    /// G4-repair: the source line of the def/lambda statement currently
    /// being executed (written by the Stmt::Line marker preceding it in
    /// the lowered block). Joins the nested-def dedupe key as the
    /// "which def in this body" component — stable across calls of the
    /// SAME enclosing function (fresh frames carry the same lines).
    pub def_line: Cell<usize>,
    pub is_module: bool,
    /// Class-body frames: the dotted qualname prefix nested classes extend
    /// (S-03-04 `Outer.Inner`); empty elsewhere.
    pub class_qual_prefix: String,
    /// Class-body frames: the `__class__` cell (zero-arg super, S-03-17).
    /// Seeded AFTER the class object exists; methods defined in the body
    /// capture it through their closure `frees` (name `__class__`).
    pub class_cell: std::cell::RefCell<Option<CellSlot>>,
    /// Class-body frames: the class namespace (Some only on class frames).
    pub class_ns: Option<Gc<PyDict>>,
    /// 02.03b: when `__prepare__` returned a dict-subclass INSTANCE
    /// (enum.EnumDict), class-body stores route through its `__setitem__`
    /// (the instance carries the member-tracking side effects) while reads
    /// still hit the shared backing `class_ns` dict.
    pub class_ns_inst: Option<Value>,
}

/// Where a local name lives.
#[derive(Clone)]
pub enum Loc {
    /// Direct value slot (None = unbound → UnboundLocalError on read).
    Slot(Arc<RefCell<Option<Value>>>),
    /// A shared cell (closure traffic).
    CellSlot(CellSlot),
}

impl Frame {
    pub fn new_module(globals: Gc<PyDict>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            parent: None,
            frame_id: next_frame_id(),
            globals,
            scope_path: String::new(),
            my_cells: HashMap::new(),
            free_cells: HashMap::new(),
            scope_table: std::cell::Cell::new(None),
            has_frame_cells: std::cell::Cell::new(false),
            declared_globals: empty_declared(),
            declared_nonlocals: empty_declared(),
            qualname: "<module>".to_string(),
            is_module: true,
            last_line: std::cell::Cell::new(0),
            def_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: None,
            class_ns_inst: None,
        }
    }

    /// A function frame with pre-bound locals and closure cell sets.
    #[allow(clippy::too_many_arguments)] // mirrors the CPython C-function signature shape
    pub fn new_function(
        globals: Gc<PyDict>,
        scope_path: &str,
        qualname: &str,
        declared_globals: DeclaredNames,
        declared_nonlocals: DeclaredNames,
        my_cells: HashMap<String, CellSlot>,
        free_cells: HashMap<String, CellSlot>,
        locals: HashMap<String, Loc>,
    ) -> Frame {
        Frame {
            locals: RefCell::new(locals),
            parent: None,
            frame_id: next_frame_id(),
            globals,
            scope_path: scope_path.to_string(),
            my_cells,
            free_cells,
            scope_table: std::cell::Cell::new(None),
            has_frame_cells: std::cell::Cell::new(false),
            declared_globals,
            declared_nonlocals,
            qualname: qualname.to_string(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
            def_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: None,
            class_ns_inst: None,
        }
    }

    /// A comprehension/child scope that falls back to `parent` (v0: the
    /// child shares the parent's globals; comp locals are isolated here).
    pub fn new_child(parent: &Arc<Frame>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            parent: Some(parent.clone()),
            frame_id: next_frame_id(),
            globals: parent.globals.clone(),
            scope_path: parent.scope_path.clone(),
            my_cells: HashMap::new(),
            free_cells: parent.my_cells.clone(),
            scope_table: std::cell::Cell::new(parent.scope_table.get()),
            has_frame_cells: std::cell::Cell::new(false),
            declared_globals: empty_declared(),
            declared_nonlocals: empty_declared(),
            qualname: parent.qualname.clone(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
            def_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: None,
            class_ns_inst: None,
        }
    }

    /// A class-body frame: stores land in the class namespace `ns`
    /// (S-03-3); the namespace is NOT consulted for name LOOKUPS (class
    /// scope reads go straight to globals — S-03-3's "not an enclosing
    /// scope" rule, and CPython class-body semantics).
    pub fn new_class(parent: &Arc<Frame>, ns: Gc<PyDict>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            parent: None,
            frame_id: next_frame_id(),
            globals: parent.globals.clone(),
            scope_path: parent.scope_path.clone(),
            my_cells: HashMap::new(),
            free_cells: HashMap::new(),
            scope_table: std::cell::Cell::new(parent.scope_table.get()),
            has_frame_cells: std::cell::Cell::new(false),
            declared_globals: empty_declared(),
            declared_nonlocals: empty_declared(),
            qualname: parent.qualname.clone(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
            def_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: Some(ns),
            class_ns_inst: None,
        }
    }

    pub fn store_local(&self, name: &str, v: Value) {
        let mut locals = self.locals.borrow_mut();
        match locals.get(name) {
            Some(Loc::Slot(slot)) => *slot.borrow_mut() = Some(v),
            Some(Loc::CellSlot(cell)) => *cell.borrow_mut() = Some(v),
            None => {
                locals.insert(name.to_string(), Loc::Slot(Arc::new(RefCell::new(Some(v)))));
            }
        }
    }

    /// Does this frame hold ANY binding for `name` (local slot, own cell,
    /// or free cell)? jit_hook's slot sync-back uses this to distinguish
    /// "the JIT'd body assigned None" from "the slot was never assigned"
    /// (the cfg ABI models unbound as None; store_name(None) on a fresh
    /// name would fabricate a binding the tree-walker never makes).
    pub fn has_binding(&self, name: &str) -> bool {
        if self.locals.borrow().contains_key(name)
            || self.my_cells.contains_key(name)
            || self.free_cells.contains_key(name)
        {
            return true;
        }
        // Declared globals write through to the module dict, which the
        // caller treats as always-bindable; class-ns frames never reach
        // the JIT hook (the dispatch arm is call_function on FnDef bodies).
        self.declared_globals.contains(name) || self.declared_nonlocals.contains(name)
    }

    pub fn store_global(&self, name: &str, v: Value) {
        let _ = self.globals.set(Value::Str(ap_obj::PyString::new(name)), v);
    }

    pub fn lookup(&self, name: &str) -> Lookup {
        {
            let locals = self.locals.borrow();
            if let Some(loc) = locals.get(name) {
                return match loc {
                    Loc::Slot(cell) => match &*cell.borrow() {
                        Some(v) => Lookup::Found(v.clone()),
                        None => Lookup::UnboundLocal,
                    },
                    Loc::CellSlot(cell) => match &*cell.borrow() {
                        Some(v) => Lookup::Found(v.clone()),
                        None => Lookup::UnboundLocal,
                    },
                };
            }
        }
        if let Some(cell) = self.free_cells.get(name) {
            return match &*cell.borrow() {
                Some(v) => Lookup::Found(v.clone()),
                None => Lookup::UnboundLocal,
            };
        }
        // A6 (memperf audit): comprehension/child frames read the
        // ENCLOSING frame's locals LIVE through the parent link instead of
        // a per-run snapshot copy (one Value clone + one HashMap insert per
        // bound name, per comprehension execution). Precedence replicates
        // the old snapshot EXACTLY: the old child frame's `locals` map
        // merged its own bindings over a copy of the parent's BOUND locals,
        // and a nested comp's parent snapshot transitively carried the
        // grandparent's bound values (they had been seeded into the parent
        // comp frame's own map). So: walk the parent chain consulting ONLY
        // each frame's bound locals (skip unbound slots — the old snapshot
        // never seeded them), self.free_cells first (new_child's
        // `parent.my_cells.clone()` — the parent's OWN cells were visible
        // to the old child through that copy), and ancestor free/my cells
        // NOT consulted beyond that (a 2-level-old child could not see the
        // grandparent's cell-only names; preserved).
        {
            let mut cur = self.parent.as_ref();
            while let Some(p) = cur {
                let locals = p.locals.borrow();
                if let Some(loc) = locals.get(name) {
                    let found = match loc {
                        Loc::Slot(cell) => cell.borrow().clone(),
                        Loc::CellSlot(cell) => cell.borrow().clone(),
                    };
                    drop(locals);
                    if let Some(v) = found {
                        return Lookup::Found(v);
                    }
                } else {
                    drop(locals);
                }
                cur = p.parent.as_ref();
            }
        }
        // T1-d fix (@x.setter on __name__-style dunders): the CLASS namespace
        // must be consulted BEFORE module globals — the importer seeds
        // globals["__name__"] = "__main__", which otherwise shadowed a
        // @property __name__ defined in the class body (typing_extensions
        // sentinel class; any @property-over-__name__ + @__name__.setter
        // resolved to the "__main__" STR and `.setter` blew up).
        // CPython: class-body name resolution sees the class namespace
        // before module globals (the body executes with the ns as locals).
        if let Some(ns) = &self.class_ns {
            if let Some(v) = ns.get(&Value::Str(ap_obj::PyString::new(name))) {
                return Lookup::Found(v);
            }
        }
        // 02.07: globals may be momentarily borrowed by the sibling thread
        // (store in flight when the GIL rotated). Retry — writes are
        // statement-atomic.
        let v = loop {
            match self
                .globals
                .try_get(&Value::Str(ap_obj::PyString::new(name)))
            {
                Some(v) => break v,
                None => std::thread::sleep(std::time::Duration::from_micros(50)),
            }
        };
        if let Some(v) = v {
            return Lookup::Found(v);
        }
        Lookup::Miss
    }

    /// `del x` — unbind. Returns false when the name was never bound.
    pub fn delete_name(&self, name: &str) -> bool {
        {
            let mut locals = self.locals.borrow_mut();
            if let Some(loc) = locals.remove(name) {
                match loc {
                    Loc::Slot(_) => return true,
                    Loc::CellSlot(cell) => {
                        *cell.borrow_mut() = None;
                        // The cell survives del (other closures may read it);
                        // rebind as cleared.
                        locals.insert(name.to_string(), Loc::CellSlot(cell));
                        return true;
                    }
                }
            }
        }
        if let Some(cns) = &self.class_ns {
            if cns.pop(&Value::Str(ap_obj::PyString::new(name))).is_some() {
                return true;
            }
        }
        self.globals
            .pop(&Value::Str(ap_obj::PyString::new(name)))
            .is_some()
    }
}

impl Drop for Frame {
    /// M2-adj (memperf audit 2026-11-06): MODULE_FRAME_CELLS entries keyed
    /// by this frame's id become unreachable the moment the last Arc<Frame>
    /// drops (every consult path goes through a LIVE defining frame), so
    /// retire them here. Failed-import frames are additionally retired
    /// eagerly at the importer's failed-body arm because `defining_frames`
    /// still pins them there (the frame outlives the failed module).
    /// Cross-thread final drops hit another thread's TLS table and no-op
    /// (bounded leak of that frame's cells on that thread, never a wrong
    /// hit — ids are per-thread monotone).
    fn drop(&mut self) {
        if self.is_module && self.has_frame_cells.get() {
            crate::interp::retire_module_frame_cells(self.frame_id);
        }
    }
}

pub enum Lookup {
    Found(Value),
    /// Local slot exists but is unbound (UnboundLocalError, S-02-2).
    UnboundLocal,
    /// Not local/cell/global — caller checks builtins then raises NameError.
    Miss,
}

/// A registered function (function-table entry).
/// A2 (memperf audit 2026-11-06): `def` is an `Arc<FnDef>` — the registry
/// holds one copy of each function's AST and every clone of an entry (per
/// call, per generator start) bumps a refcount instead of deep-cloning the
/// whole body tree twice. B5/A6: the entry carries its PRE-RESOLVED scope
/// classification (from its defining module's own table) plus Arc-shared
/// declared-global/nonlocal sets, so call setup does zero scope lookups
/// and zero HashSet rebuilds.
#[derive(Clone)]
pub struct FnEntry {
    pub def: Arc<ap_ir::item::FnDef>,
    /// Dotted scope path matching ScopeInfoModule.
    pub scope_path: String,
    /// Qualname for tracebacks.
    pub qualname: String,
    /// 02.04a: the DEFINING module's globals (functions from imported
    /// modules must resolve names in THEIR module, not the runner's).
    /// Construction sites default this to interp.globals.
    pub globals: Gc<PyDict>,
    /// B5: scope classification resolved at registration against the
    /// DEFINING module's table (exact; no cross-module path collision).
    pub scope: Option<&'static ap_parse::ScopeInfo>,
    /// B5: the DEFINING module's leaked scope table (nested-def path
    /// lookups resolve against THIS table, not the global richest-wins
    /// merge — the twisted deprecate.py collision class).
    pub scope_table: Option<&'static ap_parse::ScopeInfoModule>,
    /// A6: Arc-shared declared-names sets (see FnRegistry::declared_sets).
    pub declared_globals: DeclaredNames,
    pub declared_nonlocals: DeclaredNames,
}

impl FnEntry {
    /// Construct with scope pre-resolution against `table` (the defining
    /// module's registered scope table) through `reg`'s path index.
    pub(crate) fn with_scope(
        def: ap_ir::item::FnDef,
        scope_path: String,
        qualname: String,
        globals: Gc<PyDict>,
        scope: Option<&'static ap_parse::ScopeInfo>,
        scope_table: Option<&'static ap_parse::ScopeInfoModule>,
    ) -> FnEntry {
        let (dg, dn) = match scope {
            Some(s) => (
                std::sync::Arc::new(s.globals.iter().cloned().collect()),
                std::sync::Arc::new(s.nonlocals.iter().cloned().collect()),
            ),
            None => (empty_declared(), empty_declared()),
        };
        FnEntry {
            def: Arc::new(def),
            scope_path,
            qualname,
            globals,
            scope,
            scope_table,
            declared_globals: dg,
            declared_nonlocals: dn,
        }
    }
}

/// The interpreter: owns module state, function table, iterator table,
/// generator registry, output buffer, traceback accumulation.
pub struct Interpreter {
    pub globals: Gc<PyDict>,
    pub builtins: Gc<PyDict>,
    /// Module-level function table (index == fn_id).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub functions: Mutex<Vec<FnEntry>>,
    /// Nested/lambda functions (ids offset by NESTED_BASE).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub nested_functions: Mutex<Vec<FnEntry>>,
    /// 02.09b P7 (decision 2): Mutex — every worker's for-loops register
    /// iterators here (the GIL-era try_borrow/sleep retry loops dissolve).
    pub iterators: Mutex<HashMap<u64, Gc<crate::iter::PyIterator>>>,
    pub generators: Mutex<HashMap<u64, crate::interp::GenFrame>>,
    /// Iterator ids are EVEN, generator ids ODD (both share the iterators
    /// table; disjoint namespaces prevent key collisions).
    /// 02.09b P7: atomics — two workers minting ids concurrently must never
    /// collide (the id spaces are process-unique; Cell read-modify-write
    /// raced under the flip).
    pub next_iter_id: std::sync::atomic::AtomicU64,
    pub next_gen_id: std::sync::atomic::AtomicU64,
    /// stdout accumulation (S-06-2 byte gate).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub stdout: Mutex<Vec<u8>>,
    /// G6: bytes already drained to fd 1 by the mid-run threshold flush —
    /// re-prepended into RunOutcome.stdout at the end-of-run flush so the
    /// outcome is byte-identical to the unbuffered behavior. G6-repair:
    /// appended ONLY when `stdout_capture` is set (opt-in) — the drained
    /// bytes are already on fd 1, and no production caller reads
    /// RunOutcome.stdout, so the unconditional copy kept ~2x the full
    /// output resident.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub stdout_flushed_prefix: Mutex<Vec<u8>>,
    /// G6-repair: opt-in RunOutcome.stdout capture (prefix + tail). Set
    /// only by embedders that actually consume RunOutcome.stdout (none of
    /// the production binaries do — they read exit_code and write to fd 1
    /// directly); unset = the drained prefix is NOT retained, and
    /// RunOutcome.stdout comes back EMPTY (only the final <1MB tail when
    /// no mid-run drain happened).
    pub stdout_capture: bool,
    /// Traceback frames of the propagating exception, OUTERMOST-FIRST
    /// (built at each frame exit; module frame pushes last).
    /// Entries: (qualname, lineno, filename) — the filename is the DEFINING
    /// module's __file__ (T2 gate finding 2: per-frame file attribution).
    /// A5 (memperf audit 2026-11-06): VecDeque so the head-records during an
    /// unwind are O(1) per frame instead of O(depth) element shifts (the
    /// old Vec::insert(0, ...) made a depth-d unwind O(depth^2)). All
    /// consumers use len()/iter()/insert(idx)/truncate/drain, which VecDeque
    /// provides with identical logical ordering.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub tb_frames: Mutex<std::collections::VecDeque<(String, usize, String)>>,
    /// Per-frame current-line trackers (top = innermost frame).
    /// 02.09b P7 (decision 2): Mutex — worker executors share the
    /// Interpreter; this stack is per-mutator execution state that other
    /// threads legitimately read (f_back stubs).
    pub frame_stack: Mutex<Vec<Arc<Cell<usize>>>>,
    /// Recursion depth + limit (RecursionError at ~1000, task brief §5).
    /// 02.09b P7: Mutex — the counter is shared (workers' depths sum; a
    /// false RecursionError would only fire under extreme concurrency, and
    /// the sum is actually the RIGHT process bound for the 256MB stack
    /// shared by... no: each worker has its OWN stack. The Mutex keeps the
    /// accounting sound without per-thread tables; enter/exit park rarely.
    pub depth: Mutex<usize>,
    pub recursion_limit: Mutex<usize>,
    /// sys.exit code (SystemExit(n)) — captured at raise site.
    /// 02.09b P7: Mutex (SystemExit code capture).
    pub exit_code: Mutex<Option<i32>>,
    /// The in-flight exception for bare `raise` / implicit chaining.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub current_exception: Mutex<Option<Arc<ExceptionValue>>>,
    /// The module function registry (set once at entry).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub registry: Mutex<Option<Arc<crate::interp::FnRegistry>>>,
    /// Lambda bodies by fn id (lambdas eval an expression, not a block).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub lambda_bodies: Mutex<HashMap<u32, ap_ir::Expr>>,
    /// Defining frames by fn id (closure/defaults scope wiring).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub defining_frames: Mutex<HashMap<u32, std::sync::Arc<Frame>>>,
    /// Generator yield sink (v0 stream materialization).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub gen_sink: Mutex<Vec<Vec<ap_obj::value::Value>>>,
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub active_gen_sink: Mutex<Option<Arc<RefCell<Vec<ap_obj::value::Value>>>>>,
    /// PEP-342: the return value of the generator that JUST finished
    /// (StopIteration.value; S-02-22). Set by the yield-site return path.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub si_value: Mutex<Option<ap_obj::value::Value>>,
    /// The sys module object (lazy singleton for `import sys`).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub sys_module: Mutex<Option<Value>>,
    /// Interned builtin type objects (__class__ returns the SAME object
    /// every call so `x.__class__ is T` holds; V045).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub type_objects: Mutex<HashMap<String, Value>>,
    /// Statement start-lines in source order + running ordinal (traceback
    /// line mapping; the IR is span-free).
    pub stmt_lines: Mutex<Vec<usize>>,
    /// 02.09b P7 (decision 2): Mutex — the statement ordinal is
    /// execution-order state shared across worker executors (traceback
    /// line mapping).
    pub stmt_ordinal: Mutex<usize>,
    /// Real exception CLASSES by name (02.03: the BaseException tree as
    /// TypeObject-backed class values; populated at entry).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub exc_classes: Mutex<HashMap<String, Value>>,
    /// User-subclass edges (child name -> parent name) for __subclasses__.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub user_subclasses: Mutex<Vec<(String, String)>>,
    /// User class values by name (identity for __subclasses__ returns).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub user_class_values: Mutex<HashMap<String, Value>>,
    /// The `object` root class payload (lazy intern).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub object_class: Mutex<Option<std::sync::Arc<ap_obj::classes::TypeObject>>>,
    /// The `type` metaclass payload (lazy intern).
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub type_class: Mutex<Option<std::sync::Arc<ap_obj::classes::TypeObject>>>,
    /// Active super() context per frame depth: (start-class, self/cls)
    /// stacked so zero-arg super() inside a method resolves (S-03-17/18).
    /// 02.09b P7 (decision 2): Mutex — the super() context stack is shared
    /// executor state (every worker's method calls push/pop it).
    pub super_stack: Mutex<Vec<(std::sync::Arc<ap_obj::classes::TypeObject>, Value, bool)>>,
    /// A6 (memperf audit): interned plain-function super() sentinels —
    /// (non-mcap, mcap). One full TypeObject + fresh PyDict used to be
    /// built on EVERY function call as the super_stack sentinel; nothing
    /// mutates a sentinel or compares it by identity, so the two variants
    /// are minted once per interpreter and cloned by refcount.
    pub plain_super_ctx: (
        std::sync::Arc<ap_obj::classes::TypeObject>,
        std::sync::Arc<ap_obj::classes::TypeObject>,
    ),
    /// Context staged by instantiate() for the __new__/__init__ call it is
    /// about to make (picked up by call_function after its sentinel push).
    pub pending_super_ctx:
        Mutex<Option<(std::sync::Arc<ap_obj::classes::TypeObject>, Value, bool)>>,
    pub source: String,
    pub script_path: String,
    /// Self handle (set at run_module entry) so &Interpreter surfaces can
    /// reach the Arc'd interpreter for re-entrant calls.
    /// 02.09b P7 (decision 2): Mutex (shared executor state).
    pub self_rc: Mutex<Option<std::sync::Weak<Interpreter>>>,
}

// 02.09b P7 post-flip marker (supersedes the 02.09a GIL-era justification —
// that contract is DELETED with the GIL). Soundness now rests on decision 2:
// every shared executor interior on Interpreter (frame_stack, registry,
// iterators, generators, super_stack, current_exception, stdout, tb_frames,
// depth, exit_code, ...) is a Mutex/RwLock with a documented cross-thread
// contract at the field; FnRegistry is RwLock (F1); refcount traffic is Arc
// (Decision 1). The remaining Cell fields (depth/recursion_limit/exit_code
// were converted too) — no Cell remains on a cross-thread path. Any new
// RefCell/Cell added to this struct breaks the marker's soundness proof.
unsafe impl Send for Interpreter {}
unsafe impl Sync for Interpreter {}

impl Interpreter {
    /// The Arc'd handle (panics when called before run_module sets it).
    pub fn rc(&self) -> std::sync::Arc<Interpreter> {
        self.self_rc
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .and_then(|w| w.upgrade())
            .expect("interpreter self handle set at entry")
    }
}

/// Nested function ids start above the module-level count (module tables
/// are small; the offset avoids id collisions between the two tables).
pub const NESTED_BASE: u32 = 1_000_000;

thread_local! {
    /// G4 (oom-bughunt-2026-11-06): nested-def/lambda dedupe,
    /// CODE-OBJECT identity -> registered nested-fn id. Key =
    /// (defining scope_path, def/lambda source line, AST-node addr) —
    /// stable across calls of the same enclosing function, so hot loops
    /// re-executing a def statement register ONCE (CPython shares one
    /// code object the same way). Bounded: FIFO eviction past
    /// NESTED_DEDUPE_CAP entries (eviction only costs re-registration of
    /// a long-cold def — semantics unchanged).
    pub static NESTED_DEDUPE:
        std::cell::RefCell<HashMap<(String, usize, usize, u64), u32>> =
        std::cell::RefCell::new(HashMap::new());
    /// Insertion-order ledger for the NESTED_DEDUPE cap (sre.rs style).
    static NESTED_DEDUPE_ORDER: std::cell::RefCell<Vec<(String, usize, usize, u64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// G4-repair table cap (mirrors SRE_TABLE_CAP, sre.rs).
const NESTED_DEDUPE_CAP: usize = 1024;

/// FIFO-capped insert into NESTED_DEDUPE (oldest key evicted past cap).
pub fn nested_dedupe_insert(key: (String, usize, usize, u64), id: u32) {
    NESTED_DEDUPE.with(|d| {
        let mut map = d.borrow_mut();
        if !map.contains_key(&key) {
            NESTED_DEDUPE_ORDER.with(|l| {
                let mut ledger = l.borrow_mut();
                ledger.push(key.clone());
                if ledger.len() > NESTED_DEDUPE_CAP {
                    let oldest = ledger.remove(0);
                    map.remove(&oldest);
                }
            });
        }
        map.insert(key, id);
    });
}

impl Interpreter {
    pub fn new(source: String, script_path: String) -> Interpreter {
        let globals = PyDict::new();
        let builtins_d = PyDict::new();
        Interpreter {
            globals,
            builtins: builtins_d,
            functions: Mutex::new(Vec::new()),
            nested_functions: Mutex::new(Vec::new()),
            iterators: Mutex::new(HashMap::new()),
            generators: Mutex::new(HashMap::new()),
            next_iter_id: std::sync::atomic::AtomicU64::new(2),
            next_gen_id: std::sync::atomic::AtomicU64::new(1),
            stdout: Mutex::new(Vec::new()),
            stdout_flushed_prefix: Mutex::new(Vec::new()),
            stdout_capture: false,
            tb_frames: Mutex::new(std::collections::VecDeque::new()),
            frame_stack: Mutex::new(Vec::new()),
            depth: Mutex::new(0),
            recursion_limit: Mutex::new(1000),
            exit_code: Mutex::new(None),
            current_exception: Mutex::new(None),
            registry: Mutex::new(None),
            lambda_bodies: Mutex::new(HashMap::new()),
            defining_frames: Mutex::new(HashMap::new()),
            gen_sink: Mutex::new(Vec::new()),
            active_gen_sink: Mutex::new(None),
            si_value: Mutex::new(None),
            sys_module: Mutex::new(None),
            type_objects: Mutex::new(HashMap::new()),
            stmt_lines: Mutex::new(Vec::new()),
            stmt_ordinal: Mutex::new(0),
            exc_classes: Mutex::new(HashMap::new()),
            user_subclasses: Mutex::new(Vec::new()),
            user_class_values: Mutex::new(HashMap::new()),
            object_class: Mutex::new(None),
            type_class: Mutex::new(None),
            super_stack: Mutex::new(Vec::new()),
            plain_super_ctx: {
                // 02.09a P4: build the sentinel in two steps so the MRO
                // Mutex can be seeded with the Arc itself (write-once, same
                // shape as TypeObject::create's self-reference splice).
                let mk = |qual: &str| -> std::sync::Arc<ap_obj::classes::TypeObject> {
                    let placeholder = std::sync::Arc::new(ap_obj::classes::TypeObject {
                        name: "__plain_fn__".to_string(),
                        qualname: qual.to_string(),
                        bases: vec![],
                        mro: std::sync::Mutex::new(Vec::new()),
                        dict: ap_obj::dict::PyDict::new(),
                        flags: ap_obj::classes::TypeFlags::default(),
                        metaclass: std::sync::RwLock::new(None),
                        class_value: std::sync::RwLock::new(None),
                        // 02.05b: super_stack sentinels never mutate (A6
                        // audit: "nothing mutates a sentinel") — counters
                        // stay at seed and never guard an IC.
                        type_version: std::sync::atomic::AtomicU64::new(1),
                        surgery_version: std::sync::atomic::AtomicU64::new(1),
                    });
                    // Seed the write-once MRO = [self] BEFORE any observer
                    // can see the Arc (identical discipline to `create`).
                    *placeholder.mro.lock().unwrap() = vec![placeholder.clone()];
                    placeholder
                };
                (mk(""), mk("__mcap__"))
            },
            pending_super_ctx: Mutex::new(None),
            source,
            script_path,
            self_rc: Mutex::new(None),
        }
    }

    pub fn write_out(&self, s: &str) {
        const STDOUT_FLUSH_THRESHOLD: usize = 1 << 20; // 1MB (G6)
        let mut buf = self.stdout.lock().unwrap_or_else(|p| p.into_inner());
        buf.extend_from_slice(s.as_bytes());
        // G6 (oom-bughunt-2026-11-06 static audit): the buffer used to grow
        // for the WHOLE run (1GB of print output = 1GB RSS before the single
        // end-of-run flush). Past a threshold, drain the buffer to the REAL
        // stdout (fd 1) — both production callers (ap-cli, ap_run_probe) pass
        // std::io::stdout(), and differential suites consume the subprocess's
        // fd 1, so byte-order is preserved. G6-repair: the drained prefix is
        // retained in stdout_flushed_prefix ONLY when stdout_capture is on
        // (an embedder that consumes RunOutcome.stdout); the default keeps
        // nothing — fd 1 already has the bytes and the production callers
        // never read RunOutcome.stdout (verifier: 2x full output resident).
        if buf.len() >= STDOUT_FLUSH_THRESHOLD {
            let n = buf.len();
            let ok = unsafe { libc::write(1, buf.as_ptr() as *const libc::c_void, n) };
            if ok == n as isize {
                if self.stdout_capture {
                    self.stdout_flushed_prefix
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .extend_from_slice(&buf[..]);
                }
                buf.clear();
            }
            // A short/failed write keeps the bytes buffered; the end-of-run
            // flush still writes them (correctness over peak RSS).
        }
    }

    /// Function entry lookup (module table or nested table by id offset).
    /// 02.04a: module-level ids resolve through the CURRENT registry (the
    /// importer appends imported modules' defs there — the snapshot in
    /// `functions` only knows the main module).
    pub fn fn_entry(&self, fn_id: u32) -> Option<FnEntry> {
        if fn_id >= NESTED_BASE {
            self.nested_functions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get((fn_id - NESTED_BASE) as usize)
                .cloned()
        } else if let Some(reg) = self
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            reg.entry(fn_id)
        } else {
            self.functions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(fn_id as usize)
                .cloned()
        }
    }

    /// Register a nested def/lambda; returns its id (NESTED_BASE-offset).
    /// G4 (oom-bughunt-2026-11-06): re-registrations of the SAME def
    /// statement (a hot function defining a helper per call,
    /// `sorted(x, key=lambda ...)` in a loop) used to append a fresh
    /// FnEntry (full FnDef AST clone) + defining_frames Arc entry PER
    /// EXECUTION — an unbounded per-call leak. G4-repair: the dedupe key
    /// is the CODE-OBJECT identity — (defining frame's scope_path,
    /// def/lambda source line, AST-node address) — all STABLE across
    /// calls of the same enclosing function (the wave-2 fix's frame
    /// pointer was fresh every call and deduped nothing). CPython mints
    /// a new function OBJECT per execution but shares one CODE object;
    /// this key is that code-object identity. The call-site refreshes
    /// defining_frames[id] per call (fresh per-call closure target), so
    /// no stale frame pins here.
    pub fn register_nested_for(
        &self,
        entry: FnEntry,
        ast_node: usize,
        defining: &Arc<Frame>,
    ) -> u32 {
        let key = (
            defining.scope_path.clone(),
            defining.def_line.get(),
            ast_node,
            defining.frame_id,
        );
        let existing = NESTED_DEDUPE.with(|d| d.borrow().get(&key).copied());
        if let Some(id) = existing {
            return id;
        }
        let id = self.register_nested(entry);
        nested_dedupe_insert(key, id);
        id
    }

    pub fn register_nested(&self, entry: FnEntry) -> u32 {
        let mut t = self
            .nested_functions
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let id = NESTED_BASE + t.len() as u32;
        t.push(entry);
        id
    }

    /// Advance depth for a call; Err when over the recursion limit.
    pub fn enter_call(&self) -> Result<(), crate::Flow> {
        // 02.09b P7: Mutex (see the field note) — read + write under one
        // critical section so concurrent workers cannot lose an increment.
        let mut dg = self.depth.lock().unwrap_or_else(|p| p.into_inner());
        let d = *dg + 1;
        // F4 (oom-bughunt-2026-11-06 RSS doc): the Python limit is
        // user-settable (setrecursionlimit(5000)) but the interpreter thread
        // has a FIXED stack (256MB reservation; ~50KB native per Python
        // frame in RELEASE, ~4x in debug). Depth beyond the native budget
        // SIGABRTs (rc=-6) instead of raising RecursionError. Backstop:
        // derive the hard cap from the same budget the thread spawn uses;
        // setrecursionlimit still lowers it freely, but a raised limit can
        // no longer push past the stack.
        const FRAME_STACK_BYTES: usize = 256 << 20;
        #[cfg(debug_assertions)]
        const BYTES_PER_FRAME: usize = 256 * 1024; // debug frames ~5x release
        #[cfg(not(debug_assertions))]
        const BYTES_PER_FRAME: usize = 50 * 1024;
        let hard_cap = (FRAME_STACK_BYTES / BYTES_PER_FRAME).max(1000);
        if d > *self
            .recursion_limit
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            || d > hard_cap
        {
            return Err(crate::Flow::Raise(ExceptionValue::msg(
                "RecursionError",
                "maximum recursion depth exceeded",
            )));
        }
        *dg = d;
        Ok(())
    }

    pub fn exit_call(&self) {
        let mut dg = self.depth.lock().unwrap_or_else(|p| p.into_inner());
        *dg = dg.saturating_sub(1);
    }
}

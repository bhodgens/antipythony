//! Frames + name resolution (task brief §1): a global dict, per-frame
//! locals, cells/frees through shared `RefCell` slots (02.04 revisits the
//! representation), builtin fallback chain, and the interpreter state
//! (function table, iterator table, generator registry, output buffer).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::exc_hier::ExceptionValue;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::Value;

/// One shared cell: closure variable storage (S-02-12/13).
pub type CellSlot = Rc<RefCell<Option<Value>>>;

/// A single execution frame.
pub struct Frame {
    /// Locals (module frames keep this empty and use `globals` directly).
    pub locals: RefCell<HashMap<String, Loc>>,
    /// The module-level dict this frame reads globals from.
    pub globals: Gc<PyDict>,
    /// Scope path for classification lookup ("" for module).
    pub scope_path: String,
    /// Cells THIS function owns (its `cells` classification).
    pub my_cells: HashMap<String, CellSlot>,
    /// Cells captured from enclosing scopes (its `frees`).
    pub free_cells: HashMap<String, CellSlot>,
    /// Names declared `global` in this scope (S-02-3): stores go to globals.
    pub declared_globals: std::collections::HashSet<String>,
    /// Names declared `nonlocal` (S-02-4).
    pub declared_nonlocals: std::collections::HashSet<String>,
    /// Qualname for traceback rendering.
    pub qualname: String,
    /// Wave-58: the last executed statement's source line IN THIS
    /// frame. The old frame_stack-of-cells approach let a parent
    /// frame's touch_line overwrite a child's cell mid-unwind (shared
    /// fs.last() writes), so traceback entries recorded the caller's
    /// line. Per-frame cell is written only by this frame's exec_stmt.
    pub last_line: Cell<usize>,
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
    Slot(Rc<RefCell<Option<Value>>>),
    /// A shared cell (closure traffic).
    CellSlot(CellSlot),
}

impl Frame {
    pub fn new_module(globals: Gc<PyDict>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            globals,
            scope_path: String::new(),
            my_cells: HashMap::new(),
            free_cells: HashMap::new(),
            declared_globals: std::collections::HashSet::new(),
            declared_nonlocals: std::collections::HashSet::new(),
            qualname: "<module>".to_string(),
            is_module: true,
            last_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: None,
            class_ns_inst: None,
        }
    }

    /// A function frame with pre-bound locals and closure cell sets.
    pub fn new_function(
        globals: Gc<PyDict>,
        scope_path: &str,
        qualname: &str,
        declared_globals: std::collections::HashSet<String>,
        declared_nonlocals: std::collections::HashSet<String>,
        my_cells: HashMap<String, CellSlot>,
        free_cells: HashMap<String, CellSlot>,
        locals: HashMap<String, Loc>,
    ) -> Frame {
        Frame {
            locals: RefCell::new(locals),
            globals,
            scope_path: scope_path.to_string(),
            my_cells,
            free_cells,
            declared_globals,
            declared_nonlocals,
            qualname: qualname.to_string(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
            class_qual_prefix: String::new(),
            class_cell: std::cell::RefCell::new(None),
            class_ns: None,
            class_ns_inst: None,
        }
    }

    /// A comprehension/child scope that falls back to `parent` (v0: the
    /// child shares the parent's globals; comp locals are isolated here).
    pub fn new_child(parent: &Rc<Frame>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            globals: parent.globals.clone(),
            scope_path: parent.scope_path.clone(),
            my_cells: HashMap::new(),
            free_cells: parent.my_cells.clone(),
            declared_globals: std::collections::HashSet::new(),
            declared_nonlocals: std::collections::HashSet::new(),
            qualname: parent.qualname.clone(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
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
    pub fn new_class(parent: &Rc<Frame>, ns: Gc<PyDict>) -> Frame {
        Frame {
            locals: RefCell::new(HashMap::new()),
            globals: parent.globals.clone(),
            scope_path: parent.scope_path.clone(),
            my_cells: HashMap::new(),
            free_cells: HashMap::new(),
            declared_globals: std::collections::HashSet::new(),
            declared_nonlocals: std::collections::HashSet::new(),
            qualname: parent.qualname.clone(),
            is_module: false,
            last_line: std::cell::Cell::new(0),
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
                locals.insert(name.to_string(), Loc::Slot(Rc::new(RefCell::new(Some(v)))));
            }
        }
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

pub enum Lookup {
    Found(Value),
    /// Local slot exists but is unbound (UnboundLocalError, S-02-2).
    UnboundLocal,
    /// Not local/cell/global — caller checks builtins then raises NameError.
    Miss,
}

/// A registered function (function-table entry).
#[derive(Clone)]
pub struct FnEntry {
    pub def: ap_ir::item::FnDef,
    /// Dotted scope path matching ScopeInfoModule.
    pub scope_path: String,
    /// Qualname for tracebacks.
    pub qualname: String,
    /// 02.04a: the DEFINING module's globals (functions from imported
    /// modules must resolve names in THEIR module, not the runner's).
    /// Construction sites default this to interp.globals.
    pub globals: Gc<PyDict>,
}

/// The interpreter: owns module state, function table, iterator table,
/// generator registry, output buffer, traceback accumulation.
pub struct Interpreter {
    pub globals: Gc<PyDict>,
    pub builtins: Gc<PyDict>,
    /// Module-level function table (index == fn_id).
    pub functions: RefCell<Vec<FnEntry>>,
    /// Nested/lambda functions (ids offset by NESTED_BASE).
    pub nested_functions: RefCell<Vec<FnEntry>>,
    pub iterators: RefCell<HashMap<u64, Gc<crate::iter::PyIterator>>>,
    pub generators: RefCell<HashMap<u64, crate::interp::GenFrame>>,
    /// Iterator ids are EVEN, generator ids ODD (both share the iterators
    /// table; disjoint namespaces prevent key collisions).
    pub next_iter_id: Cell<u64>,
    pub next_gen_id: Cell<u64>,
    /// stdout accumulation (S-06-2 byte gate).
    pub stdout: RefCell<Vec<u8>>,
    /// G6: bytes already drained to fd 1 by the mid-run threshold flush —
    /// re-prepended into RunOutcome.stdout at the end-of-run flush so the
    /// outcome is byte-identical to the unbuffered behavior.
    pub stdout_flushed_prefix: RefCell<Vec<u8>>,
    /// Traceback frames of the propagating exception, OUTERMOST-FIRST
    /// (built by insert(0) at each frame exit; module frame pushes last).
    /// Entries: (qualname, lineno, filename) — the filename is the DEFINING
    /// module's __file__ (T2 gate finding 2: per-frame file attribution).
    pub tb_frames: RefCell<Vec<(String, usize, String)>>,
    /// Per-frame current-line trackers (top = innermost frame).
    pub frame_stack: RefCell<Vec<Rc<Cell<usize>>>>,
    /// Recursion depth + limit (RecursionError at ~1000, task brief §5).
    pub depth: Cell<usize>,
    pub recursion_limit: Cell<usize>,
    /// sys.exit code (SystemExit(n)) — captured at raise site.
    pub exit_code: Cell<Option<i32>>,
    /// The in-flight exception for bare `raise` / implicit chaining.
    pub current_exception: RefCell<Option<Rc<ExceptionValue>>>,
    /// The module function registry (set once at entry).
    pub registry: RefCell<Option<Rc<crate::interp::FnRegistry>>>,
    /// Lambda bodies by fn id (lambdas eval an expression, not a block).
    pub lambda_bodies: RefCell<HashMap<u32, ap_ir::Expr>>,
    /// Defining frames by fn id (closure/defaults scope wiring).
    pub defining_frames: RefCell<HashMap<u32, std::rc::Rc<Frame>>>,
    /// Generator yield sink (v0 stream materialization).
    pub gen_sink: RefCell<Vec<Vec<ap_obj::value::Value>>>,
    pub active_gen_sink: RefCell<Option<Rc<RefCell<Vec<ap_obj::value::Value>>>>>,
    /// PEP-342: the return value of the generator that JUST finished
    /// (StopIteration.value; S-02-22). Set by the yield-site return path.
    pub si_value: RefCell<Option<ap_obj::value::Value>>,
    /// The sys module object (lazy singleton for `import sys`).
    pub sys_module: RefCell<Option<Value>>,
    /// Interned builtin type objects (__class__ returns the SAME object
    /// every call so `x.__class__ is T` holds; V045).
    pub type_objects: RefCell<HashMap<String, Value>>,
    /// Statement start-lines in source order + running ordinal (traceback
    /// line mapping; the IR is span-free).
    pub stmt_lines: RefCell<Vec<usize>>,
    pub stmt_ordinal: Cell<usize>,
    /// Real exception CLASSES by name (02.03: the BaseException tree as
    /// TypeObject-backed class values; populated at entry).
    pub exc_classes: RefCell<HashMap<String, Value>>,
    /// User-subclass edges (child name -> parent name) for __subclasses__.
    pub user_subclasses: RefCell<Vec<(String, String)>>,
    /// User class values by name (identity for __subclasses__ returns).
    pub user_class_values: RefCell<HashMap<String, Value>>,
    /// The `object` root class payload (lazy intern).
    pub object_class: RefCell<Option<std::rc::Rc<ap_obj::classes::TypeObject>>>,
    /// The `type` metaclass payload (lazy intern).
    pub type_class: RefCell<Option<std::rc::Rc<ap_obj::classes::TypeObject>>>,
    /// Active super() context per frame depth: (start-class, self/cls)
    /// stacked so zero-arg super() inside a method resolves (S-03-17/18).
    pub super_stack: RefCell<Vec<(std::rc::Rc<ap_obj::classes::TypeObject>, Value, bool)>>,
    /// Context staged by instantiate() for the __new__/__init__ call it is
    /// about to make (picked up by call_function after its sentinel push).
    pub pending_super_ctx: RefCell<Option<(std::rc::Rc<ap_obj::classes::TypeObject>, Value, bool)>>,
    pub source: String,
    pub script_path: String,
    /// Self handle (set at run_module entry) so &Interpreter surfaces can
    /// reach the Rc'd interpreter for re-entrant calls.
    pub self_rc: RefCell<Option<std::rc::Weak<Interpreter>>>,
}

impl Interpreter {
    /// The Rc'd handle (panics when called before run_module sets it).
    pub fn rc(&self) -> std::rc::Rc<Interpreter> {
        self.self_rc
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .expect("interpreter self handle set at entry")
    }
}

/// Nested function ids start above the module-level count (module tables
/// are small; the offset avoids id collisions between the two tables).
pub const NESTED_BASE: u32 = 1_000_000;

thread_local! {
    /// G4 (oom-bughunt-2026-11-06): (AST-node addr, defining-frame addr) ->
    /// registered nested-fn id. Dedupes re-executions of the same def/lambda
    /// statement in the same frame so nested_functions/lambda_bodies/
    /// defining_frames stop growing per call in hot loops. Entries live for
    /// the interpreter's life — bounded by distinct (node, frame) pairs
    /// (CPython shares code objects the same way), NOT by executions.
    pub static NESTED_DEDUPE: std::cell::RefCell<HashMap<(usize, usize), u32>> =
        std::cell::RefCell::new(HashMap::new());
}

impl Interpreter {
    pub fn new(source: String, script_path: String) -> Interpreter {
        let globals = PyDict::new();
        let builtins_d = PyDict::new();
        Interpreter {
            globals,
            builtins: builtins_d,
            functions: RefCell::new(Vec::new()),
            nested_functions: RefCell::new(Vec::new()),
            iterators: RefCell::new(HashMap::new()),
            generators: RefCell::new(HashMap::new()),
            next_iter_id: Cell::new(2),
            next_gen_id: Cell::new(1),
            stdout: RefCell::new(Vec::new()),
            stdout_flushed_prefix: RefCell::new(Vec::new()),
            tb_frames: RefCell::new(Vec::new()),
            frame_stack: RefCell::new(Vec::new()),
            depth: Cell::new(0),
            recursion_limit: Cell::new(1000),
            exit_code: Cell::new(None),
            current_exception: RefCell::new(None),
            registry: RefCell::new(None),
            lambda_bodies: RefCell::new(HashMap::new()),
            defining_frames: RefCell::new(HashMap::new()),
            gen_sink: RefCell::new(Vec::new()),
            active_gen_sink: RefCell::new(None),
            si_value: RefCell::new(None),
            sys_module: RefCell::new(None),
            type_objects: RefCell::new(HashMap::new()),
            stmt_lines: RefCell::new(Vec::new()),
            stmt_ordinal: Cell::new(0),
            exc_classes: RefCell::new(HashMap::new()),
            user_subclasses: RefCell::new(Vec::new()),
            user_class_values: RefCell::new(HashMap::new()),
            object_class: RefCell::new(None),
            type_class: RefCell::new(None),
            super_stack: RefCell::new(Vec::new()),
            pending_super_ctx: RefCell::new(None),
            source,
            script_path,
            self_rc: RefCell::new(None),
        }
    }

    pub fn write_out(&self, s: &str) {
        const STDOUT_FLUSH_THRESHOLD: usize = 1 << 20; // 1MB (G6)
        let mut buf = self.stdout.borrow_mut();
        buf.extend_from_slice(s.as_bytes());
        // G6 (oom-bughunt-2026-11-06 static audit): the buffer used to grow
        // for the WHOLE run (1GB of print output = 1GB RSS before the single
        // end-of-run flush). Past a threshold, drain the buffer to the REAL
        // stdout (fd 1) — both production callers (ap-cli, ap_run_probe) pass
        // std::io::stdout(), and differential suites consume the subprocess's
        // fd 1, so byte-order is preserved. RunOutcome.stdout correctness for
        // capture consumers: the drained prefix is ALSO kept in
        // stdout_flushed_prefix, re-prepended at the end-of-run flush, so the
        // outcome bytes are identical to the unbuffered behavior.
        if buf.len() >= STDOUT_FLUSH_THRESHOLD {
            use std::io::Write as _;
            let n = buf.len();
            let ok = unsafe { libc::write(1, buf.as_ptr() as *const libc::c_void, n) };
            if ok == n as isize {
                self.stdout_flushed_prefix.borrow_mut().extend_from_slice(&buf[..]);
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
                .borrow()
                .get((fn_id - NESTED_BASE) as usize)
                .cloned()
        } else if let Some(reg) = self.registry.borrow().as_ref() {
            reg.entries.get(fn_id as usize).cloned()
        } else {
            self.functions.borrow().get(fn_id as usize).cloned()
        }
    }

    /// Register a nested def/lambda; returns its id (NESTED_BASE-offset).
    /// G4 (oom-bughunt-2026-11-06): re-registrations of the SAME def
    /// statement in the SAME defining frame (a hot function defining a
    /// helper per call, `sorted(x, key=lambda ...)` in a loop) used to
    /// append a fresh FnEntry (full FnDef AST clone) + defining_frames Rc
    /// entry PER EXECUTION — an unbounded per-call leak. Dedupe by
    /// (AST-node address, defining-frame address): CPython likewise mints
    /// a new function OBJECT per execution but shares one CODE object; the
    /// (node, frame) pair is our code-object identity.
    pub fn register_nested_for(&self, entry: FnEntry, ast_node: usize, defining: &Rc<Frame>) -> u32 {
        let key = (ast_node, Rc::as_ptr(defining) as *const u8 as usize);
        let existing = NESTED_DEDUPE.with(|d| d.borrow().get(&key).copied());
        if let Some(id) = existing {
            // The AST entry is already registered; keep the defining_frames
            // slot fresh (it points at THIS execution's frame — same Rc for
            // re-execution in one frame instance, so the insert is idempotent
            // but refresh guards frame-identity reuse).
            self.defining_frames.borrow_mut().insert(id, defining.clone());
            return id;
        }
        let id = self.register_nested(entry);
        NESTED_DEDUPE.with(|d| {
            d.borrow_mut().insert(key, id);
        });
        self.defining_frames
            .borrow_mut()
            .insert(id, defining.clone());
        id
    }

    pub fn register_nested(&self, entry: FnEntry) -> u32 {
        let mut t = self.nested_functions.borrow_mut();
        let id = NESTED_BASE + t.len() as u32;
        t.push(entry);
        id
    }

    /// Advance depth for a call; Err when over the recursion limit.
    pub fn enter_call(&self) -> Result<(), crate::Flow> {
        let d = self.depth.get() + 1;
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
        if d > self.recursion_limit.get() || d > hard_cap {
            return Err(crate::Flow::Raise(ExceptionValue::msg(
                "RecursionError",
                "maximum recursion depth exceeded",
            )));
        }
        self.depth.set(d);
        Ok(())
    }

    pub fn exit_call(&self) {
        self.depth.set(self.depth.get().saturating_sub(1));
    }
}

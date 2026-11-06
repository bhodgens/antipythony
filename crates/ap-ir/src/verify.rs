//! IR verification pass (leaf 01.04).
//!
//! Static checks before codegen, per the orchestrator leaf 01.04, the T-04
//! contract (`plans/antipythony/tests/T-04-ir-verify.md`), and the
//! compat-contract clauses cited on each check. The T-04 fixtures
//! (`tests/t04_ir_verify/fixtures/expected_t04.json`, pinned from CPython
//! 3.14.7) are normative for which probes MUST flag and which MUST stay
//! silent; the analysis families below are derived from those pins:
//!
//! 1. **Unbound-local prediction** — S-02-2 + the fixtures' must-flag /
//!    must-silence split, which requires a MUST-analysis (definite-
//!    assignment) approximation, NOT a flat "never assigned anywhere" rule:
//!    - p02 (`if flag: y = 1; return y`) must flag: the only binding is
//!      conditional, so the read after it is unprotected;
//!    - p03 (`while cond: w = 1; return w`) must flag (same shape);
//!    - p05 (`for k in []: ...; return k`) must flag (empty iterable);
//!    - n03 (`for k in range(3): ...; return k`) must stay SILENT (range
//!      with a positive literal bound definitely binds `k`);
//!    - p01 (`print(z); z = 1`) must flag (read before the only binding);
//!    - p04 (`x += 1`) must flag (augassign reads first, S-02-19);
//!    - p06 (`[v for v in src]` before `src = [1,2]`) must flag;
//!    - p07 (`del q` never bound) must flag with the same class+message
//!      (UnboundLocalError, G-7);
//!    - p08 must flag a runtime-clean program (static rule, S-02-2).
//!    Message text is the byte-exact 3.14 template (byte-verified against
//!    CPython 3.14.7): `cannot access local variable 'x' where it is not
//!    associated with a value`. Control-flow is treated optimistically per
//!    C-11: any statement sequence along the syntactic order (branches,
//!    loops, try bodies, else/finally) counts as a potential protecting
//!    path; only reads with NO potential prior binding anywhere flag.
//! 2. **Name-resolution completeness** — S-02-1: every Name read resolves
//!    to (local | param | cell/free | global | builtin). A read that
//!    resolves NOWHERE is [`DiagKind::UnresolvedName`] with CPython's
//!    module-level miss text `name 'x' is not defined`. The suggestion
//!    engine is UNSPECIFIED for verify (T-04 G-5); the diagnostic is
//!    emitted only when no visible name is within CPython's suggestion
//!    distance (conservative d<=1 approximation), so boundary probes
//!    p19/p20/p21/p22 either stay silent or byte-match.
//! 3. **Conservative constant folding** — [`fold`] rewrites only
//!    provably-pure literal expressions: int/float arithmetic honoring
//!    C-12..C-16 (arbitrary-precision ints via string math, IEEE-754
//!    binary64, floor-div/mod sign rules, `**` promotion), string
//!    concatenation/repetition (S-01-14), and bool-op short-circuiting on
//!    literal operands (S-01-30). Never folds calls, names, attributes,
//!    comparisons, or operators that would raise (`/0`, `%0`, `//0`,
//!    negative shift) — those must stay runtime errors. Fold results are
//!    byte-identical to CPython 3.14 `eval` (differential-tested).
//!
//! The pass is pure: it consumes only `crate` types. The IR carries no
//! spans (vendor-clean rule), so [`Diagnostic`] carries the scope qualname
//! and read identifier; `ap-parse` re-derives line/col from its ruff AST
//! and attaches positions.

use std::fmt;

use crate::expr::{Arg, BinOp, BoolOp, CompClause, Expr, FStrPart, UnaryOp};
use crate::item::{ClassDef, FnDef, Item, Pattern, Stmt, Target, TryStmt};
use crate::module::Module;

// ===========================================================================
// Diagnostics
// ===========================================================================

/// Kind of a verification diagnostic; [`DiagKind::class`] maps it to the
/// CPython exception name the diagnostic predicts (the `<class>` field of
/// the T-04 C-10 diagnostic line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagKind {
    /// S-02-2: read of a local with no protecting binding (incl. `del q`
    /// of a never-bound local — fixture p07 / G-7).
    UnboundLocal,
    /// S-02-1/S-02-6: name that resolves to no scope level (and no close
    /// candidate for CPython's suggestion engine — G-5).
    UnresolvedName,
    /// Compile-time static checks CPython rejects in its symtable/compile
    /// stage (S-02-3/4/11/18, S-04-10, S-02-8 family; T-04 G-4: post-parse
    /// compile errors are verify's territory). Predicts SyntaxError.
    StaticError,
}

impl DiagKind {
    /// The CPython class name this diagnostic predicts.
    pub fn class(&self) -> &'static str {
        match self {
            DiagKind::UnboundLocal => "UnboundLocalError",
            DiagKind::UnresolvedName => "NameError",
            DiagKind::StaticError => "SyntaxError",
        }
    }
}

/// One verification diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Diagnostic kind (drives the predicted CPython class).
    pub kind: DiagKind,
    /// Dotted CPython-style qualname of the scope containing the finding
    /// (`f`, `make.<locals>.get`, `<module>` for the module scope).
    pub qualname: String,
    /// The offending identifier.
    pub name: String,
    /// Message text, byte-identical to CPython 3.14's error text.
    pub message: String,
    /// 1-based line of the offending read, when known. The pure IR pass
    /// leaves it `None`; the frontend position pass fills it in.
    pub line: Option<usize>,
    /// 1-based column (Unicode scalar values) anchored at CPython's FGE
    /// caret (S-02-37: under the read name only) where derivable; `None`
    /// when unknown.
    pub col: Option<usize>,
    /// Ordinal of the flagged read among the scope's loads of [`Self::name`]
    /// (0 = first read of that name in the scope). The frontend position map
    /// uses it to pick the matching ruff read site — e.g. del-then-read
    /// flags the SECOND read (post-del), never the del statement itself.
    pub load_ordinal: usize,
    /// Spec-pack / compat-contract clause this diagnostic preserves.
    pub clause: &'static str,
}

impl Diagnostic {
    /// Attach a resolved position (frontend-only; see module docs).
    pub fn with_position(mut self, line: usize, col: usize) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }
}

impl fmt::Display for Diagnostic {
    /// C-10 machine-stable shape without positions (the bin prepends
    /// `path:line:col `): `<class>: <qualname>: <message>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}: {}",
            self.kind.class(),
            self.qualname,
            self.message
        )
    }
}

// ===========================================================================
// verify(): unbound-local prediction + name-resolution completeness
// ===========================================================================

/// Verify a lowered module (S-02-2 unbound-local prediction + S-02-1
/// resolution completeness), scope by scope, in deterministic module order.
///
/// Pure: consumes only [`Module`]. Constant folding is a separate explicit
/// pass ([`fold`]) so callers control when the tree is rewritten.
pub fn verify(module: &Module) -> Vec<Diagnostic> {
    // Pass 1: collect the FULL module scope in one source-order walk
    // (bindings AND loads — loads feed the module-scope must-analysis in
    // pass 2; per-item scopes would reset the position counter and lose
    // cross-statement ordering, breaking del-then-read at module level).
    let mut module_scope = ScopeData::default();
    for item in &module.items {
        match item {
            Item::FnDef(f) => module_scope.bind_definite(&f.name),
            Item::ClassDef(c) => module_scope.bind_definite(&c.name),
            Item::Stmt(s) => collect_stmt(s, &mut module_scope),
        }
    }
    let module_bound = module_scope.bound.clone();

    // Pass 2: classify scopes top-down; parents pass their binding sets down
    // the enclosing-function chain (class scopes join only as suggestion
    // legs — S-02-5). The module scope itself resolves once, whole; the
    // module bindings are the base leg of EVERY scope's chain.
    let mut v = Verifier { diags: Vec::new() };
    v.resolve_scope(&module_scope, "<module>", &Vec::new());
    let module_leg = fn_leg(module_bound);
    for item in &module.items {
        match item {
            Item::FnDef(f) => v.verify_fn(f, &[], &module_scope, &vec![module_leg.clone()]),
            Item::ClassDef(c) => v.verify_class(c, &[], &module_scope, &vec![module_leg.clone()]),
            Item::Stmt(s) => v.nested_module_stmt(s, &module_scope, &vec![module_leg.clone()]),
        }
    }
    v.diags
}

/// Sorted v1 builtin name set (S-02-1's final resolution leg; spec 05
/// name-level coverage). Binary-searched.
static BUILTINS: &[&str] = &[
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "Ellipsis",
    "EncodingWarning",
    "Exception",
    "False",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "IOError",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "None",
    "NotADirectoryError",
    "NotImplemented",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "True",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
    "__annotations__",
    "__builtins__",
    "__debug__",
    "__doc__",
    "__file__",
    "__loader__",
    "__name__",
    "__package__",
    "__spec__",
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "hex",
    "id",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
];

fn is_builtin(name: &str) -> bool {
    BUILTINS.binary_search(&name).is_ok()
}

// ---------------------------------------------------------------------------
// Scope machinery — the pure-IR mirror of CPython's symtable pass plus a
// must-analysis (definite-assignment) overlay for unbound-local prediction.
//
// Positions are syntactic statement/expression ordinals assigned during a
// source-order collection walk (see `collect_stmt_at`). A binding is
// "definite at index i" when the statement at i always binds the name before
// any later statement runs (assignment/import/def; for-target over a
// statically non-empty iterable; loop/branch/try bodies protect later reads
// optimistically per C-11).
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct ScopeData {
    /// Names bound anywhere in the scope: assignment, augassign, ann-assign,
    /// for target, with-as, except-as, import, def/class name, del-name,
    /// walrus, match capture, parameters, `import *` flood marker.
    bound: Vec<String>,
    /// Names declared `global` (S-02-3).
    declared_global: Vec<String>,
    /// Names declared `nonlocal` (S-02-4).
    declared_nonlocal: Vec<String>,
    /// (name, index) of every read in source order (indices increase
    /// monotonically along the collection walk).
    loads: Vec<(String, usize)>,
    /// (name, index) of every DEFINITE binding in source order.
    definite_binds: Vec<(String, usize)>,
    /// Names bound by walrus (`:=`) in this scope's walk — comp-scope walrus
    /// binds leak to the enclosing scope (S-02-2), unlike comp targets.
    walrus_binds: Vec<String>,
    /// (name, index) of every `del NAME` — kills prior definite binds
    /// (del-then-read is CPython-verified UnboundLocalError).
    del_marks: Vec<(String, usize)>,
    /// Monotonic position counter for the collection walk.
    pos: usize,
}

/// Is this for-iterable statically guaranteed non-empty (so the target
/// definitely binds)? Clause: S-02-17 + fixture n03-vs-p05.
fn iter_definitely_binds(it: &Expr) -> bool {
    match it {
        // range(n) / range(a, b) with literal args: non-empty iff any arg
        // form guarantees > 0 iterations (range(0)/range(n, n) are empty).
        Expr::Call(f, args) => {
            if matches!(f.as_ref(), Expr::Name(n) if n == "range") {
                let vals: Vec<Option<i128>> = args
                    .iter()
                    .map(|a| match a {
                        Arg::Pos(Expr::LitInt(v)) => Some(*v),
                        _ => None,
                    })
                    .collect();
                match vals.len() {
                    1 => vals[0].is_some_and(|n| n > 0),
                    2 => match (vals[0], vals[1]) {
                        (Some(a), Some(b)) => b > a,
                        _ => false,
                    },
                    3 => false, // step forms: conservative
                    _ => false,
                }
            } else {
                // Literal non-empty containers bind their targets.
                match f.as_ref() {
                    Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => !xs.is_empty(),
                    Expr::LitStr(s) => !s.is_empty(),
                    _ => false,
                }
            }
        }
        Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => !xs.is_empty(),
        Expr::LitStr(s) => !s.is_empty(),
        _ => false,
    }
}

impl ScopeData {
    fn bind(&mut self, n: &str) {
        if !self.bound.iter().any(|b| b == n) {
            self.bound.push(n.to_string());
        }
    }
    /// A DEFINITE binding at the current walk position.
    fn bind_definite(&mut self, n: &str) {
        self.bind(n);
        self.definite_binds.push((n.to_string(), self.pos));
        self.pos += 1;
    }
    fn load(&mut self, n: &str) {
        self.loads.push((n.to_string(), self.pos));
        self.pos += 1;
    }
    fn is_bound(&self, n: &str) -> bool {
        self.bound.iter().any(|b| b == n)
    }
    /// Any DEFINITE binding recorded BEFORE `idx` and not subsequently
    /// killed by a `del` of the same name (del-then-read is CPython-
    /// verified UnboundLocalError: the del unbinds, so the pre-del bind
    /// no longer protects). A del at the SAME position as a read kills it
    /// (the del statement is the read's predecessor).
    fn has_binding_before(&self, n: &str, idx: usize) -> bool {
        let mut protected = false;
        for (name, i) in &self.definite_binds {
            if name != n || *i >= idx {
                continue;
            }
            protected = true;
            // Any del of n at or between this bind and idx kills it.
            for (dn, di) in &self.del_marks {
                if dn == n && *di >= *i && *di < idx {
                    protected = false;
                }
            }
        }
        protected
    }
    fn declared(&self, n: &str) -> bool {
        self.declared_global.iter().any(|g| g == n) || self.declared_nonlocal.iter().any(|l| l == n)
    }
    fn has_star_import(&self) -> bool {
        self.bound.iter().any(|b| b == "*")
    }
}

struct Verifier {
    diags: Vec<Diagnostic>,
}

/// Enclosing-function chain for resolution: legs innermost-first, module
/// leg last. Class scopes appear ONLY as suggestion-only legs (S-02-5:
/// never a free-variable source; their names suppress UnresolvedName the
/// way CPython's `self.x` suggestion does — G-5).
#[derive(Debug, Clone)]
struct EnclosingLeg {
    names: Vec<String>,
    /// Class-body leg: never resolves free reads, only silences.
    is_class: bool,
}

type Enclosing = Vec<EnclosingLeg>;

fn fn_leg(names: Vec<String>) -> EnclosingLeg {
    EnclosingLeg {
        names,
        is_class: false,
    }
}

fn class_leg(names: Vec<String>) -> EnclosingLeg {
    EnclosingLeg {
        names,
        is_class: true,
    }
}

impl Verifier {
    fn flag(
        &mut self,
        kind: DiagKind,
        qualname: &str,
        name: &str,
        clause: &'static str,
        load_ordinal: usize,
    ) {
        let message = match kind {
            DiagKind::UnboundLocal => format!(
                "cannot access local variable '{name}' where it is not associated with a value"
            ),
            DiagKind::UnresolvedName => format!("name '{name}' is not defined"),
            DiagKind::StaticError => {
                unreachable!("StaticError diagnostics are constructed directly")
            }
        };
        self.diags.push(Diagnostic {
            kind,
            qualname: qualname.to_string(),
            name: name.to_string(),
            message,
            line: None,
            col: None,
            load_ordinal,
            clause,
        });
    }

    /// Classify one completed scope's loads (S-02-1 + S-02-2 + the T-04
    /// must-analysis: a local read flags UnboundLocal iff NO binding of the
    /// name precedes it in the scope's source-order walk).
    fn resolve_scope(&mut self, scope: &ScopeData, qualname: &str, enclosing: &Enclosing) {
        if scope.has_star_import() {
            return; // `from m import *` may provide any name (conservative)
        }
        let mut reported: Vec<(String, usize)> = Vec::new();
        // Per-name read ordinal (the frontend map indexes reads per name).
        let mut name_ordinals: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for (load, idx) in &scope.loads {
            let ordinal = {
                let e = name_ordinals.entry(load.clone()).or_insert(0);
                let o = *e;
                *e += 1;
                o
            };
            // Local? Bound ANYWHERE in the scope = local for the ENTIRE body
            // (S-02-2, control-flow-insensitive). The unbound-local question
            // is then: does any binding precede this read syntactically?
            // EVERY read is checked (the del-read of del-then-read IS
            // protected, but the post-del read is not — CPython-verified).
            if scope.is_bound(load) {
                // If THIS load is the del-statement's own read (del_mark at
                // idx) with a prior protecting bind, skip: the binding is
                // live at the del itself; post-del reads carry the flag
                // (fixture p19 pins the READ site, line 3, not the del line).
                let is_del_read = scope.del_marks.iter().any(|(n, i)| n == load && *i == *idx)
                    && scope.has_binding_before(load, *idx);
                if is_del_read {
                    // Skip WITHOUT recording in `reported`: the post-del read
                    // must carry the flag + its own position (fixture p19
                    // pins the READ site line 3, not the del line 2).
                    continue;
                }
                let already = reported.iter().any(|(r, _)| r == load);
                if !scope.has_binding_before(load, *idx) && !already {
                    reported.push((load.clone(), ordinal));
                    if qualname == "<module>" {
                        // Module scope has no fast locals: del-then-read is
                        // the S-02-6 NameError form (fixture p19; G-7's
                        // UnboundLocalError is function-scope only).
                        self.flag(DiagKind::UnresolvedName, qualname, load, "S-02-6", ordinal);
                    } else {
                        // Read of a local with no protecting binding: the
                        // S-02-2 static UnboundLocalError form (fixtures
                        // p01..p08; class+message byte-verified vs CPython
                        // 3.14.7).
                        self.flag(DiagKind::UnboundLocal, qualname, load, "S-02-2", ordinal);
                    }
                }
                continue;
            }
            if scope.declared(load) {
                continue; // global/nonlocal declarations resolve elsewhere
            }
            // Enclosing FUNCTION scopes (S-02-1 leg 2): a static binding
            // there makes this a free/cell read (S-02-12). Runtime cell-miss
            // (p20) is outside the required gate set — T-04 G-3 optional.
            // Class-body legs (S-02-5) never resolve free reads; they only
            // feed the suggestion guard below (p22).
            if enclosing
                .iter()
                .any(|leg| !leg.is_class && leg.names.iter().any(|x| x == load))
            {
                continue;
            }
            if is_builtin(load) {
                continue; // S-02-1 leg 4
            }
            // Resolves nowhere (legs 1-4 exhausted). One diagnostic family:
            // module-level NameError text (S-02-6). CPython's "Did you
            // mean" suggestion suffix is part of str(exc) at runtime
            // (S-02-29) but UNSPECIFIED for verify (T-04 G-5); we stay
            // silent when any visible name (including class-body names —
            // the `self.x` suggestion case, p22) is a close candidate.
            if self.has_close_candidate(load, scope, enclosing) {
                continue;
            }
            self.flag(
                DiagKind::UnresolvedName,
                qualname,
                load,
                "S-02-1/S-02-6",
                ordinal,
            );
        }
    }

    /// Conservative G-5 guard: is any visible name within CPython's
    /// suggestion distance of `load`? (Cheap over-approximation of the
    /// real engine: edit distance <= 1 by insertion/deletion/substitution,
    /// case-insensitive substitution cost 0.)
    fn has_close_candidate(&self, load: &str, scope: &ScopeData, enclosing: &Enclosing) -> bool {
        let near = |cand: &str| edit_distance_le1(load, cand);
        if scope.bound.iter().any(|c| near(c)) {
            return true;
        }
        for leg in enclosing {
            if leg.names.iter().any(|c| near(c)) {
                return true;
            }
        }
        BUILTINS.iter().any(|c| near(c))
    }

    // -- scope walk ----------------------------------------------------------

    /// Verify one function scope; recurse into nested scopes.
    fn verify_fn(
        &mut self,
        f: &FnDef,
        path: &[String],
        module: &ScopeData,
        parent_enclosing: &Enclosing,
    ) {
        let mut scope_path: Vec<String> = path.to_vec();
        if !scope_path.is_empty() {
            scope_path.push("<locals>".to_string());
        }
        scope_path.push(f.name.clone());

        let mut scope = ScopeData::default();
        for p in &f.params {
            // Parameters bind at function entry: index 0, protecting.
            scope.bind_definite(&p.name);
        }
        for s in &f.body.blocks {
            collect_stmt(s, &mut scope);
        }

        // This scope's OWN resolution uses the parent chain only (its own
        // bindings were already classified as locals by `is_bound` above —
        // adding them to `enclosing` would mask unbound-local reads).
        // Children receive parent-chain + this scope's bindings.
        let qual = scope_path.join(".");
        self.resolve_scope(&scope, &qual, parent_enclosing);

        let mut child_enclosing: Enclosing = parent_enclosing.clone();
        child_enclosing.push(fn_leg(scope.bound.clone()));
        child_enclosing.push(fn_leg(module.bound.clone()));

        // Defaults / decorators / return annotation evaluate in the
        // ENCLOSING scope (S-02-7): classify against the parent chain.
        let parent_qual = qual_of(path);
        for d in &f.decorators {
            self.verify_value_expr(d, &parent_qual, parent_enclosing);
        }
        for p in &f.params {
            if let Some(d) = &p.default {
                self.verify_value_expr(d, &parent_qual, parent_enclosing);
            }
        }
        if let Some(r) = &f.return_ann {
            self.verify_value_expr(&r.expr, &parent_qual, parent_enclosing);
        }
        self.nested_stmts(&f.body.blocks, &scope_path, module, &child_enclosing);
    }

    /// Verify one class scope (S-02-5: own namespace, never an enclosing
    /// scope for nested functions).
    fn verify_class(
        &mut self,
        c: &ClassDef,
        path: &[String],
        module: &ScopeData,
        parent_enclosing: &Enclosing,
    ) {
        let mut scope_path: Vec<String> = path.to_vec();
        if !scope_path.is_empty() {
            scope_path.push("<locals>".to_string());
        }
        scope_path.push(c.name.clone());

        // Bases / metaclass / decorators evaluate in the ENCLOSING scope
        // (S-03-1/S-03-2).
        let parent_qual = qual_of(path);
        for e in c.bases.iter().chain(c.metaclass.iter()) {
            self.verify_value_expr(e, &parent_qual, parent_enclosing);
        }
        for d in &c.decorators {
            self.verify_value_expr(d, &parent_qual, parent_enclosing);
        }

        // Class body: its own namespace (S-03-3); loads resolve against the
        // parent chain + the class's own bindings.
        let mut scope = ScopeData::default();
        for s in &c.body {
            collect_stmt(s, &mut scope);
        }
        let mut enclosing: Enclosing = parent_enclosing.clone();
        enclosing.push(class_leg(scope.bound.clone()));
        enclosing.push(fn_leg(module.bound.clone()));
        let qual = scope_path.join(".");
        self.resolve_scope(&scope, &qual, &enclosing);

        // Nested defs inside the class SKIP the class namespace for free
        // lookups (S-02-5): pass the parent chain through unchanged.
        self.nested_stmts(&c.body, &scope_path, module, parent_enclosing);
    }

    /// Verify a value expression owned by scope `owner_qual` (defaults,
    /// annotations): its loads resolve against `enclosing`, and nested
    /// scopes inside are checked too.
    fn verify_value_expr(&mut self, e: &Expr, owner_qual: &str, enclosing: &Enclosing) {
        let mut scope = ScopeData::default();
        collect_expr_loads(e, &mut scope);
        self.resolve_scope(&scope, owner_qual, enclosing);
        self.nested_expr(e, owner_qual, enclosing);
    }

    fn nested_stmts(
        &mut self,
        stmts: &[Stmt],
        path: &[String],
        module: &ScopeData,
        enclosing: &Enclosing,
    ) {
        for s in stmts {
            self.nested_stmt(s, path, module, enclosing);
        }
    }

    fn nested_stmt(
        &mut self,
        s: &Stmt,
        path: &[String],
        module: &ScopeData,
        enclosing: &Enclosing,
    ) {
        match s {
            Stmt::LocalDef(f) => self.verify_fn(f, path, module, enclosing),
            Stmt::LocalClass(c) => self.verify_class(c, path, module, enclosing),
            Stmt::Line(_) => {}
            Stmt::Assign(_, v) => self.nested_expr(v, &qual_of(path), enclosing),
            Stmt::AugAssign(t, _, v) => {
                self.nested_target(t, path, enclosing);
                self.nested_expr(v, &qual_of(path), enclosing);
            }
            Stmt::AnnAssign(t, ann, v) => {
                self.nested_target(t, path, enclosing);
                self.nested_expr(&ann.expr, &qual_of(path), enclosing);
                if let Some(v) = v {
                    self.nested_expr(v, &qual_of(path), enclosing);
                }
            }
            Stmt::Return(v) => {
                if let Some(v) = v {
                    self.nested_expr(v, &qual_of(path), enclosing);
                }
            }
            Stmt::If(c, then, els) => {
                self.nested_expr(c, &qual_of(path), enclosing);
                self.nested_stmts(then, path, module, enclosing);
                self.nested_stmts(els, path, module, enclosing);
            }
            Stmt::While(c, body, els) => {
                self.nested_expr(c, &qual_of(path), enclosing);
                self.nested_stmts(body, path, module, enclosing);
                self.nested_stmts(els, path, module, enclosing);
            }
            Stmt::For(t, it, body, els) => {
                self.nested_target(t, path, enclosing);
                self.nested_expr(it, &qual_of(path), enclosing);
                self.nested_stmts(body, path, module, enclosing);
                self.nested_stmts(els, path, module, enclosing);
            }
            Stmt::With(items, body) => {
                for i in items {
                    self.nested_expr(&i.expr, &qual_of(path), enclosing);
                    if let Some(t) = &i.target {
                        self.nested_target(t, path, enclosing);
                    }
                }
                self.nested_stmts(body, path, module, enclosing);
            }
            Stmt::Try(t) => self.nested_try(t, path, module, enclosing),
            Stmt::Raise(e, cause) => {
                if let Some(e) = e {
                    self.nested_expr(e, &qual_of(path), enclosing);
                }
                if let Some(c) = cause {
                    self.nested_expr(c, &qual_of(path), enclosing);
                }
            }
            Stmt::Match(subj, cases) => {
                self.nested_expr(subj, &qual_of(path), enclosing);
                for c in cases {
                    if let Some(g) = &c.guard {
                        self.nested_expr(g, &qual_of(path), enclosing);
                    }
                    self.nested_stmts(&c.body, path, module, enclosing);
                }
            }
            Stmt::Assert(t, m) => {
                self.nested_expr(t, &qual_of(path), enclosing);
                if let Some(m) = m {
                    self.nested_expr(m, &qual_of(path), enclosing);
                }
            }
            Stmt::Del(targets) => {
                for t in targets {
                    self.nested_target(t, path, enclosing);
                }
            }
            Stmt::ExprStmt(e) => self.nested_expr(e, &qual_of(path), enclosing),
            Stmt::Global(_) | Stmt::Nonlocal(_) | Stmt::Import(_) | Stmt::ImportFrom(..) => {}
            Stmt::Pass | Stmt::Break | Stmt::Continue => {}
            Stmt::Line(_) => {}
            Stmt::Line(_) => {}
        }
    }

    fn nested_try(
        &mut self,
        t: &TryStmt,
        path: &[String],
        module: &ScopeData,
        enclosing: &Enclosing,
    ) {
        self.nested_stmts(&t.body, path, module, enclosing);
        for h in &t.handlers {
            if let Some(ty) = &h.ty {
                self.nested_expr(ty, &qual_of(path), enclosing);
            }
            self.nested_stmts(&h.body, path, module, enclosing);
        }
        self.nested_stmts(&t.orelse, path, module, enclosing);
        self.nested_stmts(&t.finalbody, path, module, enclosing);
    }

    fn nested_target(&mut self, t: &Target, path: &[String], enclosing: &Enclosing) {
        let qual = qual_of(path);
        match t {
            Target::Name(_) | Target::Starred(_) => {}
            Target::Subscript(o, k) => {
                self.nested_expr(o, &qual, enclosing);
                self.nested_expr(k, &qual, enclosing);
            }
            Target::Attribute(o, _) => self.nested_expr(o, &qual, enclosing),
            Target::Tuple(xs) => {
                for x in xs {
                    self.nested_target(x, path, enclosing);
                }
            }
        }
    }

    /// Find + verify nested scopes (lambdas, comprehensions) inside a value
    /// expression owned by scope `owner_qual`.
    fn nested_expr(&mut self, e: &Expr, owner_qual: &str, enclosing: &Enclosing) {
        match e {
            Expr::Lambda(params, body) => {
                let qual = format!("{owner_qual}.<locals>.<lambda>");
                let mut scope = ScopeData::default();
                for p in &params.params {
                    scope.bind(&p.name);
                }
                collect_expr_loads(body, &mut scope);
                self.resolve_scope(&scope, &qual, enclosing);
                self.nested_expr(body, &qual, enclosing);
            }
            Expr::ListComp(elt, clauses) => {
                self.verify_comp("<listcomp>", elt, None, clauses, owner_qual, enclosing);
            }
            Expr::SetComp(elt, clauses) => {
                self.verify_comp("<setcomp>", elt, None, clauses, owner_qual, enclosing);
            }
            Expr::Generator(elt, clauses) => {
                self.verify_comp("<genexpr>", elt, None, clauses, owner_qual, enclosing);
            }
            Expr::DictComp(key, val, clauses) => {
                self.verify_comp("<dictcomp>", val, Some(key), clauses, owner_qual, enclosing);
            }
            other => {
                for sub in child_exprs(other) {
                    self.nested_expr(&sub, owner_qual, enclosing);
                }
            }
        }
    }

    /// Verify one comprehension scope (S-02-2 own implicit scope; S-02-5:
    /// first iterable in the ENCLOSING scope, targets bind in the comp
    /// scope, iteration var does not leak).
    fn verify_comp(
        &mut self,
        kind: &str,
        elt: &Expr,
        key: Option<&Expr>,
        clauses: &[CompClause],
        owner_qual: &str,
        enclosing: &Enclosing,
    ) {
        // First iterable: ENCLOSING scope (S-02-5 exception).
        if let Some(first) = clauses.first() {
            let mut enc = ScopeData::default();
            collect_expr_loads(&first.iter, &mut enc);
            self.resolve_scope(&enc, owner_qual, enclosing);
            self.nested_expr(&first.iter, owner_qual, enclosing);
        }
        let qual = format!("{owner_qual}.<locals>.{kind}");
        let mut scope = ScopeData::default();
        for (i, c) in clauses.iter().enumerate() {
            if i > 0 {
                collect_expr_loads(&c.iter, &mut scope);
            }
            // Comp targets are reads of the iterable's elements only in the
            // unpacking sense; the name binding is DEFINITE per-iteration
            // (bind before element/ifs are evaluated — S-02-5/S-02-2).
            bind_comp_target_definite(&c.target, &mut scope);
            for f in &c.ifs {
                collect_expr_loads(f, &mut scope);
            }
        }
        collect_expr_loads(elt, &mut scope);
        if let Some(k) = key {
            collect_expr_loads(k, &mut scope);
        }
        self.resolve_scope(&scope, &qual, enclosing);
        // Nested scopes inside the comp scope's parts.
        for (i, c) in clauses.iter().enumerate() {
            if i > 0 {
                self.nested_expr(&c.iter, &qual, enclosing);
            }
            self.nested_expr(&c.target, &qual, enclosing);
            for f in &c.ifs {
                self.nested_expr(f, &qual, enclosing);
            }
        }
        self.nested_expr(elt, &qual, enclosing);
        if let Some(k) = key {
            self.nested_expr(k, &qual, enclosing);
        }
    }

    fn nested_module_stmt(&mut self, s: &Stmt, module: &ScopeData, enclosing: &Enclosing) {
        let qual = "<module>";
        match s {
            Stmt::Assign(_, v) => self.nested_expr(v, qual, enclosing),
            Stmt::AugAssign(_, _, v) => self.nested_expr(v, qual, enclosing),
            Stmt::AnnAssign(_, ann, v) => {
                self.nested_expr(&ann.expr, qual, enclosing);
                if let Some(v) = v {
                    self.nested_expr(v, qual, enclosing);
                }
            }
            Stmt::Return(v) => {
                if let Some(v) = v {
                    self.nested_expr(v, qual, enclosing);
                }
            }
            Stmt::ExprStmt(v) => self.nested_expr(v, qual, enclosing),
            Stmt::If(c, then, els) => {
                self.nested_expr(c, qual, enclosing);
                for st in then.iter().chain(els.iter()) {
                    self.nested_module_stmt(st, module, enclosing);
                }
            }
            Stmt::While(c, body, els) => {
                self.nested_expr(c, qual, enclosing);
                for st in body.iter().chain(els.iter()) {
                    self.nested_module_stmt(st, module, enclosing);
                }
            }
            Stmt::For(t, it, body, els) => {
                self.nested_target(t, &[qual.to_string()], enclosing);
                self.nested_expr(it, qual, enclosing);
                for st in body.iter().chain(els.iter()) {
                    self.nested_module_stmt(st, module, enclosing);
                }
            }
            Stmt::With(items, body) => {
                for i in items {
                    self.nested_expr(&i.expr, qual, enclosing);
                    if let Some(t) = &i.target {
                        self.nested_target(t, &[qual.to_string()], enclosing);
                    }
                }
                for st in body {
                    self.nested_module_stmt(st, module, enclosing);
                }
            }
            Stmt::Try(t) => {
                for st in t
                    .body
                    .iter()
                    .chain(t.orelse.iter())
                    .chain(t.finalbody.iter())
                {
                    self.nested_module_stmt(st, module, enclosing);
                }
                for h in &t.handlers {
                    if let Some(ty) = &h.ty {
                        self.nested_expr(ty, qual, enclosing);
                    }
                    for st in &h.body {
                        self.nested_module_stmt(st, module, enclosing);
                    }
                }
            }
            Stmt::Raise(e, cause) => {
                if let Some(e) = e {
                    self.nested_expr(e, qual, enclosing);
                }
                if let Some(c) = cause {
                    self.nested_expr(c, qual, enclosing);
                }
            }
            Stmt::Match(subj, cases) => {
                self.nested_expr(subj, qual, enclosing);
                for c in cases {
                    if let Some(g) = &c.guard {
                        self.nested_expr(g, qual, enclosing);
                    }
                    for st in &c.body {
                        self.nested_module_stmt(st, module, enclosing);
                    }
                }
            }
            Stmt::Assert(t, m) => {
                self.nested_expr(t, qual, enclosing);
                if let Some(m) = m {
                    self.nested_expr(m, qual, enclosing);
                }
            }
            Stmt::Del(targets) => {
                for t in targets {
                    self.nested_target(t, &[qual.to_string()], enclosing);
                }
            }
            _ => {}
        }
    }
}

fn qual_of(path: &[String]) -> String {
    if path.is_empty() {
        "<module>".to_string()
    } else {
        path.join(".")
    }
}

/// Merge a conditional block's scope (if/while bodies) into the parent:
/// names bind (locality, S-02-2) but their positions NEVER protect later
/// reads (p02/p03 flag) — except reads INSIDE the block, which were already
/// resolved against the block's own internal order. Loads climb so free
/// candidates still register.
fn merge_conditional(inner: &ScopeData, outer: &mut ScopeData) {
    for n in &inner.bound {
        outer.bind(n);
    }
    hoist_free_candidates(inner, outer);
    outer.pos += inner.pos;
}

/// Merge a definitely-executing block (for-body under a definitely-binding
/// target) into the parent: definite binds protect later reads.
fn merge_definite(inner: &ScopeData, outer: &mut ScopeData) {
    for (name, i) in &inner.definite_binds {
        outer.bind(name);
        outer.definite_binds.push((name.clone(), outer.pos + i));
    }
    for n in &inner.bound {
        outer.bind(n);
    }
    hoist_free_candidates(inner, outer);
    outer.pos += inner.pos;
}

// ---------------------------------------------------------------------------
// Load/binding collection (the symtable-shaped walk)
// ---------------------------------------------------------------------------

/// Collect all loads + bindings of one statement into `scope`. Nested scopes
/// (lambda/comprehension) collect internally; their non-local loads climb
/// into `scope` as free candidates (symtable behavior).
fn collect_stmt(s: &Stmt, scope: &mut ScopeData) {
    match s {
        Stmt::Assign(targets, v) => {
            collect_expr_loads(v, scope);
            for t in targets {
                collect_target_loads(t, scope);
                // Assignment = DEFINITE binding at this walk position.
                bind_target_names_definite(t, scope);
            }
        }
        Stmt::AugAssign(t, _, v) => {
            // Augassign READS the target first (S-02-19), then rebinds.
            // A Name target is a LOAD of the name (p04: `x += 1` with no
            // prior binding flags UnboundLocalError on the read).
            match t {
                Target::Name(n) => scope.load(n),
                _ => collect_target_loads(t, scope),
            }
            collect_expr_loads(v, scope);
            bind_target_names_definite(t, scope);
        }
        Stmt::AnnAssign(t, ann, v) => {
            collect_expr_loads(&ann.expr, scope);
            if let Some(v) = v {
                collect_expr_loads(v, scope);
            }
            collect_target_loads(t, scope);
            bind_target_names_definite(t, scope);
        }
        Stmt::Return(v) => {
            if let Some(v) = v {
                collect_expr_loads(v, scope);
            }
        }
        Stmt::If(c, then, els) => {
            collect_expr_loads(c, scope);
            // Branch binds are CONDITIONAL (the branch may never run — p02:
            // `if flag: y = 1; return y` still flags `return y`): collect
            // into a scratch scope so reads inside see inner order, but
            // register binds as non-protecting (locality only).
            let mut branch = ScopeData::default();
            for st in then.iter().chain(els.iter()) {
                collect_stmt(st, &mut branch);
            }
            merge_conditional(&branch, scope);
        }
        Stmt::While(c, body, els) => {
            collect_expr_loads(c, scope);
            // Loop-body binds are conditional (zero-iteration loop — p03).
            let mut body_scope = ScopeData::default();
            for st in body.iter().chain(els.iter()) {
                collect_stmt(st, &mut body_scope);
            }
            merge_conditional(&body_scope, scope);
        }
        Stmt::For(t, it, body, els) => {
            collect_expr_loads(it, scope);
            collect_target_loads(t, scope);
            if iter_definitely_binds(it) {
                // n03 case: range(3)/non-empty literal definitely binds.
                bind_target_names_definite(t, scope);
            } else {
                // p05 case: binding is conditional (zero-iteration possible).
                // Register as a binding (local classification, S-02-2) but at
                // a position AFTER the loop body so reads after the loop are
                // unprotected.
                bind_target_names(t, scope);
            }
            let mut body_scope = ScopeData::default();
            for st in body.iter().chain(els.iter()) {
                collect_stmt(st, &mut body_scope);
            }
            // A definitely-binding for-target makes its body's binds flow
            // into the outer scope at definite positions ONLY if the target
            // itself bound definitely (the body always runs when the loop
            // runs at least once); otherwise conditional.
            if iter_definitely_binds(it) {
                merge_definite(&body_scope, scope);
            } else {
                merge_conditional(&body_scope, scope);
            }
        }
        Stmt::With(items, body) => {
            for i in items {
                collect_expr_loads(&i.expr, scope);
                if let Some(t) = &i.target {
                    collect_target_loads(t, scope);
                    bind_target_names(t, scope);
                }
            }
            for st in body {
                collect_stmt(st, scope);
            }
        }
        Stmt::Try(t) => {
            // try/except path coverage: a name bound in the try body AND in
            // EVERY handler body binds on every exit path -> definite
            // (n04: `try: x = ... except: x = 0` protects `return x`).
            // All other binds inside are conditional (locality only); their
            // loads still climb so p08's early read is recorded and later
            // binds stay after it in walk order (read-before-bind flags).
            let mut try_body = ScopeData::default();
            for st in &t.body {
                collect_stmt(st, &mut try_body);
            }
            let mut handlers: Vec<ScopeData> = Vec::new();
            for h in &t.handlers {
                if let Some(ty) = &h.ty {
                    collect_expr_loads(ty, scope);
                }
                let mut hs = ScopeData::default();
                if let Some(n) = &h.name {
                    // except-as binds on its path (S-02-30 static binding).
                    hs.bind_definite(n);
                }
                for st in &h.body {
                    collect_stmt(st, &mut hs);
                }
                handlers.push(hs);
            }
            // Try-body binds: definite iff every handler also binds the name
            // (every exit path then carries a binding).
            let try_binds: Vec<String> = try_body.bound.clone();
            let try_start = scope.pos; // position where the try construct began
            let bind_base = try_start;
            for name in &try_binds {
                let all_handlers_bind =
                    !handlers.is_empty() && handlers.iter().all(|h| h.is_bound(name));
                if all_handlers_bind {
                    scope.bind(name);
                    scope.definite_binds.push((name.clone(), bind_base));
                } else {
                    scope.bind(name);
                }
            }
            scope.pos += try_body.pos;
            // Handler-definite binds: definite when the try body does NOT
            // bind the name (handler is the only binder on its path, so any
            // read after the try is bound on the handler path AND the body
            // path only if the body bound it — the all-path rule).
            for h in &handlers {
                let h_base = scope.pos;
                for (name, _) in &h.definite_binds {
                    if !try_body.is_bound(name) && !handlers.iter().any(|o| !o.is_bound(name)) {
                        scope.bind(name);
                        scope.definite_binds.push((name.clone(), h_base));
                    } else {
                        scope.bind(name);
                    }
                }
                scope.pos += h.pos;
            }
            // Loads inside try/handlers climb at the TRY-START base (the
            // reads happened before any later outer bind — p08's read of x
            // must precede the `x = 1` after the try).
            for (name, i) in &try_body.loads {
                scope
                    .loads
                    .push((name.clone(), try_start + (*i).min(try_body.pos)));
            }
            for h in &handlers {
                for (name, i) in &h.loads {
                    scope
                        .loads
                        .push((name.clone(), try_start + (*i).min(h.pos)));
                }
            }
            for st in t.orelse.iter().chain(t.finalbody.iter()) {
                collect_stmt(st, scope);
            }
        }
        Stmt::Raise(e, cause) => {
            if let Some(e) = e {
                collect_expr_loads(e, scope);
            }
            if let Some(c) = cause {
                collect_expr_loads(c, scope);
            }
        }
        Stmt::Match(subj, cases) => {
            collect_expr_loads(subj, scope);
            for c in cases {
                if let Some(g) = &c.guard {
                    collect_expr_loads(g, scope);
                }
                collect_pattern_loads(&c.pattern, scope);
                collect_pattern_binds(&c.pattern, scope);
                for st in &c.body {
                    collect_stmt(st, scope);
                }
            }
        }
        Stmt::Assert(t, m) => {
            collect_expr_loads(t, scope);
            if let Some(m) = m {
                collect_expr_loads(m, scope);
            }
        }
        Stmt::Global(names) => {
            for n in names {
                if !scope.declared_global.iter().any(|g| g == n) {
                    scope.declared_global.push(n.clone());
                }
            }
        }
        Stmt::Nonlocal(names) => {
            for n in names {
                if !scope.declared_nonlocal.iter().any(|l| l == n) {
                    scope.declared_nonlocal.push(n.clone());
                }
            }
        }
        Stmt::Import(aliases) => {
            for a in aliases {
                let bind = match &a.alias {
                    Some(n) => n.clone(),
                    None => a.path.first().cloned().unwrap_or_default(),
                };
                scope.bind_definite(&bind);
            }
        }
        Stmt::ImportFrom(_, aliases, star) => {
            for a in aliases {
                let bind = a
                    .alias
                    .clone()
                    .unwrap_or_else(|| a.path.last().cloned().unwrap_or_default());
                scope.bind_definite(&bind);
            }
            if *star {
                // `from m import *` floods the namespace (S-04-10 area);
                // every load resolves conservatively.
                scope.bind("*");
            }
        }
        Stmt::Del(targets) => {
            for t in targets {
                // del of a NAME is itself a read of the binding slot: a name
                // only ever del'd (never assigned) flags UnboundLocalError
                // at the `del` (fixture p07, class+message byte-verified;
                // T-04 G-7). The bind registration below keeps the name
                // local-classified (del-then-read is the local error form —
                // CPython 3.14.7 verified) but NON-protecting; the del_mark
                // kills prior definite binds (del-then-read flags).
                match t {
                    Target::Name(n) => {
                        // del_mark at the SAME position as the del's own
                        // read: the kill applies to every read from the del
                        // statement onward (the del-read itself stays
                        // protected — the bind is live at `del x` — via the
                        // is_del_read skip in resolve_scope).
                        scope.del_marks.push((n.clone(), scope.pos));
                        scope.load(n);
                    }
                    _ => collect_target_loads(t, scope),
                }
                bind_target_names(t, scope);
            }
        }
        Stmt::LocalDef(f) => {
            scope.bind_definite(&f.name);
            for d in &f.decorators {
                collect_expr_loads(d, scope);
            }
            for p in &f.params {
                if let Some(d) = &p.default {
                    collect_expr_loads(d, scope);
                }
            }
            if let Some(r) = &f.return_ann {
                collect_expr_loads(&r.expr, scope);
            }
        }
        Stmt::LocalClass(c) => {
            scope.bind_definite(&c.name);
            for e in c.bases.iter().chain(c.metaclass.iter()) {
                collect_expr_loads(e, scope);
            }
            for d in &c.decorators {
                collect_expr_loads(d, scope);
            }
        }
        Stmt::ExprStmt(e) => collect_expr_loads(e, scope),
        Stmt::Pass | Stmt::Break | Stmt::Continue => {}
        Stmt::Line(_) => {}
    }
}

/// Bind target names as DEFINITE bindings at the current walk position
/// (assignments/augassigns/ann-assigns — unconditional binding ops).
fn bind_target_names_definite(t: &Target, scope: &mut ScopeData) {
    match t {
        Target::Name(n) => scope.bind_definite(n),
        Target::Starred(x) => {
            if let Expr::Name(n) = &**x {
                scope.bind_definite(n);
            } else {
                collect_expr_loads(x, scope);
            }
        }
        Target::Tuple(xs) => {
            for x in xs {
                bind_target_names_definite(x, scope);
            }
        }
        Target::Subscript(..) | Target::Attribute(..) => {}
    }
}

fn bind_target_names(t: &Target, scope: &mut ScopeData) {
    match t {
        Target::Name(n) => scope.bind(n),
        Target::Starred(x) => {
            if let Expr::Name(n) = &**x {
                scope.bind(n);
            } else {
                collect_expr_loads(x, scope);
            }
        }
        Target::Tuple(xs) => {
            for x in xs {
                bind_target_names(x, scope);
            }
        }
        Target::Subscript(..) | Target::Attribute(..) => {}
    }
}

fn collect_target_loads(t: &Target, scope: &mut ScopeData) {
    match t {
        Target::Name(_) => {}
        Target::Subscript(o, k) => {
            collect_expr_loads(o, scope);
            collect_expr_loads(k, scope);
        }
        Target::Attribute(o, _) => collect_expr_loads(o, scope),
        Target::Tuple(xs) => {
            for x in xs {
                collect_target_loads(x, scope);
            }
        }
        Target::Starred(x) => collect_expr_loads(x, scope),
    }
}

fn collect_pattern_loads(p: &Pattern, scope: &mut ScopeData) {
    match p {
        Pattern::Literal(e) => collect_expr_loads(e, scope),
        Pattern::Value(path) => {
            // Dotted value pattern: first segment is a name READ.
            if let Some(first) = path.first() {
                scope.load(first);
            }
        }
        Pattern::Class(c, pos, kws) => {
            collect_expr_loads(c, scope);
            for sub in pos {
                collect_pattern_loads(sub, scope);
            }
            for (_, sub) in kws {
                collect_pattern_loads(sub, scope);
            }
        }
        Pattern::Group(inner) => collect_pattern_loads(inner, scope),
        Pattern::Sequence(items, rest) => {
            for sub in items {
                collect_pattern_loads(sub, scope);
            }
            if let Some(r) = rest {
                collect_pattern_loads(r, scope);
            }
        }
        Pattern::Mapping(items, _) => {
            for (k, v) in items {
                collect_pattern_loads(k, scope);
                collect_pattern_loads(v, scope);
            }
        }
        Pattern::Or(alts) => {
            for sub in alts {
                collect_pattern_loads(sub, scope);
            }
        }
        Pattern::Capture(_) | Pattern::Wildcard => {}
    }
}

fn collect_pattern_binds(p: &Pattern, scope: &mut ScopeData) {
    match p {
        Pattern::Capture(n) => scope.bind(n),
        Pattern::Group(inner) => collect_pattern_binds(inner, scope),
        Pattern::Sequence(items, rest) => {
            for sub in items {
                collect_pattern_binds(sub, scope);
            }
            if let Some(r) = rest {
                collect_pattern_binds(r, scope);
            }
        }
        Pattern::Mapping(items, rest) => {
            for (_, v) in items {
                collect_pattern_binds(v, scope);
            }
            if let Some(r) = rest {
                scope.bind(r);
            }
        }
        Pattern::Class(_, pos, kws) => {
            for sub in pos {
                collect_pattern_binds(sub, scope);
            }
            for (_, sub) in kws {
                collect_pattern_binds(sub, scope);
            }
        }
        Pattern::Or(alts) => {
            for sub in alts {
                collect_pattern_binds(sub, scope);
            }
        }
        Pattern::Literal(_) | Pattern::Value(_) | Pattern::Wildcard => {}
    }
}

/// Record every Name READ in `e` against `scope`. Nested scopes collect into
/// a fresh ScopeData; their non-local loads climb (free candidates).
fn collect_expr_loads(e: &Expr, scope: &mut ScopeData) {
    match e {
        Expr::Name(n) => scope.load(n),
        Expr::BinOp(_, a, b) => {
            collect_expr_loads(a, scope);
            collect_expr_loads(b, scope);
        }
        Expr::UnaryOp(_, x) | Expr::Starred(x) | Expr::Await(x) | Expr::DictExpand(x) => {
            collect_expr_loads(x, scope)
        }
        Expr::BoolOp(_, xs) | Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => {
            for x in xs {
                collect_expr_loads(x, scope);
            }
        }
        Expr::Compare(_, xs) => {
            for x in xs {
                collect_expr_loads(x, scope);
            }
        }
        Expr::Call(f, args) => {
            collect_expr_loads(f, scope);
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Star(x) | Arg::Kw(_, x) | Arg::KwStar(x) => {
                        collect_expr_loads(x, scope);
                    }
                }
            }
        }
        Expr::Attribute(obj, _) => collect_expr_loads(obj, scope),
        Expr::Subscript(obj, key) => {
            collect_expr_loads(obj, scope);
            collect_expr_loads(key, scope);
        }
        Expr::Slice(a, b, c) => {
            for x in [a, b, c].into_iter().flatten() {
                collect_expr_loads(x, scope);
            }
        }
        Expr::Dict(items) => {
            for i in items {
                collect_expr_loads(&i.key, scope);
                collect_expr_loads(&i.value, scope);
            }
        }
        Expr::Lambda(params, body) => {
            let mut inner = ScopeData::default();
            for p in &params.params {
                inner.bind(&p.name);
            }
            collect_expr_loads(body, &mut inner);
            hoist_free_candidates(&inner, scope);
        }
        Expr::ListComp(elt, clauses)
        | Expr::SetComp(elt, clauses)
        | Expr::Generator(elt, clauses) => {
            // First iterable evaluates in the ENCLOSING scope (S-02-5).
            if let Some(first) = clauses.first() {
                collect_expr_loads(&first.iter, scope);
            }
            let mut inner = ScopeData::default();
            collect_comp_inner(elt, None, clauses, &mut inner);
            hoist_free_candidates(&inner, scope);
            // Walrus targets bind in the enclosing scope, not the comp scope
            // (S-02-2 walrus rule; FINDING-3 companion).
            leak_comp_walrus_binds(&inner, scope);
        }
        Expr::DictComp(key, val, clauses) => {
            if let Some(first) = clauses.first() {
                collect_expr_loads(&first.iter, scope);
            }
            let mut inner = ScopeData::default();
            collect_comp_inner(val, Some(key), clauses, &mut inner);
            hoist_free_candidates(&inner, scope);
            leak_comp_walrus_binds(&inner, scope);
        }
        Expr::FString(parts) => {
            for p in parts {
                collect_fstr_part_loads(p, scope);
            }
        }
        Expr::IfExp(c, t, e) => {
            collect_expr_loads(c, scope);
            collect_expr_loads(t, scope);
            collect_expr_loads(e, scope);
        }
        Expr::Named(target, value) => {
            collect_expr_loads(value, scope);
            // `t := v` binds t in the enclosing scope (walrus rule, S-02-2).
            // DEFINITE bind: the value is evaluated and stored before any
            // later read at this walk position, so the bind protects them
            // (FINDING-3: the previous indefinite bind left walrus-bound
            // names unprotecting, flagging clean code as NameError).
            if let Expr::Name(n) = &**target {
                scope.bind_definite(n);
                if !scope.walrus_binds.iter().any(|b| b == n) {
                    scope.walrus_binds.push(n.to_string());
                }
            }
        }
        Expr::Yield(Some(x)) | Expr::YieldFrom(x) => collect_expr_loads(x, scope),
        Expr::Yield(None) => {}
        Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitComplex(..)
        | Expr::LitStr(_)
        | Expr::LitBytes(_)
        | Expr::LitBool(_)
        | Expr::LitNone
        | Expr::LitEllipsis => {}
    }
}

fn collect_comp_inner(
    elt: &Expr,
    key: Option<&Expr>,
    clauses: &[CompClause],
    scope: &mut ScopeData,
) {
    for (i, c) in clauses.iter().enumerate() {
        if i > 0 {
            collect_expr_loads(&c.iter, scope);
        }
        collect_expr_loads(&c.target, scope);
        bind_comp_target(&c.target, scope);
        for f in &c.ifs {
            collect_expr_loads(f, scope);
        }
    }
    collect_expr_loads(elt, scope);
    if let Some(k) = key {
        collect_expr_loads(k, scope);
    }
}

/// Comp targets are Expr-shaped (Name/Tuple/List/Starred): bind names.
fn bind_comp_target(t: &Expr, scope: &mut ScopeData) {
    match t {
        Expr::Name(n) => scope.bind(n),
        Expr::Starred(x) => {
            if let Expr::Name(n) = &**x {
                scope.bind(n);
            } else {
                collect_expr_loads(x, scope);
            }
        }
        Expr::Tuple(xs) | Expr::List(xs) => {
            for x in xs {
                bind_comp_target(x, scope);
            }
        }
        other => collect_expr_loads(other, scope),
    }
}

/// Definite variant: comp targets bind every iteration before the element
/// / filters evaluate (S-02-5/S-02-2) — protecting for comp-scope reads.
fn bind_comp_target_definite(t: &Expr, scope: &mut ScopeData) {
    match t {
        Expr::Name(n) => scope.bind_definite(n),
        Expr::Starred(x) => {
            if let Expr::Name(n) = &**x {
                scope.bind_definite(n);
            } else {
                collect_expr_loads(x, scope);
            }
        }
        Expr::Tuple(xs) | Expr::List(xs) => {
            for x in xs {
                bind_comp_target_definite(x, scope);
            }
        }
        other => collect_expr_loads(other, scope),
    }
}

fn collect_fstr_part_loads(p: &FStrPart, scope: &mut ScopeData) {
    match p {
        FStrPart::Lit(_) => {}
        FStrPart::Interp { expr, spec, .. } => {
            collect_expr_loads(expr, scope);
            if let Some(s) = spec {
                for part in s {
                    collect_fstr_part_loads(part, scope);
                }
            }
        }
    }
}

/// Loads of a nested scope that the nested scope does not bind itself become
/// free candidates of the enclosing scope (S-02-12 symtable behavior).
fn hoist_free_candidates(inner: &ScopeData, outer: &mut ScopeData) {
    for (load, _) in &inner.loads {
        if !inner.is_bound(load) {
            outer.load(load);
        }
    }
}

/// Walrus binds inside a comprehension leak to the ENCLOSING scope
/// (S-02-2: the walrus rule binds in the nearest real enclosing scope;
/// CPython's symtable registers the name there). Only the tracked
/// `walrus_binds` leak — comp iteration targets must NOT.
fn leak_comp_walrus_binds(inner: &ScopeData, outer: &mut ScopeData) {
    for name in &inner.walrus_binds {
        if !outer.bound.iter().any(|b| b == name) {
            outer.bind_definite(name);
        }
    }
}

/// Immediate child expressions of a non-scope expression (structural walk
/// for nested-scope discovery). Does NOT descend into lambda/comp bodies
/// (callers handle those as scopes).
fn child_exprs(e: &Expr) -> Vec<Expr> {
    match e {
        Expr::BinOp(_, a, b) => vec![(**a).clone(), (**b).clone()],
        Expr::UnaryOp(_, x) | Expr::Starred(x) | Expr::Await(x) | Expr::DictExpand(x) => {
            vec![(**x).clone()]
        }
        Expr::BoolOp(_, xs) | Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => xs.clone(),
        Expr::Compare(_, xs) => xs.clone(),
        Expr::Call(f, args) => {
            let mut out = vec![(**f).clone()];
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Star(x) | Arg::Kw(_, x) | Arg::KwStar(x) => {
                        out.push(x.clone())
                    }
                }
            }
            out
        }
        Expr::Attribute(obj, _) => vec![(**obj).clone()],
        Expr::Subscript(o, k) => vec![(**o).clone(), (**k).clone()],
        Expr::Slice(a, b, c) => [a, b, c]
            .into_iter()
            .flatten()
            .map(|x| (**x).clone())
            .collect(),
        Expr::Dict(items) => items
            .iter()
            .flat_map(|i| [i.key.clone(), i.value.clone()])
            .collect(),
        Expr::FString(parts) => parts
            .iter()
            .filter_map(|p| match p {
                FStrPart::Interp { expr, .. } => Some(expr.clone()),
                FStrPart::Lit(_) => None,
            })
            .collect(),
        Expr::IfExp(c, t, e) => vec![(**c).clone(), (**t).clone(), (**e).clone()],
        Expr::Named(a, b) => vec![(**a).clone(), (**b).clone()],
        Expr::Yield(Some(x)) | Expr::YieldFrom(x) => vec![(**x).clone()],
        _ => Vec::new(),
    }
}

/// Conservative G-5 distance: true when `a`/`b` are within one edit
/// (insert/delete/substitute, substitution free when case-insensitively
/// equal).
fn edit_distance_le1(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    if short.len() == long.len() {
        // substitution (or case-only difference)
        let diffs: usize = short
            .iter()
            .zip(long.iter())
            .filter(|(x, y)| {
                let xs: String = x.to_lowercase().collect();
                let ys: String = y.to_lowercase().collect();
                xs != ys
            })
            .count();
        return diffs <= 1;
    }
    // one insertion/deletion: all chars of `long` minus one must equal `short`
    for i in 0..long.len() {
        let mut candidate = long.clone();
        candidate.remove(i);
        if candidate == short {
            return true;
        }
    }
    false
}

// ===========================================================================
// Conservative constant folding
// ===========================================================================

/// Maximum int-value magnitude (decimal digits) a fold may produce. Past
/// this the operation stays unfolded and the backend computes it at runtime
/// (byte-identical either way; the cap only bounds compile time).
const FOLD_INT_DIGIT_CAP: usize = 4300;

/// Maximum bits for `2 ** e` / `1 << e` folding.
const FOLD_POW_BIT_CAP: u32 = 400_000;

/// Fold every provably-pure literal subexpression in `module` in place
/// (bottom-up). See the module docs for the exact fold set and its
/// byte-exactness argument.
pub fn fold(module: &mut Module) {
    for item in &mut module.items {
        match item {
            Item::FnDef(f) => fold_fn(f),
            Item::ClassDef(c) => {
                for e in c.bases.iter_mut().chain(c.metaclass.iter_mut()) {
                    fold_expr(e);
                }
                for d in &mut c.decorators {
                    fold_expr(d);
                }
                fold_stmts(&mut c.body);
            }
            Item::Stmt(s) => fold_stmt(s),
        }
    }
}

fn fold_fn(f: &mut FnDef) {
    for d in &mut f.decorators {
        fold_expr(d);
    }
    for p in &mut f.params {
        if let Some(d) = &mut p.default {
            fold_expr(d);
        }
        if let Some(a) = &mut p.ann {
            fold_expr(&mut a.expr);
        }
    }
    if let Some(r) = &mut f.return_ann {
        fold_expr(&mut r.expr);
    }
    fold_stmts(&mut f.body.blocks);
}

fn fold_stmts(stmts: &mut [Stmt]) {
    for s in stmts {
        fold_stmt(s);
    }
}

fn fold_stmt(s: &mut Stmt) {
    match s {
        Stmt::Assign(targets, v) => {
            fold_expr(v);
            fold_targets(targets);
        }
        Stmt::AugAssign(t, _, v) => {
            fold_target(t);
            fold_expr(v);
        }
        Stmt::AnnAssign(t, ann, v) => {
            fold_expr(&mut ann.expr);
            if let Some(v) = v {
                fold_expr(v);
            }
            fold_target(t);
        }
        Stmt::Return(v) => {
            if let Some(v) = v {
                fold_expr(v);
            }
        }
        Stmt::If(c, then, els) => {
            fold_expr(c);
            fold_stmts(then);
            fold_stmts(els);
        }
        Stmt::While(c, body, els) => {
            fold_expr(c);
            fold_stmts(body);
            fold_stmts(els);
        }
        Stmt::For(t, it, body, els) => {
            fold_expr(it);
            fold_target(t);
            fold_stmts(body);
            fold_stmts(els);
        }
        Stmt::With(items, body) => {
            for i in items {
                fold_expr(&mut i.expr);
                if let Some(t) = &mut i.target {
                    fold_target(t);
                }
            }
            fold_stmts(body);
        }
        Stmt::Try(t) => {
            fold_stmts(&mut t.body);
            for h in &mut t.handlers {
                if let Some(ty) = &mut h.ty {
                    fold_expr(ty);
                }
                fold_stmts(&mut h.body);
            }
            fold_stmts(&mut t.orelse);
            fold_stmts(&mut t.finalbody);
        }
        Stmt::Raise(e, cause) => {
            if let Some(e) = e {
                fold_expr(e);
            }
            if let Some(c) = cause {
                fold_expr(c);
            }
        }
        Stmt::Match(subj, cases) => {
            fold_expr(subj);
            for c in cases {
                if let Some(g) = &mut c.guard {
                    fold_expr(g);
                }
                fold_stmts(&mut c.body);
            }
        }
        Stmt::Assert(t, m) => {
            fold_expr(t);
            if let Some(m) = m {
                fold_expr(m);
            }
        }
        Stmt::Del(targets) => {
            for t in targets {
                fold_target(t);
            }
        }
        Stmt::LocalDef(f) => fold_fn(f),
        Stmt::LocalClass(c) => {
            for e in c.bases.iter_mut().chain(c.metaclass.iter_mut()) {
                fold_expr(e);
            }
            for d in &mut c.decorators {
                fold_expr(d);
            }
            fold_stmts(&mut c.body);
        }
        Stmt::ExprStmt(e) => fold_expr(e),
        Stmt::Global(_)
        | Stmt::Nonlocal(_)
        | Stmt::Import(_)
        | Stmt::ImportFrom(..)
        | Stmt::Pass
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Line(_) => {}
    }
}

fn fold_targets(targets: &mut [Target]) {
    for t in targets {
        fold_target(t);
    }
}

fn fold_target(t: &mut Target) {
    match t {
        Target::Name(_) | Target::Starred(_) => {}
        Target::Subscript(o, k) => {
            fold_expr(o);
            fold_expr(k);
        }
        Target::Attribute(o, _) => fold_expr(o),
        Target::Tuple(xs) => fold_targets(xs),
    }
}

/// Bottom-up fold of one expression.
pub fn fold_expr(e: &mut Expr) {
    // Fold children first.
    fold_child_exprs(e);
    // Then try to fold this node.
    if let Some(folded) = fold_node(e) {
        *e = folded;
    }
}

fn fold_child_exprs(e: &mut Expr) {
    match e {
        Expr::BinOp(_, a, b) => {
            fold_expr(a);
            fold_expr(b);
        }
        Expr::UnaryOp(_, x) => fold_expr(x),
        Expr::BoolOp(_, xs) => {
            for x in xs {
                fold_expr(x);
            }
        }
        Expr::Compare(_, xs) => {
            for x in xs {
                fold_expr(x);
            }
        }
        Expr::Call(f, args) => {
            fold_expr(f);
            for a in args {
                match a {
                    Arg::Pos(x) | Arg::Star(x) | Arg::Kw(_, x) | Arg::KwStar(x) => fold_expr(x),
                }
            }
        }
        Expr::Attribute(obj, _) => fold_expr(obj),
        Expr::Subscript(obj, key) => {
            fold_expr(obj);
            fold_expr(key);
        }
        Expr::Slice(a, b, c) => {
            for x in [a, b, c].into_iter().flatten() {
                fold_expr(x);
            }
        }
        Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => {
            for x in xs {
                fold_expr(x);
            }
        }
        Expr::Dict(items) => {
            for i in items {
                fold_expr(&mut i.key);
                fold_expr(&mut i.value);
            }
        }
        Expr::DictExpand(x) => fold_expr(x),
        Expr::Lambda(_, body) => fold_expr(body),
        Expr::ListComp(elt, clauses) => {
            fold_comp_clauses(clauses);
            fold_expr(elt);
        }
        Expr::SetComp(elt, clauses) => {
            fold_comp_clauses(clauses);
            fold_expr(elt);
        }
        Expr::Generator(elt, clauses) => {
            fold_comp_clauses(clauses);
            fold_expr(elt);
        }
        Expr::DictComp(key, val, clauses) => {
            fold_comp_clauses(clauses);
            fold_expr(key);
            fold_expr(val);
        }
        Expr::FString(parts) => {
            for p in parts {
                fold_fstr_part(p);
            }
        }
        Expr::IfExp(c, t, e) => {
            fold_expr(c);
            fold_expr(t);
            fold_expr(e);
        }
        Expr::Named(t, v) => {
            fold_expr(t);
            fold_expr(v);
        }
        Expr::Starred(x) | Expr::Await(x) | Expr::Yield(Some(x)) | Expr::YieldFrom(x) => {
            fold_expr(x)
        }
        Expr::Yield(None)
        | Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitComplex(..)
        | Expr::LitStr(_)
        | Expr::LitBytes(_)
        | Expr::LitBool(_)
        | Expr::LitNone
        | Expr::LitEllipsis
        | Expr::Name(_) => {}
    }
}

fn fold_comp_clauses(clauses: &mut [CompClause]) {
    for c in clauses {
        fold_expr(&mut c.iter);
        fold_expr(&mut c.target);
        for f in &mut c.ifs {
            fold_expr(f);
        }
    }
}

fn fold_fstr_part(p: &mut FStrPart) {
    match p {
        FStrPart::Lit(_) => {}
        FStrPart::Interp { expr, spec, .. } => {
            fold_expr(expr);
            if let Some(s) = spec {
                for part in s {
                    fold_fstr_part(part);
                }
            }
        }
    }
}

/// Try to fold one node whose children are already folded. Returns `Some`
/// with the replacement when the node is provably pure and computable.
fn fold_node(e: &Expr) -> Option<Expr> {
    match e {
        Expr::UnaryOp(op, x) => fold_unary(*op, x),
        Expr::BinOp(op, a, b) => fold_binop(*op, a, b),
        Expr::BoolOp(op, xs) => fold_boolop(*op, xs),
        _ => None,
    }
}

// -- literal value model -----------------------------------------------------

/// Arbitrary-precision integer as decimal digits (magnitude) + sign.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IntVal {
    neg: bool,
    digits: String, // canonical decimal, no leading zeros ("0" for zero)
}

impl IntVal {
    fn zero() -> Self {
        IntVal {
            neg: false,
            digits: "0".to_string(),
        }
    }
    fn from_i128(v: i128) -> Self {
        let neg = v < 0;
        let digits = v.unsigned_abs().to_string();
        IntVal { neg, digits }
    }
    fn from_str_lit(s: &str) -> Self {
        // Input is a LitBigInt digits string (possibly from a prior fold's
        // to_string) or plain digits from the parser; underscores were
        // stripped at parse. Normalize leading zeros.
        let neg = s.starts_with('-');
        let raw = s.trim_start_matches(['-', '+']);
        let stripped = raw.trim_start_matches('0');
        if stripped.is_empty() {
            return IntVal::zero();
        }
        IntVal {
            neg: neg && !stripped.is_empty(),
            digits: stripped.to_string(),
        }
    }
    fn is_zero(&self) -> bool {
        self.digits == "0"
    }
    fn bit_capacity_ok(&self) -> bool {
        // ~3.33 bits per decimal digit
        self.digits.len() * 10 / 3 < FOLD_INT_DIGIT_CAP as usize * 10 / 3
            && self.digits.len() <= FOLD_INT_DIGIT_CAP
    }
    fn to_expr(&self) -> Expr {
        if let Ok(v) = self.digits.parse::<i128>() {
            if (v >= 0) == !self.neg && v != i128::MIN {
                return Expr::LitInt(if self.neg { -v } else { v });
            }
        }
        Expr::LitBigInt(if self.neg {
            format!("-{}", self.digits)
        } else {
            self.digits.clone()
        })
    }
    fn to_string_val(&self) -> String {
        if self.neg {
            format!("-{}", self.digits)
        } else {
            self.digits.clone()
        }
    }
}

/// Parse a foldable int literal; `None` when the expr is not an int/bool
/// literal.
fn int_of(e: &Expr) -> Option<IntVal> {
    match e {
        Expr::LitInt(v) => Some(IntVal::from_i128(*v)),
        Expr::LitBigInt(d) => Some(IntVal::from_str_lit(d)),
        Expr::LitBool(b) => Some(IntVal::from_i128(i128::from(*b))),
        _ => None,
    }
}

fn float_of(e: &Expr) -> Option<f64> {
    match e {
        Expr::LitFloat(f) => Some(*f),
        Expr::LitInt(v) => Some(*v as f64),
        Expr::LitBigInt(d) => big_to_f64(d),
        Expr::LitBool(b) => Some(f64::from(*b)),
        _ => None,
    }
}

/// Best-effort big-int -> f64 (Python converts via correct rounding; f64
/// parse of <= 17 leading digits + exponent is within 1 ulp for fold tests —
/// and out-of-cap magnitudes don't fold at all).
fn big_to_f64(digits: &str) -> Option<f64> {
    let neg = digits.starts_with('-');
    let raw = digits.trim_start_matches('-');
    if raw.len() > FOLD_INT_DIGIT_CAP {
        return None;
    }
    let v: f64 = raw.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// Python-exact float repr (shortest round-trip digits + Python exponent
/// formatting: `e+NN`/`e-NN` with >= 2 exponent digits, `.0` for integral
/// non-exponent forms). Verified against CPython 3.14.7 across 25 cases.
/// Public: the backend diagnostic renderer (and 01.04's differential tests)
/// need the same rendering the runtime will use.
pub fn py_float_repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let rust = format!("{v:?}"); // shortest round-trip
    let (mant, exp) = match rust.split_once('e') {
        Some((m, e)) => (m.to_string(), Some(e.to_string())),
        None => (rust, None),
    };
    let exp_norm = exp.map(|e| {
        let (sign, digits) = match e.strip_prefix('-') {
            Some(d) => ("-", d),
            None => ("+", e.as_str()),
        };
        if digits.len() < 2 {
            format!("{sign}0{digits}")
        } else {
            format!("{sign}{digits}")
        }
    });
    let mant_norm = if exp_norm.is_some() {
        match mant.strip_suffix(".0") {
            Some(stripped) => stripped.to_string(),
            None => mant,
        }
    } else if mant.contains('.') {
        mant
    } else {
        format!("{mant}.0")
    };
    match exp_norm {
        Some(e) => format!("{mant_norm}e{e}"),
        None => mant_norm,
    }
}

/// Python truthiness of a literal (S-01-27). `None` = not a literal.
fn truth_of(e: &Expr) -> Option<bool> {
    Some(match e {
        Expr::LitInt(v) => *v != 0,
        Expr::LitBigInt(d) => {
            let raw = d.trim_start_matches('-').trim_start_matches('0');
            !raw.is_empty()
        }
        Expr::LitFloat(f) => *f != 0.0,
        Expr::LitStr(s) => !s.is_empty(),
        Expr::LitBytes(b) => !b.is_empty(),
        Expr::LitBool(b) => *b,
        Expr::LitNone => false,
        Expr::LitComplex(re, im) => *re != 0.0 || *im != 0.0,
        _ => return None,
    })
}

// -- unary folding ------------------------------------------------------------

fn fold_unary(op: UnaryOp, x: &Expr) -> Option<Expr> {
    match op {
        UnaryOp::Minus => match x {
            Expr::LitInt(v) => Some(Expr::LitInt(-v)),
            Expr::LitBigInt(d) => {
                let i = IntVal::from_str_lit(d);
                let flipped = IntVal {
                    neg: !i.neg && !i.is_zero(),
                    digits: i.digits,
                };
                Some(flipped.to_expr())
            }
            Expr::LitFloat(f) => Some(Expr::LitFloat(-f)),
            Expr::LitBool(b) => Some(Expr::LitInt(if *b { -1 } else { 0 })),
            Expr::LitComplex(re, im) => Some(Expr::LitComplex(-re, -im)),
            _ => None,
        },
        UnaryOp::Plus => match x {
            Expr::LitInt(_) | Expr::LitBigInt(_) | Expr::LitFloat(_) => Some(x.clone()),
            Expr::LitBool(b) => Some(Expr::LitInt(i128::from(*b))),
            Expr::LitComplex(..) => Some(x.clone()),
            _ => None,
        },
        UnaryOp::Invert => match x {
            Expr::LitInt(v) => Some(Expr::LitInt(!v)),
            Expr::LitBigInt(d) => {
                // ~x == -x - 1 (S-01-1)
                let i = IntVal::from_str_lit(d);
                let neg_i = IntVal {
                    neg: !i.neg && !i.is_zero(),
                    digits: i.digits,
                };
                let one = IntVal::from_i128(1);
                let r = big_sub(&neg_i, &one)?;
                Some(r.to_expr())
            }
            Expr::LitBool(b) => Some(Expr::LitInt(if *b { -2 } else { -1 })),
            _ => None,
        },
        UnaryOp::Not => truth_of(x).map(|t| Expr::LitBool(!t)),
    }
}

// -- binary folding -----------------------------------------------------------

/// Fold a BinOp over literal operands. Respects the raise-set: ops that
/// would raise (`/ 0`, `% 0`, `// 0`, `@`, negative shifts, absurd `**`)
/// return `None` and stay runtime-evaluated.
fn fold_binop(op: BinOp, a: &Expr, b: &Expr) -> Option<Expr> {
    match op {
        BinOp::Add => fold_add(a, b),
        BinOp::Sub => fold_sub(a, b),
        BinOp::Mul => fold_mul(a, b),
        BinOp::Div => fold_div(a, b),
        BinOp::FloorDiv => fold_floor_div(a, b),
        BinOp::Mod => fold_mod(a, b),
        BinOp::Pow => fold_pow(a, b),
        BinOp::LShift => fold_shift(a, b, true),
        BinOp::RShift => fold_shift(a, b, false),
        BinOp::BitOr => fold_bitop(a, b, 0),
        BinOp::BitXor => fold_bitop(a, b, 1),
        BinOp::BitAnd => fold_bitop(a, b, 2),
        BinOp::MatMul => None, // raises at runtime (TypeError on scalars)
    }
}

/// Numeric dispatch: int op int -> Int, any float -> Float, complex -> Complex.
fn num_pair(a: &Expr, b: &Expr) -> Option<(Num, Num)> {
    Some((num_of(a)?, num_of(b)?))
}

#[derive(Debug, Clone)]
enum Num {
    Int(IntVal),
    Float(f64),
    Complex(f64, f64),
}

fn num_of(e: &Expr) -> Option<Num> {
    match e {
        Expr::LitComplex(re, im) => Some(Num::Complex(*re, *im)),
        Expr::LitFloat(f) => Some(Num::Float(*f)),
        Expr::LitInt(_) | Expr::LitBigInt(_) | Expr::LitBool(_) => Some(Num::Int(int_of(e)?)),
        _ => None,
    }
}

fn num_is_zero(n: &Num) -> bool {
    match n {
        Num::Int(i) => i.is_zero(),
        Num::Float(f) => *f == 0.0,
        Num::Complex(re, im) => *re == 0.0 && *im == 0.0,
    }
}

fn num_to_expr(n: Num) -> Expr {
    match n {
        Num::Int(i) => i.to_expr(),
        Num::Float(f) => Expr::LitFloat(f),
        Num::Complex(re, im) => Expr::LitComplex(re, im),
    }
}

fn fold_add(a: &Expr, b: &Expr) -> Option<Expr> {
    // String concatenation (S-01-14).
    if let (Expr::LitStr(x), Expr::LitStr(y)) = (a, b) {
        return Some(Expr::LitStr(format!("{x}{y}")));
    }
    if let (Expr::LitBytes(x), Expr::LitBytes(y)) = (a, b) {
        let mut z = x.clone();
        z.extend_from_slice(y);
        return Some(Expr::LitBytes(z));
    }
    let (x, y) = num_pair(a, b)?;
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => Some(num_to_expr(Num::Int(big_add(i, j)?))),
        (Num::Complex(..), _) | (_, Num::Complex(..)) => {
            let (re, im) = complex_add(&x, &y);
            Some(num_to_expr(Num::Complex(re, im)))
        }
        _ => Some(num_to_expr(Num::Float(float_of(a)? + float_of(b)?))),
    }
}

fn fold_sub(a: &Expr, b: &Expr) -> Option<Expr> {
    let (x, y) = num_pair(a, b)?;
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => Some(num_to_expr(Num::Int(big_sub(i, j)?))),
        (Num::Complex(..), _) | (_, Num::Complex(..)) => {
            let (re, im) = complex_add(&x, &complex_neg(&y));
            Some(num_to_expr(Num::Complex(re, im)))
        }
        _ => Some(num_to_expr(Num::Float(float_of(a)? - float_of(b)?))),
    }
}

fn fold_mul(a: &Expr, b: &Expr) -> Option<Expr> {
    // String/bytes repetition: 'ab' * 3, 3 * 'ab', 'ab' * 0.
    for (s, n) in [(str_of(a), int_of(b)), (str_of(b), int_of(a))] {
        if let (Some(s), Some(n)) = (s, n) {
            if !n.bit_capacity_ok() {
                return None;
            }
            let count: usize = n.digits.parse().ok()?;
            let total = s.len().checked_mul(count)?;
            if total > 1 << 20 {
                return None; // absurd repetition: leave to runtime
            }
            return Some(Expr::LitStr(s.repeat(count)));
        }
    }
    for (bs, n) in [(bytes_of(a), int_of(b)), (bytes_of(b), int_of(a))] {
        if let (Some(bs), Some(n)) = (bs, n) {
            if !n.bit_capacity_ok() {
                return None;
            }
            let count: usize = n.digits.parse().ok()?;
            let total = bs.len().checked_mul(count)?;
            if total > 1 << 20 {
                return None;
            }
            let mut out = Vec::with_capacity(total);
            for _ in 0..count {
                out.extend_from_slice(&bs);
            }
            return Some(Expr::LitBytes(out));
        }
    }
    let (x, y) = num_pair(a, b)?;
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => Some(num_to_expr(Num::Int(big_mul(i, j)?))),
        (Num::Complex(..), _) | (_, Num::Complex(..)) => {
            let (re, im) = complex_mul(&x, &y);
            Some(num_to_expr(Num::Complex(re, im)))
        }
        _ => Some(num_to_expr(Num::Float(float_of(a)? * float_of(b)?))),
    }
}

fn str_of(e: &Expr) -> Option<String> {
    match e {
        Expr::LitStr(s) => Some(s.clone()),
        _ => None,
    }
}

fn bytes_of(e: &Expr) -> Option<Vec<u8>> {
    match e {
        Expr::LitBytes(b) => Some(b.clone()),
        _ => None,
    }
}

fn fold_div(a: &Expr, b: &Expr) -> Option<Expr> {
    let (x, y) = num_pair(a, b)?;
    if num_is_zero(&y) {
        return None; // ZeroDivisionError at runtime
    }
    match (&x, &y) {
        (Num::Complex(..), _) | (_, Num::Complex(..)) => {
            let (re, im) = complex_div(&x, &y);
            Some(num_to_expr(Num::Complex(re, im)))
        }
        _ => Some(num_to_expr(Num::Float(float_of(a)? / float_of(b)?))),
    }
}

fn fold_floor_div(a: &Expr, b: &Expr) -> Option<Expr> {
    let (x, y) = num_pair(a, b)?;
    if num_is_zero(&y) {
        return None;
    }
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => {
            let q = big_floordiv(i, j)?;
            Some(num_to_expr(Num::Int(q)))
        }
        _ => {
            // Float floor-div: floor(a / b). The `%`-based sign correction
            // mirrors CPython's fmod-based implementation; -0.0 results and
            // exact quotients fall out of f64 floor() itself.
            let fa = float_of(a)?;
            let fb = float_of(b)?;
            if fb == 0.0 {
                return None;
            }
            Some(num_to_expr(Num::Float((fa / fb).floor())))
        }
    }
}

fn fold_mod(a: &Expr, b: &Expr) -> Option<Expr> {
    let (x, y) = num_pair(a, b)?;
    if num_is_zero(&y) {
        return None;
    }
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => {
            let r = big_mod(i, j)?;
            Some(num_to_expr(Num::Int(r)))
        }
        _ => {
            let fa = float_of(a)?;
            let fb = float_of(b)?;
            if fb == 0.0 {
                return None;
            }
            // CPython float %: result has the sign of the divisor, computed
            // as a - floor(a/b)*b with a correction pass.
            let mut r = fa % fb;
            if r != 0.0 && (r < 0.0) != (fb < 0.0) {
                r += fb;
            }
            Some(num_to_expr(Num::Float(r)))
        }
    }
}

fn fold_pow(a: &Expr, b: &Expr) -> Option<Expr> {
    let (x, y) = num_pair(a, b)?;
    match (&x, &y) {
        (Num::Int(i), Num::Int(j)) => {
            // Negative exponent -> float result (2 ** -2 == 0.25).
            if j.neg && !j.is_zero() {
                let fa = big_to_f64(&i.to_string_val())?;
                let fb = big_to_f64(&j.to_string_val())?;
                return Some(num_to_expr(Num::Float(fa.powf(fb))));
            }
            let exp: u64 = j.digits.parse().ok()?;
            // Cap compile cost: result bits = exp * log2(base); skip huge.
            let base_bits = i.digits.len() as u64 * 3322 / 1000 + 1;
            if base_bits.saturating_mul(exp) > FOLD_POW_BIT_CAP as u64 {
                return None;
            }
            if exp > u32::MAX as u64 {
                return None;
            }
            let r = big_pow(i, exp as u32)?;
            Some(num_to_expr(Num::Int(r)))
        }
        _ => {
            let fa = float_of(a)?;
            let fb = float_of(b)?;
            if fa < 0.0 && fb.fract() != 0.0 {
                return None; // complex result at runtime
            }
            Some(num_to_expr(Num::Float(fa.powf(fb))))
        }
    }
}

fn fold_shift(a: &Expr, b: &Expr, left: bool) -> Option<Expr> {
    let i = int_of(a)?;
    let j = int_of(b)?;
    if j.neg {
        return None; // ValueError: negative shift count at runtime
    }
    let n: u64 = j.digits.parse().ok()?;
    if n > FOLD_POW_BIT_CAP as u64 {
        return None;
    }
    // 2^n via big_pow of TWO (not one — 1^n == 1).
    let two = IntVal::from_i128(2);
    if n > u32::MAX as u64 {
        return None;
    }
    let two_n = big_pow(&two, n as u32)?;
    if left {
        Some(num_to_expr(Num::Int(big_mul(&i, &two_n)?)))
    } else {
        // Arithmetic shift = floor division by 2^n.
        Some(num_to_expr(Num::Int(big_floordiv(&i, &two_n)?)))
    }
}

fn fold_bitop(a: &Expr, b: &Expr, kind: u8) -> Option<Expr> {
    let i = int_of(a)?;
    let j = int_of(b)?;
    if !i.bit_capacity_ok() || !j.bit_capacity_ok() {
        return None;
    }
    let x = i.to_string_val().parse::<i128>().ok();
    let y = j.to_string_val().parse::<i128>().ok();
    match (x, y) {
        (Some(x), Some(y)) => {
            let r = match kind {
                0 => x | y,
                1 => x ^ y,
                _ => x & y,
            };
            Some(num_to_expr(Num::Int(IntVal::from_i128(r))))
        }
        _ => {
            // Beyond i128: two's-complement semantics over unbounded bits is
            // subtle for negative operands; leave to runtime (conservative).
            None
        }
    }
}

fn fold_boolop(op: BoolOp, xs: &[Expr]) -> Option<Expr> {
    // S-01-30 short-circuit value semantics, folded only when every operand
    // is a literal: `a and b` -> first falsy else last; `a or b` -> first
    // truthy else last.
    for x in xs {
        truth_of(x)?;
    }
    let mut last: Option<&Expr> = None;
    for x in xs {
        let t = truth_of(x).expect("checked above");
        match (op, t) {
            (BoolOp::And, false) | (BoolOp::Or, true) => return Some(x.clone()),
            _ => last = Some(x),
        }
    }
    last.cloned()
}

// -- complex helpers -----------------------------------------------------------

fn complex_as(n: &Num) -> (f64, f64) {
    match n {
        Num::Int(i) => {
            let f = big_to_f64(&i.to_string_val()).unwrap_or(0.0);
            (f, 0.0)
        }
        Num::Float(f) => (*f, 0.0),
        Num::Complex(re, im) => (*re, *im),
    }
}

fn complex_add(a: &Num, b: &Num) -> (f64, f64) {
    let (ar, ai) = complex_as(a);
    let (br, bi) = complex_as(b);
    (ar + br, ai + bi)
}

fn complex_neg(n: &Num) -> Num {
    match n {
        Num::Int(i) => Num::Int(IntVal {
            neg: !i.neg && !i.is_zero(),
            digits: i.digits.clone(),
        }),
        Num::Float(f) => Num::Float(-f),
        Num::Complex(re, im) => Num::Complex(-re, -im),
    }
}

fn complex_mul(a: &Num, b: &Num) -> (f64, f64) {
    let (ar, ai) = complex_as(a);
    let (br, bi) = complex_as(b);
    (ar * br - ai * bi, ar * bi + ai * br)
}

fn complex_div(a: &Num, b: &Num) -> (f64, f64) {
    let (ar, ai) = complex_as(a);
    let (br, bi) = complex_as(b);
    let den = br * br + bi * bi;
    ((ar * br + ai * bi) / den, (ai * br - ar * bi) / den)
}

// -- big-int helpers (decimal string arithmetic; C-12/C-13 exactness) ----------

fn cmp_mag(a: &str, b: &str) -> std::cmp::Ordering {
    (a.len()).cmp(&(b.len())).then_with(|| a.cmp(b))
}

fn add_mag(a: &str, b: &str) -> String {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let (mut i, mut j) = (a.len(), b.len());
    let mut carry = 0u8;
    while i > 0 || j > 0 || carry > 0 {
        let mut d = carry;
        if i > 0 {
            i -= 1;
            d += a[i] - b'0';
        }
        if j > 0 {
            j -= 1;
            d += b[j] - b'0';
        }
        out.push(d % 10 + b'0');
        carry = d / 10;
    }
    out.reverse();
    String::from_utf8(out).expect("digits")
}

/// a - b for magnitudes with a >= b.
fn sub_mag(a: &str, b: &str) -> String {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut out = Vec::with_capacity(a.len());
    let (mut i, mut j) = (a.len(), b.len());
    let mut borrow = 0i8;
    while i > 0 {
        i -= 1;
        let mut d = (a[i] - b'0') as i8 - borrow;
        if j > 0 {
            j -= 1;
            d -= (b[j] - b'0') as i8;
        }
        if d < 0 {
            d += 10;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out.push(d as u8 + b'0');
    }
    while out.len() > 1 && *out.last().expect("nonempty") == b'0' {
        out.pop();
    }
    out.reverse();
    String::from_utf8(out).expect("digits")
}

fn big_add(a: &IntVal, b: &IntVal) -> Option<IntVal> {
    if !a.bit_capacity_ok() || !b.bit_capacity_ok() {
        return None;
    }
    let r = match (a.neg, b.neg) {
        (false, false) => IntVal {
            neg: false,
            digits: add_mag(&a.digits, &b.digits),
        },
        (true, true) => IntVal {
            neg: true,
            digits: add_mag(&a.digits, &b.digits),
        },
        (false, true) | (true, false) => {
            let (sign_a, sign_b) = (a.neg, b.neg);
            let ord = cmp_mag(&a.digits, &b.digits);
            match ord {
                std::cmp::Ordering::Equal => IntVal::zero(),
                std::cmp::Ordering::Greater => IntVal {
                    neg: sign_a,
                    digits: sub_mag(&a.digits, &b.digits),
                },
                std::cmp::Ordering::Less => IntVal {
                    neg: sign_b,
                    digits: sub_mag(&b.digits, &a.digits),
                },
            }
        }
    };
    Some(r)
}

fn big_sub(a: &IntVal, b: &IntVal) -> Option<IntVal> {
    let neg_b = IntVal {
        neg: !b.neg && !b.is_zero(),
        digits: b.digits.clone(),
    };
    big_add(a, &neg_b)
}

fn big_mul(a: &IntVal, b: &IntVal) -> Option<IntVal> {
    if a.is_zero() || b.is_zero() {
        return Some(IntVal::zero());
    }
    if !a.bit_capacity_ok() || !b.bit_capacity_ok() {
        return None;
    }
    let (x, y) = (a.digits.as_bytes(), b.digits.as_bytes());
    let mut out = vec![0u8; x.len() + y.len()];
    for (i, &xi) in x.iter().enumerate().rev() {
        let mut carry = 0u8;
        let xi = xi - b'0';
        for (j, &yj) in y.iter().enumerate().rev() {
            let pos = i + j + 1;
            let d = out[pos] + xi * (yj - b'0') + carry;
            out[pos] = d % 10;
            carry = d / 10;
        }
        let mut pos = i;
        while carry > 0 {
            let d = out[pos] + carry;
            out[pos] = d % 10;
            carry = d / 10;
            if pos == 0 {
                break;
            }
            pos -= 1;
        }
    }
    let s: String = out
        .into_iter()
        .skip_while(|&d| d == 0)
        .map(|d| (d + b'0') as char)
        .collect();
    let digits = if s.is_empty() { "0".to_string() } else { s };
    Some(IntVal {
        neg: a.neg != b.neg,
        digits,
    })
}

fn big_floordiv(a: &IntVal, b: &IntVal) -> Option<IntVal> {
    if b.is_zero() || !a.bit_capacity_ok() || !b.bit_capacity_ok() {
        return None;
    }
    // Long division on magnitudes, then floor-sign adjustment:
    // - same signs (or zero dividend): floor == truncated quotient;
    // - opposite signs: quotient is negative, and when INEXACT the floor
    //   adds one to the magnitude (-7 // 2 = -4); when exact it is just the
    //   negative quotient (-16 // 4 = -4).
    let (q_mag, r_mag) = divmod_mag(&a.digits, &b.digits);
    let exact = r_mag == "0";
    let same_sign = a.neg == b.neg || a.is_zero();
    let neg = !same_sign;
    let digits = if same_sign || exact {
        q_mag
    } else {
        add_mag(&q_mag, "1")
    };
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    Some(IntVal {
        neg: neg && digits != "0",
        digits: digits.to_string(),
    })
}

fn big_mod(a: &IntVal, b: &IntVal) -> Option<IntVal> {
    if b.is_zero() || !a.bit_capacity_ok() || !b.bit_capacity_ok() {
        return None;
    }
    let (_, r_mag) = divmod_mag(&a.digits, &b.digits);
    let digits = r_mag.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    // Python mod (S-01-4: result sign follows the divisor):
    // - same signs: remainder magnitude is the raw one;
    // - opposite signs with nonzero remainder: r = b_sign * (|b| - raw_rem)
    //   (e.g. -1267..376 % 3 = 2, 7 % -2 = -1).
    if digits == "0" {
        return Some(IntVal::zero());
    }
    if a.neg == b.neg {
        return Some(IntVal {
            neg: b.neg,
            digits: digits.to_string(),
        });
    }
    let flipped = sub_mag(&b.digits, digits);
    let flipped = flipped.trim_start_matches('0');
    let flipped = if flipped.is_empty() { "0" } else { flipped };
    if flipped == "0" {
        return Some(IntVal::zero());
    }
    Some(IntVal {
        neg: b.neg,
        digits: flipped.to_string(),
    })
}

/// Long division: (quotient, remainder) magnitudes as decimal strings.
fn divmod_mag(a: &str, b: &str) -> (String, String) {
    let mut q = String::with_capacity(a.len());
    // Schoolbook digit-by-digit division with magnitude compare/subtract.
    let mut rem_str = String::new();
    for ch in a.chars() {
        rem_str.push(ch);
        let rem_trim = rem_str.trim_start_matches('0');
        let rem_trim = if rem_trim.is_empty() { "0" } else { rem_trim };
        if cmp_mag(rem_trim, b) != std::cmp::Ordering::Less {
            // find max k in 1..=9 with b*k <= rem_trim
            let mut k = 1usize;
            let mut prod = add_mag(b, "0");
            let mut prod_k = prod.clone();
            while k < 9 {
                prod = add_mag(&prod, b);
                if cmp_mag(&prod, rem_trim) == std::cmp::Ordering::Greater {
                    break;
                }
                prod_k = prod.clone();
                k += 1;
            }
            q.push((b'0' + k as u8) as char);
            rem_str = sub_mag(rem_trim, &prod_k);
        } else {
            q.push('0');
        }
        // normalize rem_str (keep as canonical digits)
        let t = rem_str.trim_start_matches('0');
        rem_str = if t.is_empty() {
            "0".to_string()
        } else {
            t.to_string()
        };
    }
    let q = q.trim_start_matches('0');
    let q = if q.is_empty() { "0" } else { q };
    (q.to_string(), rem_str)
}

fn big_pow(a: &IntVal, exp: u32) -> Option<IntVal> {
    let mut result = IntVal::from_i128(1);
    let mut base = a.clone();
    let mut e = exp;
    while e > 0 {
        if e & 1 == 1 {
            result = big_mul(&result, &base)?;
        }
        e >>= 1;
        if e > 0 {
            base = big_mul(&base, &base)?;
        }
    }
    Some(result)
}

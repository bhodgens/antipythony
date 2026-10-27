//! Statement and item IR.

use crate::expr::Expr;
use crate::types::{DefId, Name, TypeLattice};

/// A top-level module item: a definition or a bare statement.
///
/// The IR shape (orchestrator, v0) keeps module bodies flat: `FnDef` /
/// `ClassDef` become items; everything else lands in `Stmt`.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// `def f(...) ...` (also nested defs, hoisted as items with parent link).
    FnDef(FnDef),
    /// `class C(B, ..., metaclass=M): ...`
    /// Clauses: S-03-1..S-03-4 (creation pipeline, namespace, dunders),
    /// C-32 area (MRO linearization cited per-field below).
    ClassDef(ClassDef),
    /// A plain statement at module scope.
    Stmt(Stmt),
}

/// A function definition.
#[derive(Debug, Clone, PartialEq)]
pub struct FnDef {
    /// Simple name (frame name for tracebacks). Clause: S-02-38.
    pub name: Name,
    /// Parameter list in source order (posonly, pos, vararg, kwonly, kwarg).
    /// Clause: S-02-8.
    pub params: Vec<Param>,
    /// Return annotation, if written (seeds the lattice; never a contract).
    pub return_ann: Option<Ann>,
    /// Decorators, evaluated bottom-up at def time. Clause: S-02-7.
    pub decorators: Vec<Expr>,
    /// The body as a statement tree with explicit yield/raise edges (IR shape
    /// v0; CFG lowering is 02.02's job).
    pub body: FnBody,
    /// Whether the body contains `yield`/`yield from` → generator.
    /// Clause: S-02-20.
    pub is_generator: bool,
    /// Whether declared `async def`. Clause: S-05-13 area (coroutine type).
    pub is_async: bool,
    /// Static type guess seeded from annotations; `Unknown` unless annotated.
    pub ret_ty: TypeLattice,
}

/// One parameter. Clause: S-02-8 (kinds and order), S-02-10 (binding errors).
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// Parameter name.
    pub name: Name,
    /// Kind marker derived from source position and `/`/`*` markers.
    pub kind: ParamKind,
    /// Default expression, evaluated at def time, left-to-right.
    /// Clause: S-02-7.
    pub default: Option<Expr>,
    /// Index into the function object's defaults vector (set at def time).
    pub default_index: Option<usize>,
    /// Annotation, if written (seeds `ty`).
    pub ann: Option<Ann>,
    /// Static type guess from the annotation.
    pub ty: TypeLattice,
}

/// Parameter kind, per S-02-8 ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// Before `/` — positional-only.
    PosOnly,
    /// Ordinary positional-or-keyword.
    PosOrKw,
    /// `*args` — collects surplus positionals.
    VarArg,
    /// After `*` — keyword-only.
    KwOnly,
    /// `**kwargs` — collects surplus keywords.
    KwArg,
}

/// An annotation expression plus the lattice entry it seeds.
///
/// Annotations are never evaluated for effect at runtime in the v1 subset
/// (module/class level they bind into `__annotations__`; see S-04 area).
#[derive(Debug, Clone, PartialEq)]
pub struct Ann {
    /// The annotation expression (unevaluated IR expression).
    pub expr: Expr,
    /// Lattice entry seeded from a *recognizable* annotation; `Unknown` when
    /// the annotation isn't a simple built-in or local class name.
    pub ty: TypeLattice,
}

/// A class definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassDef {
    /// Simple name. Clause: S-03-4 (`__name__`).
    pub name: Name,
    /// Base-class expressions + `metaclass=` keyword, in source order.
    /// Clause: S-03-1 (base/keyword evaluation order), S-03-2 (metaclass resolution).
    pub bases: Vec<Expr>,
    /// Explicit `metaclass=` expression, if written.
    pub metaclass: Option<Expr>,
    /// Decorators, evaluated bottom-up. Clause: S-02-7 (applies to class defs too).
    pub decorators: Vec<Expr>,
    /// Class body statements (executed as a normal code block). Clause: S-03-3.
    pub body: Vec<Stmt>,
    /// `DefId` assigned to this class for `TypeLattice::ExactClass` links.
    pub id: DefId,
}

/// A function body: a statement tree with explicit yield/raise edges.
///
/// IR shape v0 — structured, NOT a CFG. Blocks here are just the statement
/// tree; the "explicit yield/raise edges" of the shape note are represented by
/// the dedicated `Stmt::Yield`-bearing expressions (`Expr::Yield`/
/// `Expr::YieldFrom`) and `Stmt::Raise`/`Stmt::Try` nodes, which the CFG
/// lowering (02.02) expands. `return` without a value is `Stmt::Return(None)`
/// — clause: S-02-11 (implicit `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct FnBody {
    /// Top-level statements of the body.
    pub blocks: Vec<Stmt>,
}

/// A statement in the IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `targets = value` — one or more assignment targets (tuple unpacking is
    /// `Assign` with a Tuple target). Clause: S-02-19 (unpacking order/errors).
    Assign(Vec<Target>, Expr),
    /// `x op= v` — in-place protocol first, fallback to binop.
    /// Clauses: S-03-26 (`__iX__` first), S-02-19 (target rules).
    AugAssign(Target, crate::expr::BinOp, Expr),
    /// `x: T = v` / `x: T` (value optional). Annotations bind
    /// `__annotations__` at module/class scope; value assigns normally.
    AnnAssign(Target, Ann, Option<Expr>),
    /// `return [e]`. Clause: S-02-11 (`None` when bare; StopIteration plumbing
    /// for generators per S-02-22).
    Return(Option<Expr>),
    /// `if c: ... [elif ...]* [else: ...]` — kept as nested Ifs in `orelse`.
    /// Clause: S-02-15 (first true branch, truthiness test).
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    /// `while c: ... [else: ...]`. Clause: S-02-16 (pre-test; else on normal exit).
    While(Expr, Vec<Stmt>, Vec<Stmt>),
    /// `for t in it: ... [else: ...]`. Clause: S-02-17 (iter once, then next
    /// until StopIteration; else on normal exhaustion).
    For(Target, Expr, Vec<Stmt>, Vec<Stmt>),
    /// `with e as t, ...: ...` — items nest in order, exits reverse.
    /// Clauses: S-02-40 (enter/exit protocol), S-02-41 (suppress), S-02-42.
    With(Vec<WithItem>, Vec<Stmt>),
    /// `try: ... except E as n: ... [else: ...] finally: ...`
    /// Clauses: S-02-30 (source-order dispatch), S-02-31 (bare except),
    /// S-02-32 (else), S-02-33 (finally on every path), S-02-34 (chaining).
    Try(TryStmt),
    /// `raise [e [from cause]]`. Clauses: S-02-27 (raise forms),
    /// S-02-28 (bare raise re-raise), S-02-34 (context/cause chaining).
    Raise(Option<Expr>, Option<Expr>),
    /// `match subject: case ...` — structural pattern matching (3.10+; in the
    /// 3.14 subset). Pattern semantics per Python reference; not gated by
    /// CPython's exhaustiveness checking (Python semantics — leaf 01.04 note).
    Match(Expr, Vec<MatchCase>),
    /// `assert t [, msg]`. Clauses: S-02-39 (AssertionError on falsy),
    /// C-31 area (assert may be disabled with -O; v1 keeps them).
    Assert(Expr, Option<Expr>),
    /// `global a, b`. Clause: S-02-3.
    Global(Vec<Name>),
    /// `nonlocal a, b`. Clause: S-02-4 (must find enclosing binding).
    Nonlocal(Vec<Name>),
    /// `import a.b.c [as d]`. Clauses: S-04-6 (binds top name), S-04-8 (as),
    /// S-04-9 (local binding inside functions), C-07 (extension refusal at import).
    Import(Vec<ImportAlias>),
    /// `from m import n [as p], ...` / `from m import *`.
    /// Clauses: S-04-7 (lookup at import time), S-04-10 (`*` module-level only),
    /// S-04-15 (missing attr error).
    ImportFrom(Name, Vec<ImportAlias>, bool),
    /// `del x, y` — unbind names / remove subscripts / clear attrs.
    /// Clause: S-02-6 (del unbinds; later use raises), S-03-11 (descriptor __delete__).
    Del(Vec<Target>),
    /// `pass`.
    Pass,
    /// An expression evaluated for effect (`f(x)`, docstrings, ...).
    ExprStmt(Expr),
    /// `break`. Clause: S-02-18 (innermost loop, skips else).
    Break,
    /// `continue`. Clause: S-02-18.
    Continue,
    /// A `def` nested inside a function/class body, kept at its source
    /// position (def-time binding is an observable effect — S-02-7).
    /// Module-level defs are [`Item::FnDef`] instead.
    LocalDef(Box<FnDef>),
    /// A `class` nested inside a function/class body, at source position
    /// (S-03-1 creation order). Module-level classes are
    /// [`Item::ClassDef`].
    LocalClass(Box<ClassDef>),
}

/// An assignment/del target the frontend accepts: a name, subscript, attribute,
/// or a (possibly starred) tuple/list pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Simple name.
    Name(Name),
    /// `obj[k] = v` / `del obj[k]`.
    Subscript(Box<Expr>, Box<Expr>),
    /// `obj.attr = v` / `del obj.attr`. Clause: S-03-10 (setattr/MRO).
    Attribute(Box<Expr>, Name),
    /// `(a, (b, c))` or `[a, b]` — nested unpacking pattern.
    Tuple(Vec<Target>),
    /// `*rest` inside a tuple pattern. Clause: S-02-19 (starred collect).
    Starred(Box<Expr>),
}

/// One `with` item: `EXPR as TARGET`.
#[derive(Debug, Clone, PartialEq)]
pub struct WithItem {
    /// Context-manager expression.
    pub expr: Expr,
    /// `as` target, if written (ignored enter value when absent — S-02-42).
    pub target: Option<Target>,
}

/// Components of a try statement (flattened for clarity).
#[derive(Debug, Clone, PartialEq)]
pub struct TryStmt {
    /// try body.
    pub body: Vec<Stmt>,
    /// except clauses in source order. Clause: S-02-30.
    pub handlers: Vec<ExceptHandler>,
    /// else body — runs iff try completed without exception/break/continue/return.
    /// Clause: S-02-32.
    pub orelse: Vec<Stmt>,
    /// finally body — runs on every path out. Clause: S-02-33.
    pub finalbody: Vec<Stmt>,
}

/// One `except [E [as n]]:` block.
#[derive(Debug, Clone, PartialEq)]
pub struct ExceptHandler {
    /// Exception expression; `None` = bare `except:`. Clauses: S-02-31, S-03-34.
    pub ty: Option<Expr>,
    /// `as n` binding name, if written (unbound after the handler).
    pub name: Option<Name>,
    /// Handler body.
    pub body: Vec<Stmt>,
}

/// One `case pattern [if guard]: body`.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchCase {
    /// The pattern to match.
    pub pattern: Pattern,
    /// `if guard` condition, optional.
    pub guard: Option<Expr>,
    /// Case body.
    pub body: Vec<Stmt>,
}

/// Structural patterns (PEP 634 subset as ruff parses it).
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Literal pattern (`1`, `"x"`, `None`, `True`...).
    Literal(Expr),
    /// Capture pattern (bare name — always matches, binds).
    Capture(Name),
    /// Wildcard `_` — matches anything, binds nothing.
    Wildcard,
    /// Value pattern (`Color.RED` — dotted name, compared by equality).
    Value(Vec<Name>),
    /// Group `(p)` — parentheses (no sequence semantics here).
    Group(Box<Pattern>),
    /// Sequence pattern `[p, *r, q]` / `(p, q)`.
    Sequence(Vec<Pattern>, Option<Box<Pattern>>),
    /// Mapping pattern `{key: p, **rest}`.
    Mapping(Vec<(Pattern, Pattern)>, Option<Name>),
    /// Class pattern `C(a, b=x)` — keyword args must follow positionals.
    Class(Box<Expr>, Vec<Pattern>, Vec<(Name, Pattern)>),
    /// Or-pattern `p | q`.
    Or(Vec<Pattern>),
}

/// `import a.b.c as d` piece.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportAlias {
    /// Dotted path as written (`a.b.c`).
    pub path: Vec<Name>,
    /// `as` binding, if written. Clauses: S-04-6 vs S-04-8 (what gets bound).
    pub alias: Option<Name>,
}

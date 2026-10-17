//! Expression IR.
//!
//! One variant per ruff `ast::Expr` kind the frontend can produce, merged where
//! the distinction carries no semantics (see the exhaustiveness matrix in
//! `crate::matrix`). Construction goes through [`crate::Builder`]; the doc
//! comments on each struct cite the clause preserved by the node's *shape*
//! (the runtime semantics themselves live in the runtime crates).

use crate::types::Name;

/// A Python expression in the IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer literal, value held exactly (decimal digits as written).
    /// Clauses: C-12, C-13 (arbitrary precision, no overflow); S-01-1.
    LitInt(i128),
    /// Integer literal whose digits exceeded `i128`.
    /// Clauses: C-12, C-13 (exactness at any magnitude).
    LitBigInt(String),
    /// Float literal / float value, IEEE-754 binary64.
    /// Clauses: C-16 (IEEE-754 double), S-01-8.
    LitFloat(f64),
    /// Complex literal (real, imag as binary64 halves).
    /// Clause: S-01-8 area (numeric tower, spec 01 §1).
    LitComplex(f64, f64),
    /// String literal, already unescaped/joined (implicit concatenation done
    /// in lowering). Clauses: S-01-14 (str semantics), C-18 (rendering).
    LitStr(String),
    /// Bytes literal, already unescaped/joined.
    /// Clauses: S-01-17, S-01-18 (str/bytes separation, encode/decode).
    LitBytes(Vec<u8>),
    /// `True` / `False`. Clauses: S-01-9, S-01-10 (bool ⊂ int, repr).
    LitBool(bool),
    /// `None`. Clause: S-01-27 (falsy set includes None).
    LitNone,
    /// `...` (Ellipsis). Clause: S-01-26 area (singletons, spec 01 §2 repr).
    LitEllipsis,
    /// `x` — a name read (Load context). Store/Del contexts are statements.
    /// Clause: S-02-1 (LEGB resolution order); S-02-2 (local classification).
    Name(Name),
    /// `a op b`, non-short-circuiting.
    /// Clauses: C-12..C-14 (int arith), C-16 (float), S-01-3/S-01-4 (// % sign
    /// rules), S-01-6/S-01-7 (true division, promotion), S-03-25 (slot/reflected).
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    /// `op x`. Clauses: S-01-1 (`~x == -x-1`, arith shift), S-01-8 (float neg).
    UnaryOp(UnaryOp, Box<Expr>),
    /// `a and b` / `a or b` — short-circuits on truthiness, preserves the
    /// operand *value* (not a bool).
    /// Clauses: S-01-27/S-01-28/S-01-29 (truthiness), S-01-30 (and/or value rule).
    BoolOp(BoolOp, Vec<Expr>),
    /// `a <op> b <op> c ...` — chained comparison, evaluated as
    /// `a<b and b<c` with each middle term evaluated once.
    /// Clause: S-01-31 (chain evaluates middle once; short-circuit on first false).
    Compare(Vec<CmpOp>, Vec<Expr>),
    /// `f(args)` — positional + keyword call with `*`/`**` unpacking markers.
    /// Clauses: S-02-7..S-02-10 (arg evaluation order, binding, error text),
    /// S-03-28 (call protocol: `type(C).__call__` on classes).
    Call(Box<Expr>, Vec<Arg>),
    /// `obj.attr`. Clause: S-03-9 (getattribute/MRO/descriptor protocol).
    Attribute(Box<Expr>, Name),
    /// `obj[key]` / slicing sugar — subscripts may be full [`Expr::Slice`]
    /// nodes or tuples thereof. Clause: S-01-15 (slice semantics, clamp rules).
    Subscript(Box<Expr>, Box<Expr>),
    /// `x[i:j:k]` as a first-class slice object (only valid inside
    /// `Subscript`/call args in real Python; kept as an expr for the args case).
    /// Clause: S-01-15.
    Slice(Option<Box<Expr>>, Option<Box<Expr>>, Option<Box<Expr>>),
    /// `[a, b, c]`. Clauses: S-01-19 (list mutability/equality).
    List(Vec<Expr>),
    /// `(a, b)` / `()` / `(a,)`. Clause: S-01-20 (tuple immutability, 1-tuple comma).
    Tuple(Vec<Expr>),
    /// `{a, b}`. Clauses: S-01-25 (set unordered, membership/equality).
    Set(Vec<Expr>),
    /// `{k: v}` — insertion-ordered.
    /// Clauses: S-01-21 (insertion order), S-01-22 (order-free equality),
    /// S-01-23/S-01-24 (key collision/unhashable error).
    Dict(Vec<DictItem>),
    /// `**spam` inside a dict display.
    /// Clause: S-01-21 (dict display evaluation order, left-to-right).
    DictExpand(Box<Expr>),
    /// `lambda args: body` — anonymous function.
    /// Clauses: S-02-8 (param kinds), S-02-12 (closures capture cells),
    /// S-02-38 (`<lambda>` frame name).
    Lambda(Box<LambdaSig>, Box<Expr>),
    /// `[x for t in it if c]` — desugars to a hidden function in lowering but
    /// is stored faithfully here.
    /// Clauses: S-02-2 (comprehensions own a scope), S-03-3 area (body scope).
    ListComp(Box<Expr>, Vec<CompClause>),
    /// `{x for t in it if c}`. Clauses: as ListComp + S-01-25.
    SetComp(Box<Expr>, Vec<CompClause>),
    /// `{k: v for t in it if c}`. Clauses: as ListComp + S-01-21.
    DictComp(Box<Expr>, Box<Expr>, Vec<CompClause>),
    /// `(x for t in it if c)` — generator expression.
    /// Clauses: S-02-20..S-02-25 (generator semantics apply; lazy evaluation).
    Generator(Box<Expr>, Vec<CompClause>),
    /// `yield e` — generator suspension edge (explicit in FnBody per IR shape).
    /// Clauses: S-02-21 (suspend/produce), S-02-24 (first send), S-02-25 (close).
    Yield(Option<Box<Expr>>),
    /// `yield from sub`. Clause: S-02-26 (delegation, return-value plumbing).
    YieldFrom(Box<Expr>),
    /// `await e`. Clause: S-05-13 area (async protocol, spec 05 builtins table).
    Await(Box<Expr>),
    /// f-string: interleaved literal chunks and interpolated expressions.
    /// Clauses: S-01-16 (str() default, format specs, nesting), C-18 (rendering).
    FString(Vec<FStrPart>),
    /// `x if c else y`. Clause: S-01-27 (condition by truthiness); only the
    /// taken branch evaluates.
    IfExp(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `*x` in a call arg list / display. Clause: S-02-9 (star unpacking order).
    Starred(Box<Expr>),
    /// Named expression spelled in expression position.
    /// Clause: S-02-2 (walrus binds in the enclosing scope, never comprehension scope).
    Named(Box<Expr>, Box<Expr>),
}

/// One argument in a [`Expr::Call`].
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// `f(x)`.
    Pos(Expr),
    /// `f(*xs)`. Clause: S-02-9.
    Star(Expr),
    /// `f(k=v)`. Clause: S-02-8 (keyword must bind by name).
    Kw(Name, Expr),
    /// `f(**ks)`. Clause: S-02-9.
    KwStar(Expr),
}

/// Target/expression pair of a dict display (after the lowering boundary keeps
/// `None` for `**spam`, represented by [`Expr::DictExpand`]).
#[derive(Debug, Clone, PartialEq)]
pub struct DictItem {
    /// Key expression (`None` never appears here — see `Expr::DictExpand`).
    pub key: Expr,
    /// Value expression.
    pub value: Expr,
}

/// Lambda parameter signature (same shape as function params, no defaults
/// beyond expressions; see [`crate::item::Param`]).
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaSig {
    /// Parameter list in source order.
    pub params: Vec<crate::item::Param>,
}

/// One `for ... [if ...]` clause of a comprehension.
#[derive(Debug, Clone, PartialEq)]
pub struct CompClause {
    /// Loop target (Name/Tuple/Starred pattern; a subset of full patterns).
    pub target: Expr,
    /// Iterable expression, evaluated once per clause.
    /// Clause: S-02-17 (iter() obtained once per for — applies per clause).
    pub iter: Expr,
    /// Zero or more `if` filters (truthiness-tested).
    pub ifs: Vec<Expr>,
}

/// A piece of an f-string.
#[derive(Debug, Clone, PartialEq)]
pub enum FStrPart {
    /// Literal text (post-escape-processing).
    Lit(String),
    /// `{expr}` / `{expr!r}` / `{expr:spec}` / `{expr=}` (debug form).
    /// Clause: S-01-16 (conversion, format-spec semantics, C-18 rendering).
    /// Debug-form dump convention: an Interp with `debug_text: Some(t)` is
    /// rendered as `{t!flag:spec}` (e.g. `f"{x = !r:>4}"` dumps verbatim), so
    /// the dump round-trips the source spelling including whitespace; the
    /// runtime renders `t` as literal text before the value (Plan 02 choice).
    Interp {
        /// The interpolated expression.
        expr: Expr,
        /// `!s`/`!r`/`!a` conversion, if written OR implied by the debug form
        /// (debug text present with no explicit conversion and no format spec
        /// implies `!r`, per CPython PEP 701 debug-form semantics).
        conv: Option<Conversion>,
        /// Format spec (itself possibly nested f-string parts).
        spec: Option<Vec<FStrPart>>,
        /// Debug-form literal source text: everything between `{` and the
        /// conversion/spec/`}` (e.g. `"x = "` for `f"{x = }"`). `None` when
        /// the field is not a debug form.
        debug_text: Option<String>,
    },
}

/// f-string conversion flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    /// `!s`
    S,
    /// `!r`
    R,
    /// `!a`
    A,
}

/// Binary operators. Sign/rounding semantics are cited on [`Expr::BinOp`];
/// the discriminant only picks the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `@` (matmul; v1 raises TypeError outside numpy-likes — spec 05).
    MatMul,
    /// `/` — clause: S-01-6 (always float).
    Div,
    /// `//` — clause: S-01-3 (floor toward -inf).
    FloorDiv,
    /// `%` — clause: S-01-4 (sign of divisor).
    Mod,
    /// `**` — clauses: S-01-2 (int pow rules), precedence/right-assoc.
    Pow,
    /// `<<` — clause: S-01-1.
    LShift,
    /// `>>` — clause: S-01-1 (arithmetic shift).
    RShift,
    /// `|` (also set union — S-01-25).
    BitOr,
    /// `^` (also set symmetric difference — S-01-25).
    BitXor,
    /// `&` (also set intersection — S-01-25).
    BitAnd,
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `+x`
    Plus,
    /// `-x`
    Minus,
    /// `~x` — clause: S-01-1 (`~x == -x-1`).
    Invert,
    /// `not x` — clause: S-01-27 (result is exactly True/False by truthiness).
    Not,
}

/// Boolean short-circuit connective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoolOp {
    /// `and` — returns first falsy operand else last. Clause: S-01-30.
    And,
    /// `or` — returns first truthy operand else last. Clause: S-01-30.
    Or,
}

/// Comparison operator in a [`Expr::Compare`] chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    /// `==` — clause: S-01-19 area (recursive equality), S-01-22 (dict).
    Eq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    LtE,
    /// `>`
    Gt,
    /// `>=`
    GtE,
    /// `is` — identity. Clause: S-01-5 area (equality vs identity, spec 01 §5).
    Is,
    /// `is not`
    IsNot,
    /// `in` — membership. Clauses: S-01-19/20/21/25 (per-container rules).
    In,
    /// `not in`
    NotIn,
}

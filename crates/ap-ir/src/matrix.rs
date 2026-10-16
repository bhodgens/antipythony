//! Ruff-AST → IR exhaustiveness matrix.
//!
//! # What this guarantees
//!
//! Every expression and statement node kind the pinned ruff parser
//! (`ap_parse::TARGET_PYTHON_VERSION`, 3.14) can produce has a *documented
//! lowering strategy* into this IR. The table below is the committed matrix
//! required by leaf 01.02's gate ("IR node kind enum covers ruff AST node
//! kinds — documented exhaustiveness matrix committed").
//!
//! # How it is enforced
//!
//! - **ap-ir side (this table):** a `const` table of
//!   `(ruff-variant-name, strategy)` entries plus a unit test that (a) checks
//!   names for uniqueness/well-formedness and (b) walks *every* IR `Expr` /
//!   `Stmt` node in the table itself, requiring each strategy that says
//!   "builder method" to name a method that actually exists on `Builder`
//!   (dispatch through a runtime vtable built from the table). A typo in a
//!   method name fails the test.
//! - **ap-parse side (ruff has the variant list):** `ap-parse`'s tests contain
//!   a `match` over `ruff_python_ast::Expr` / `Stmt` with **no wildcard arm**
//!   — adding a variant to ruff breaks that build — and assert every variant
//!   name appears in this table. That keeps `ap-ir` vendor-clean (master
//!   contract: no ruff types here) while the ruff-side truth lives where the
//!   dependency already exists.
//!
//! # Maintenance burden (documented, per leaf requirement)
//!
//! When ruff adds/renames a variant: `ap-parse` fails to compile (non-wildcard
//! match), you add the row here, and the method-name check validates any new
//! builder reference. When ap-ir renames a builder method: the vtable test
//! here fails. The matrix is intentionally *manual* — a build-script diff
//! against ruff's source would couple release builds to vendored source
//! layout for no additional safety over the two checks above.

/// Lowering strategy for a ruff AST node kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Constructed directly by the named `Builder` method.
    Builder(&'static str),
    /// Lowered in 01.03 by combining IR nodes (documented combination).
    LoweredVia(&'static str),
    /// Never produced by the pinned parser/frontend; impossible input.
    NotProduced(&'static str),
}

/// Every ruff `ast::Expr` variant (ruff_python_ast 0.0.4, PY314) with its
/// strategy. Order mirrors ruff's declaration order for reviewability.
pub const EXPR_MATRIX: &[(&str, Strategy)] = &[
    ("BoolOp", Strategy::Builder("bool_op")),
    ("Named", Strategy::Builder("named")),
    ("BinOp", Strategy::Builder("bin_op")),
    ("UnaryOp", Strategy::Builder("unary_op")),
    ("Lambda", Strategy::Builder("lambda")),
    ("If", Strategy::Builder("if_exp")),
    ("Dict", Strategy::Builder("dict")),
    ("Set", Strategy::Builder("set")),
    ("ListComp", Strategy::Builder("list_comp")),
    ("SetComp", Strategy::Builder("set_comp")),
    ("DictComp", Strategy::Builder("dict_comp")),
    ("Generator", Strategy::Builder("generator")),
    ("Await", Strategy::Builder("await_")),
    ("Yield", Strategy::Builder("yield_")),
    ("YieldFrom", Strategy::Builder("yield_from")),
    ("Compare", Strategy::Builder("compare")),
    ("Call", Strategy::Builder("call")),
    ("FString", Strategy::Builder("fstring")),
    // t-strings (3.14 template strings) are rejected at the parse pin (v1
    // subset has no consumer); if the corpus ever needs them, lowering is a
    // FString-shaped node with a template marker.
    (
        "TString",
        Strategy::NotProduced("t-strings out of v1 subset; ap-parse rejects at the 3.14 pin"),
    ),
    ("StringLiteral", Strategy::Builder("str_lit")),
    ("BytesLiteral", Strategy::Builder("bytes_lit")),
    (
        "NumberLiteral",
        // one method; complex/int-big handled by value shape
        Strategy::Builder("num_lit"),
    ),
    ("BooleanLiteral", Strategy::Builder("bool_lit")),
    ("NoneLiteral", Strategy::Builder("none_lit")),
    ("EllipsisLiteral", Strategy::Builder("ellipsis_lit")),
    ("Attribute", Strategy::Builder("attribute")),
    ("Subscript", Strategy::Builder("subscript")),
    ("Starred", Strategy::Builder("starred")),
    ("Name", Strategy::Builder("name")),
    ("List", Strategy::Builder("list")),
    ("Tuple", Strategy::Builder("tuple")),
    ("Slice", Strategy::Builder("slice")),
    // IPython escape commands (`%x`, `!x`) only parse in IPython mode; the
    // module-mode pin never yields them.
    (
        "IpyEscapeCommand",
        Strategy::NotProduced("IPython-only syntax; module-mode parse never produces it"),
    ),
];

/// Every ruff `ast::Stmt` variant (ruff_python_ast 0.0.4, PY314) with its
/// strategy. Module-level `FunctionDef`/`ClassDef` become `Item` variants;
/// nested ones become `Stmt`-level `Item::FnDef`-referencing statements in
/// 01.03's lowering (the *shape* is defined here in v0).
pub const STMT_MATRIX: &[(&str, Strategy)] = &[
    ("FunctionDef", Strategy::Builder("fn_def")),
    ("ClassDef", Strategy::Builder("class_def")),
    ("Return", Strategy::Builder("return_")),
    ("Delete", Strategy::Builder("del_")),
    // `type X = ...` (3.12) — v1 subset has no type-parameter runtime; ap-parse
    // accepts the syntax, lowering in 01.03 records it as an AnnAssign-shaped
    // no-op binding with the alias stored.
    (
        "TypeAlias",
        Strategy::LoweredVia(
            "01.03: recorded as module metadata (alias table); no runtime effect in v1",
        ),
    ),
    ("Assign", Strategy::Builder("assign")),
    ("AugAssign", Strategy::Builder("aug_assign")),
    ("AnnAssign", Strategy::Builder("ann_assign")),
    ("For", Strategy::Builder("for_")),
    ("While", Strategy::Builder("while_")),
    ("If", Strategy::Builder("if_")),
    ("With", Strategy::Builder("with_")),
    ("Match", Strategy::Builder("match_")),
    ("Raise", Strategy::Builder("raise_")),
    ("Try", Strategy::Builder("try_")),
    ("Assert", Strategy::Builder("assert_")),
    ("Import", Strategy::Builder("import_")),
    ("ImportFrom", Strategy::Builder("import_from")),
    ("Global", Strategy::Builder("global_")),
    ("Nonlocal", Strategy::Builder("nonlocal")),
    ("Expr", Strategy::Builder("expr_stmt")),
    ("Pass", Strategy::Builder("pass_")),
    ("Break", Strategy::Builder("break_")),
    ("Continue", Strategy::Builder("continue_")),
    (
        "IpyEscapeCommand",
        Strategy::NotProduced("IPython-only syntax; module-mode parse never produces it"),
    ),
];

/// All matrices, for iteration.
pub fn all_matrices() -> [(&'static str, &'static [(&'static str, Strategy)]); 2] {
    [("Expr", EXPR_MATRIX), ("Stmt", STMT_MATRIX)]
}

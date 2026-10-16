//! `ap-ir` — compiler IR shared by all backends (no backend or vendor deps).
//!
//! # Vendor-clean rule (master contract)
//!
//! This crate MUST NOT depend on ruff (or any other frontend) types. The
//! AST→IR conversion lives in 01.03, in `ap-parse` or a glue module. The only
//! link to ruff is the *names* in [`matrix`], checked from `ap-parse`'s tests.
//!
//! # Stability
//!
//! Every node derives `Debug`, `Clone`, `PartialEq` — golden tests (01.03)
//! compare dumps byte-for-byte and nodes structurally.
//!
//! # Construction
//!
//! All nodes are built through [`Builder`], whose methods each cite the
//! compat-contract clause (C-xx) or spec-pack clause (S-xx-yy) whose semantics
//! the node preserves. No lowering lives here; that is leaf 01.03.
//!
//! # Exhaustiveness
//!
//! [`matrix`] holds the committed ruff-variant → strategy matrix; its module
//! docs describe the two-sided enforcement and the maintenance burden.

pub mod builder;
pub mod dump;
pub mod expr;
pub mod item;
pub mod matrix;
pub mod module;
#[cfg(test)]
mod tests;
pub mod types;
pub mod walk;

pub use builder::{Builder, NumberLit};
pub use dump::{dump_module, dump_module_tree};
pub use expr::{Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, FStrPart, UnaryOp};
pub use item::{
    Ann, ClassDef, ExceptHandler, FnBody, FnDef, ImportAlias, Item, MatchCase, Param, ParamKind,
    Pattern, Stmt, Target, TryStmt, WithItem,
};
pub use module::Module;
pub use types::{DefId, TypeLattice};

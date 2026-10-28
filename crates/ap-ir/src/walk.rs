//! Minimal IR walking utilities.
//!
//! Not a full visitor framework (that lands with 01.04's verification pass if
//! needed) — just what the builder and dump need.

use crate::expr::{Arg, DictItem, Expr, FStrPart};
use crate::item::{FnBody, Item, Stmt, Target};

/// Does this function body contain a `yield` / `yield from` anywhere
/// (including nested lambdas/comprehensions, which make the *enclosing*
/// function a generator in Python? — no: only the directly-bearing scope
/// counts. This walk is therefore scope-fenced: it does not descend into
/// nested `FnDef`s or lambdas. Clause: S-02-20.)
///
/// Comprehension/generator bodies are expressions; a bare `yield` is a syntax
/// error inside a comprehension in 3.14, so not descending is correct.
pub fn body_yields(body: &FnBody) -> bool {
    body.blocks.iter().any(stmt_yields)
}

/// Scope-fenced yield check for a statement.
pub fn stmt_yields(s: &Stmt) -> bool {
    match s {
        Stmt::Assign(_, v) => expr_yields(v),
        Stmt::AugAssign(_, _, v) => expr_yields(v),
        Stmt::AnnAssign(_, _, v) => v.as_ref().is_some_and(expr_yields),
        Stmt::Return(v) => v.as_ref().is_some_and(expr_yields),
        Stmt::If(c, then, els) => {
            expr_yields(c) || then.iter().any(stmt_yields) || els.iter().any(stmt_yields)
        }
        Stmt::While(c, body, els) => {
            expr_yields(c) || body.iter().any(stmt_yields) || els.iter().any(stmt_yields)
        }
        Stmt::For(_, it, body, els) => {
            expr_yields(it) || body.iter().any(stmt_yields) || els.iter().any(stmt_yields)
        }
        Stmt::With(items, body) => {
            items.iter().any(|i| expr_yields(&i.expr)) || body.iter().any(stmt_yields)
        }
        Stmt::Try(t) => {
            t.body.iter().any(stmt_yields)
                || t.handlers.iter().any(|h| h.body.iter().any(stmt_yields))
                || t.orelse.iter().any(stmt_yields)
                || t.finalbody.iter().any(stmt_yields)
        }
        Stmt::Raise(e, cause) => {
            e.as_ref().is_some_and(expr_yields) || cause.as_ref().is_some_and(expr_yields)
        }
        Stmt::Match(subj, cases) => {
            expr_yields(subj)
                || cases.iter().any(|c| {
                    c.guard.as_ref().is_some_and(expr_yields) || c.body.iter().any(stmt_yields)
                })
        }
        Stmt::Assert(t, m) => expr_yields(t) || m.as_ref().is_some_and(expr_yields),
        // Nested defs are scope-fenced: a yield inside them makes *that*
        // function the generator, not this one (S-02-20).
        Stmt::LocalDef(_) | Stmt::LocalClass(_) => false,
        Stmt::Del(targets) => targets.iter().any(target_yields),
        Stmt::ExprStmt(e) => expr_yields(e),
        Stmt::Global(_) | Stmt::Nonlocal(_) | Stmt::Import(_) | Stmt::ImportFrom(..) => false,
        Stmt::Pass | Stmt::Break | Stmt::Continue => false,
        Stmt::Line(_) => false,
    }
}

/// Scope-fenced yield check for an expression.
pub fn expr_yields(e: &Expr) -> bool {
    match e {
        Expr::Yield(_) | Expr::YieldFrom(_) => true,
        Expr::BinOp(_, a, b) => expr_yields(a) || expr_yields(b),
        Expr::UnaryOp(_, x) | Expr::Starred(x) | Expr::Await(x) | Expr::DictExpand(x) => {
            expr_yields(x)
        }
        Expr::BoolOp(_, xs) | Expr::List(xs) | Expr::Tuple(xs) | Expr::Set(xs) => {
            xs.iter().any(expr_yields)
        }
        Expr::Compare(_, xs) => xs.iter().any(expr_yields),
        Expr::Call(f, args) => expr_yields(f) || args.iter().any(|a| a.yields()),
        Expr::Attribute(obj, _) => expr_yields(obj),
        Expr::Subscript(obj, key) => expr_yields(obj) || expr_yields(key),
        Expr::Slice(a, b, c) => [a, b, c].into_iter().flatten().any(|e| expr_yields(e)),
        Expr::Dict(items) => items.iter().any(DictItem::yields),
        Expr::Lambda(_, body) => expr_yields(body),
        // Comprehensions own a scope; yields inside them are a syntax error,
        // so their element/iter expressions cannot make *this* fn a generator.
        // Their iterables still run in the enclosing scope for the first
        // clause — but a yield there is also a syntax error. Skip.
        Expr::ListComp(..) | Expr::SetComp(..) | Expr::DictComp(..) | Expr::Generator(..) => false,
        Expr::FString(parts) => parts.iter().any(FStrPart::yields),
        Expr::IfExp(c, t, e) => expr_yields(c) || expr_yields(t) || expr_yields(e),
        Expr::Named(t, v) => expr_yields(t) || expr_yields(v),
        Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitComplex(..)
        | Expr::LitStr(_)
        | Expr::LitBytes(_)
        | Expr::LitBool(_)
        | Expr::LitNone
        | Expr::LitEllipsis
        | Expr::Name(_) => false,
    }
}

impl Arg {
    fn yields(&self) -> bool {
        match self {
            Arg::Pos(e) | Arg::Star(e) | Arg::Kw(_, e) | Arg::KwStar(e) => expr_yields(e),
        }
    }
}

impl DictItem {
    fn yields(&self) -> bool {
        expr_yields(&self.key) || expr_yields(&self.value)
    }
}

impl FStrPart {
    fn yields(&self) -> bool {
        match self {
            FStrPart::Lit(_) => false,
            FStrPart::Interp { expr, spec, .. } => {
                expr_yields(expr)
                    || spec
                        .as_ref()
                        .is_some_and(|s| s.iter().any(FStrPart::yields))
            }
        }
    }
}

fn target_yields(t: &Target) -> bool {
    match t {
        Target::Name(_) => false,
        Target::Subscript(o, k) => expr_yields(o) || expr_yields(k),
        Target::Attribute(o, _) => expr_yields(o),
        Target::Tuple(xs) => xs.iter().any(target_yields),
        Target::Starred(x) => expr_yields(x),
    }
}

impl Item {
    /// Every statement in the item (shallow: module items' own bodies).
    pub fn stmts(&self) -> &[Stmt] {
        match self {
            Item::FnDef(f) => &f.body.blocks,
            Item::ClassDef(c) => &c.body,
            Item::Stmt(s) => std::slice::from_ref(s),
        }
    }
}

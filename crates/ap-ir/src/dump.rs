//! Compact indented-tree IR dump, used for golden tests (01.03 goldens
//! compare this text byte-for-byte).
//!
//! Format contract:
//! - One node per line: `kind(arg, arg, ...)`.
//! - Children indented two spaces per depth level.
//! - Deterministic: no addresses, no hash-order dependence, floats via
//!   shortest round-trip (`ryu`-style via Rust's float Display).
//! - Strings are quoted with Python-style single quotes and minimal escaping;
//!   bytes as `b'..'` with `\xNN` for non-printables.

use std::fmt::{self, Write};

use crate::expr::{Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, FStrPart, UnaryOp};
use crate::item::{
    ClassDef, FnBody, FnDef, ImportAlias, Item, MatchCase, Param, ParamKind, Pattern, Stmt, Target,
    TryStmt,
};
use crate::module::Module;
use crate::types::TypeLattice;

/// Dump a whole module to its canonical text form.
pub fn dump_module(m: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "module");
    for item in &m.items {
        dump_item(&mut out, item, 1);
    }
    out
}

/// Dump a whole module to the T-02 testability-bin form: one node-kind line
/// per IR node, `Kind` + optional ` detail`, children indented two spaces.
///
/// Kind names are the PascalCase of the plan-01 design shape list, pinned by
/// the T-02 contract (`BinOp` example; CONTRACT-GAPS G-1). Unlike the golden
/// form, expressions appear as their OWN lines (one `BinOp` line per BinOp
/// node — `test_binop_chain_count` counts two for `a + b + c`).
pub fn dump_module_tree(m: &Module) -> String {
    let mut out = String::new();
    for item in &m.items {
        tree_item(&mut out, item, 0);
    }
    out
}

// ---- T-02 tree form --------------------------------------------------------

fn tree_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn tree_item(out: &mut String, item: &Item, depth: usize) {
    match item {
        Item::FnDef(f) => tree_fn_def(out, f, depth),
        Item::ClassDef(c) => tree_class_def(out, c, depth),
        Item::Stmt(s) => tree_stmt(out, s, depth),
    }
}

fn tree_fn_def(out: &mut String, f: &FnDef, depth: usize) {
    tree_indent(out, depth);
    let sig = f
        .params
        .iter()
        .map(dump_param)
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "FunctionDef {}({})", f.name, sig);
    for d in &f.decorators {
        tree_indent(out, depth + 1);
        let _ = writeln!(out, "Decorator");
        tree_expr(out, d, depth + 2);
    }
    for s in &f.body.blocks {
        tree_stmt(out, s, depth + 1);
    }
}

fn tree_class_def(out: &mut String, c: &ClassDef, depth: usize) {
    tree_indent(out, depth);
    let mut header = format!("ClassDef {}", c.name);
    if !c.bases.is_empty() || c.metaclass.is_some() {
        header.push('(');
        let parts: Vec<String> = c
            .bases
            .iter()
            .map(dump_expr_inline)
            .chain(
                c.metaclass
                    .as_ref()
                    .map(|m| format!("metaclass={}", dump_expr_inline(m))),
            )
            .collect();
        header.push_str(&parts.join(", "));
        header.push(')');
    }
    let _ = writeln!(out, "{header}");
    for d in &c.decorators {
        tree_indent(out, depth + 1);
        let _ = writeln!(out, "Decorator");
        tree_expr(out, d, depth + 2);
    }
    for s in &c.body {
        tree_stmt(out, s, depth + 1);
    }
}

fn tree_stmt(out: &mut String, s: &Stmt, depth: usize) {
    match s {
        Stmt::Assign(targets, v) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Assign");
            for t in targets {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "target {}", dump_target(t));
            }
            tree_expr(out, v, depth + 1);
        }
        Stmt::AugAssign(t, op, v) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "AugAssign {}=", dump_binop(*op));
            tree_indent(out, depth + 1);
            let _ = writeln!(out, "target {}", dump_target(t));
            tree_expr(out, v, depth + 1);
        }
        Stmt::AnnAssign(t, ann, v) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "AnnAssign {}", dump_ann(ann));
            tree_indent(out, depth + 1);
            let _ = writeln!(out, "target {}", dump_target(t));
            if let Some(v) = v {
                tree_expr(out, v, depth + 1);
            }
        }
        Stmt::Return(v) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Return");
            if let Some(v) = v {
                tree_expr(out, v, depth + 1);
            }
        }
        Stmt::If(cond, then, els) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "If");
            tree_expr(out, cond, depth + 1);
            for s in then {
                tree_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "else");
                for s in els {
                    tree_stmt(out, s, depth + 2);
                }
            }
        }
        Stmt::While(cond, body, els) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "While");
            tree_expr(out, cond, depth + 1);
            for s in body {
                tree_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "elseloop");
                for s in els {
                    tree_stmt(out, s, depth + 2);
                }
            }
        }
        Stmt::For(t, it, body, els) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "For");
            tree_indent(out, depth + 1);
            let _ = writeln!(out, "target {}", dump_target(t));
            tree_expr(out, it, depth + 1);
            for s in body {
                tree_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "elseloop");
                for s in els {
                    tree_stmt(out, s, depth + 2);
                }
            }
        }
        Stmt::With(items, body) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "With");
            for i in items {
                tree_expr(out, &i.expr, depth + 1);
                if let Some(t) = &i.target {
                    tree_indent(out, depth + 1);
                    let _ = writeln!(out, "as {}", dump_target(t));
                }
            }
            for s in body {
                tree_stmt(out, s, depth + 1);
            }
        }
        Stmt::Try(t) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Try");
            for s in &t.body {
                tree_stmt(out, s, depth + 1);
            }
            for h in &t.handlers {
                tree_indent(out, depth + 1);
                match &h.ty {
                    Some(ty) => {
                        let _ = writeln!(out, "ExceptHandler");
                        tree_expr(out, ty, depth + 2);
                    }
                    None => {
                        let _ = writeln!(out, "ExceptHandler");
                    }
                }
                for s in &h.body {
                    tree_stmt(out, s, depth + 2);
                }
            }
            if !t.orelse.is_empty() {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "elsetry");
                for s in &t.orelse {
                    tree_stmt(out, s, depth + 2);
                }
            }
            if !t.finalbody.is_empty() {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "finally");
                for s in &t.finalbody {
                    tree_stmt(out, s, depth + 2);
                }
            }
        }
        Stmt::Raise(e, cause) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Raise");
            if let Some(e) = e {
                tree_expr(out, e, depth + 1);
            }
            if let Some(c) = cause {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "from");
                tree_expr(out, c, depth + 2);
            }
        }
        Stmt::Match(subj, cases) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Match");
            tree_expr(out, subj, depth + 1);
            for case in cases {
                tree_indent(out, depth + 1);
                let _ = write!(out, "case {}", dump_pattern(&case.pattern));
                if let Some(g) = &case.guard {
                    let _ = write!(out, " if {}", dump_expr_inline(g));
                }
                let _ = writeln!(out);
                for s in &case.body {
                    tree_stmt(out, s, depth + 2);
                }
            }
        }
        Stmt::Assert(t, m) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Assert");
            tree_expr(out, t, depth + 1);
            if let Some(m) = m {
                tree_expr(out, m, depth + 1);
            }
        }
        Stmt::Global(names) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Global {}", names.join(", "));
        }
        Stmt::Nonlocal(names) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Nonlocal {}", names.join(", "));
        }
        Stmt::Import(aliases) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Import {}", alias_list(aliases));
        }
        Stmt::ImportFrom(module, aliases, star) => {
            tree_indent(out, depth);
            if *star {
                let _ = writeln!(out, "ImportFrom {module} *");
            } else {
                let _ = writeln!(out, "ImportFrom {module} {}", alias_list(aliases));
            }
        }
        Stmt::Del(targets) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Delete");
            for t in targets {
                tree_indent(out, depth + 1);
                let _ = writeln!(out, "{}", dump_target(t));
            }
        }
        Stmt::Pass => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Pass");
        }
        Stmt::ExprStmt(e) => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Expr");
            tree_expr(out, e, depth + 1);
        }
        Stmt::Break => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Break");
        }
        Stmt::Continue => {
            tree_indent(out, depth);
            let _ = writeln!(out, "Continue");
        }
        Stmt::LocalDef(f) => tree_fn_def(out, f, depth),
        Stmt::LocalClass(c) => tree_class_def(out, c, depth),
    }
}

fn tree_expr(out: &mut String, e: &Expr, depth: usize) {
    tree_indent(out, depth);
    match e {
        Expr::BinOp(op, a, b) => {
            let _ = writeln!(out, "BinOp {}", dump_binop(*op));
            tree_expr(out, a, depth + 1);
            tree_expr(out, b, depth + 1);
        }
        Expr::UnaryOp(op, x) => {
            let _ = writeln!(out, "UnaryOp {}", dump_unaryop(*op));
            tree_expr(out, x, depth + 1);
        }
        Expr::BoolOp(op, xs) => {
            let name = match op {
                BoolOp::And => "And",
                BoolOp::Or => "Or",
            };
            let _ = writeln!(out, "BoolOp {name}");
            for x in xs {
                tree_expr(out, x, depth + 1);
            }
        }
        Expr::Compare(ops, xs) => {
            let _ = writeln!(
                out,
                "Compare {}",
                ops.iter()
                    .map(|o| dump_cmpop(*o))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            for x in xs {
                tree_expr(out, x, depth + 1);
            }
        }
        Expr::Call(f, args) => {
            let _ = writeln!(out, "Call");
            tree_expr(out, f, depth + 1);
            for a in args {
                tree_arg(out, a, depth + 1);
            }
        }
        Expr::Attribute(obj, a) => {
            let _ = writeln!(out, "Attribute {a}");
            tree_expr(out, obj, depth + 1);
        }
        Expr::Subscript(obj, k) => {
            let _ = writeln!(out, "Subscript");
            tree_expr(out, obj, depth + 1);
            tree_expr(out, k, depth + 1);
        }
        Expr::List(xs) => {
            let _ = writeln!(out, "List");
            for x in xs {
                tree_expr(out, x, depth + 1);
            }
        }
        Expr::Tuple(xs) => {
            let _ = writeln!(out, "Tuple");
            for x in xs {
                tree_expr(out, x, depth + 1);
            }
        }
        Expr::Set(xs) => {
            let _ = writeln!(out, "Set");
            for x in xs {
                tree_expr(out, x, depth + 1);
            }
        }
        Expr::Dict(items) => {
            let _ = writeln!(out, "Dict");
            for i in items {
                tree_expr(out, &i.key, depth + 1);
                tree_expr(out, &i.value, depth + 1);
            }
        }
        Expr::DictExpand(x) => {
            let _ = writeln!(out, "DictExpand");
            tree_expr(out, x, depth + 1);
        }
        Expr::Lambda(sig, body) => {
            let ps = sig
                .params
                .iter()
                .map(dump_param)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "Lambda {ps}");
            tree_expr(out, body, depth + 1);
        }
        Expr::ListComp(_, clauses) => {
            let _ = writeln!(out, "Comprehension list");
            tree_clauses(out, clauses, depth + 1);
        }
        Expr::SetComp(_, clauses) => {
            let _ = writeln!(out, "Comprehension set");
            tree_clauses(out, clauses, depth + 1);
        }
        Expr::DictComp(_, _, clauses) => {
            let _ = writeln!(out, "Comprehension dict");
            tree_clauses(out, clauses, depth + 1);
        }
        Expr::Generator(_, clauses) => {
            let _ = writeln!(out, "Generator");
            tree_clauses(out, clauses, depth + 1);
        }
        Expr::Yield(v) => {
            let _ = writeln!(out, "Yield");
            if let Some(v) = v {
                tree_expr(out, v, depth + 1);
            }
        }
        Expr::YieldFrom(x) => {
            let _ = writeln!(out, "YieldFrom");
            tree_expr(out, x, depth + 1);
        }
        Expr::Await(x) => {
            let _ = writeln!(out, "Await");
            tree_expr(out, x, depth + 1);
        }
        Expr::FString(_) => {
            let _ = writeln!(out, "FString {}", dump_expr_inline(e));
        }
        Expr::IfExp(..) => {
            let _ = writeln!(out, "IfExp {}", dump_expr_inline(e));
        }
        Expr::Starred(x) => {
            let _ = writeln!(out, "Starred");
            tree_expr(out, x, depth + 1);
        }
        Expr::Named(t, v) => {
            let _ = writeln!(out, "Named");
            tree_expr(out, t, depth + 1);
            tree_expr(out, v, depth + 1);
        }
        // Literals and leaves carry their value on the kind line.
        Expr::LitInt(_)
        | Expr::LitBigInt(_)
        | Expr::LitFloat(_)
        | Expr::LitComplex(..)
        | Expr::LitStr(_)
        | Expr::LitBytes(_)
        | Expr::LitBool(_)
        | Expr::LitNone
        | Expr::LitEllipsis
        | Expr::Name(_)
        | Expr::Slice(..) => {
            let _ = writeln!(out, "{}", dump_expr_inline(e));
        }
    }
}

fn tree_arg(out: &mut String, a: &Arg, depth: usize) {
    tree_indent(out, depth);
    match a {
        Arg::Pos(e) => {
            let _ = writeln!(out, "arg");
            tree_expr(out, e, depth + 1);
        }
        Arg::Star(e) => {
            let _ = writeln!(out, "arg *");
            tree_expr(out, e, depth + 1);
        }
        Arg::Kw(k, e) => {
            let _ = writeln!(out, "kwarg {k}");
            tree_expr(out, e, depth + 1);
        }
        Arg::KwStar(e) => {
            let _ = writeln!(out, "arg **");
            tree_expr(out, e, depth + 1);
        }
    }
}

fn tree_clauses(out: &mut String, clauses: &[crate::expr::CompClause], depth: usize) {
    for c in clauses {
        tree_indent(out, depth);
        let _ = writeln!(out, "clause");
        tree_indent(out, depth + 1);
        let _ = writeln!(out, "target {}", dump_expr_inline(&c.target));
        tree_indent(out, depth + 1);
        let _ = writeln!(out, "iter");
        tree_expr(out, &c.iter, depth + 2);
        for i in &c.ifs {
            tree_indent(out, depth + 1);
            let _ = writeln!(out, "if");
            tree_expr(out, i, depth + 2);
        }
    }
}

fn alias_list(aliases: &[ImportAlias]) -> String {
    aliases
        .iter()
        .map(dump_import_alias)
        .collect::<Vec<_>>()
        .join(", ")
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn dump_item(out: &mut String, item: &Item, depth: usize) {
    match item {
        Item::FnDef(f) => dump_fn_def(out, f, depth),
        Item::ClassDef(c) => dump_class_def(out, c, depth),
        Item::Stmt(s) => dump_stmt(out, s, depth),
    }
}

fn dump_fn_def(out: &mut String, f: &FnDef, depth: usize) {
    indent(out, depth);
    let _ = writeln!(
        out,
        "fn {}({}){}{}{}:",
        f.name,
        f.params
            .iter()
            .map(dump_param)
            .collect::<Vec<_>>()
            .join(", "),
        f.is_async.then_some(" async").unwrap_or(""),
        f.is_generator.then_some(" gen").unwrap_or(""),
        match &f.return_ann {
            Some(a) => format!(" -> {}", dump_ann(a)),
            None => String::new(),
        }
    );
    for d in &f.decorators {
        indent(out, depth + 1);
        let _ = writeln!(out, "@decorator");
        dump_expr(out, d, depth + 2);
    }
    dump_body(out, &f.body, depth + 1);
}

fn dump_param(p: &Param) -> String {
    let kind = match p.kind {
        ParamKind::PosOnly => "/",
        ParamKind::PosOrKw => "",
        ParamKind::VarArg => "*",
        ParamKind::KwOnly => "",
        ParamKind::KwArg => "**",
    };
    let mut s = format!("{kind}{}", p.name);
    if let Some(d) = &p.default {
        let _ = write!(s, "={}", dump_expr_inline(d));
    }
    if let Some(a) = &p.ann {
        let _ = write!(s, ":{}", dump_ann(a));
    }
    s
}

fn dump_ann(a: &crate::item::Ann) -> String {
    let t = match &a.ty {
        TypeLattice::Unknown => String::new(),
        t => format!("@{t}"),
    };
    format!("{}{t}", dump_expr_inline(&a.expr))
}

fn dump_class_def(out: &mut String, c: &ClassDef, depth: usize) {
    indent(out, depth);
    let mut header = format!("class {}#{}", c.name, c.id.0);
    if !c.bases.is_empty() || c.metaclass.is_some() {
        header.push('(');
        let parts: Vec<String> = c
            .bases
            .iter()
            .map(dump_expr_inline)
            .chain(
                c.metaclass
                    .as_ref()
                    .map(|m| format!("metaclass={}", dump_expr_inline(m))),
            )
            .collect();
        header.push_str(&parts.join(", "));
        header.push(')');
    }
    let _ = writeln!(out, "{header}:");
    for d in &c.decorators {
        indent(out, depth + 1);
        let _ = writeln!(out, "@decorator");
        dump_expr(out, d, depth + 2);
    }
    for s in &c.body {
        dump_stmt(out, s, depth + 1);
    }
}

fn dump_body(out: &mut String, body: &FnBody, depth: usize) {
    for s in &body.blocks {
        dump_stmt(out, s, depth);
    }
}

fn dump_stmt(out: &mut String, s: &Stmt, depth: usize) {
    match s {
        Stmt::Assign(targets, v) => {
            indent(out, depth);
            let _ = writeln!(
                out,
                "assign {} = {}",
                targets
                    .iter()
                    .map(dump_target)
                    .collect::<Vec<_>>()
                    .join(", "),
                dump_expr_inline(v)
            );
        }
        Stmt::AugAssign(t, op, v) => {
            indent(out, depth);
            let _ = writeln!(
                out,
                "augassign {} {}= {}",
                dump_target(t),
                dump_binop(*op),
                dump_expr_inline(v)
            );
        }
        Stmt::AnnAssign(t, ann, v) => {
            indent(out, depth);
            let _ = write!(out, "annassign {} : {}", dump_target(t), dump_ann(ann));
            if let Some(v) = v {
                let _ = write!(out, " = {}", dump_expr_inline(v));
            }
            let _ = writeln!(out);
        }
        Stmt::Return(v) => {
            indent(out, depth);
            match v {
                Some(v) => writeln!(out, "return {}", dump_expr_inline(v)),
                None => writeln!(out, "return"),
            }
            .ok();
        }
        Stmt::If(cond, then, els) => {
            indent(out, depth);
            let _ = writeln!(out, "if {}", dump_expr_inline(cond));
            for s in then {
                dump_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                indent(out, depth);
                let _ = writeln!(out, "else");
                for s in els {
                    dump_stmt(out, s, depth + 1);
                }
            }
        }
        Stmt::While(cond, body, els) => {
            indent(out, depth);
            let _ = writeln!(out, "while {}", dump_expr_inline(cond));
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                indent(out, depth);
                let _ = writeln!(out, "elseloop");
                for s in els {
                    dump_stmt(out, s, depth + 1);
                }
            }
        }
        Stmt::For(t, it, body, els) => {
            indent(out, depth);
            let _ = writeln!(out, "for {} in {}", dump_target(t), dump_expr_inline(it));
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            if !els.is_empty() {
                indent(out, depth);
                let _ = writeln!(out, "elseloop");
                for s in els {
                    dump_stmt(out, s, depth + 1);
                }
            }
        }
        Stmt::With(items, body) => {
            indent(out, depth);
            let _ = writeln!(
                out,
                "with {}",
                items
                    .iter()
                    .map(|i| match &i.target {
                        Some(t) => format!("{} as {}", dump_expr_inline(&i.expr), dump_target(t)),
                        None => dump_expr_inline(&i.expr),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
        }
        Stmt::Try(t) => dump_try(out, t, depth),
        Stmt::Raise(e, cause) => {
            indent(out, depth);
            let _ = write!(out, "raise");
            if let Some(e) = e {
                let _ = write!(out, " {}", dump_expr_inline(e));
            }
            if let Some(c) = cause {
                let _ = write!(out, " from {}", dump_expr_inline(c));
            }
            let _ = writeln!(out);
        }
        Stmt::Match(subj, cases) => {
            indent(out, depth);
            let _ = writeln!(out, "match {}", dump_expr_inline(subj));
            for case in cases {
                dump_match_case(out, case, depth + 1);
            }
        }
        Stmt::Assert(t, m) => {
            indent(out, depth);
            let _ = write!(out, "assert {}", dump_expr_inline(t));
            if let Some(m) = m {
                let _ = write!(out, ", {}", dump_expr_inline(m));
            }
            let _ = writeln!(out);
        }
        Stmt::Global(names) => {
            indent(out, depth);
            let _ = writeln!(out, "global {}", names.join(", "));
        }
        Stmt::Nonlocal(names) => {
            indent(out, depth);
            let _ = writeln!(out, "nonlocal {}", names.join(", "));
        }
        Stmt::Import(aliases) => {
            indent(out, depth);
            let _ = writeln!(
                out,
                "import {}",
                aliases
                    .iter()
                    .map(dump_import_alias)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Stmt::ImportFrom(module, aliases, star) => {
            indent(out, depth);
            let _ = write!(out, "from {module} import ");
            if *star {
                let _ = writeln!(out, "*");
            } else {
                let _ = writeln!(
                    out,
                    "{}",
                    aliases
                        .iter()
                        .map(dump_import_alias)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        Stmt::Del(targets) => {
            indent(out, depth);
            let _ = writeln!(
                out,
                "del {}",
                targets
                    .iter()
                    .map(dump_target)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Stmt::Pass => {
            indent(out, depth);
            let _ = writeln!(out, "pass");
        }
        Stmt::ExprStmt(e) => {
            indent(out, depth);
            let _ = writeln!(out, "expr {}", dump_expr_inline(e));
        }
        Stmt::Break => {
            indent(out, depth);
            let _ = writeln!(out, "break");
        }
        Stmt::Continue => {
            indent(out, depth);
            let _ = writeln!(out, "continue");
        }
        Stmt::LocalDef(f) => dump_fn_def(out, f, depth),
        Stmt::LocalClass(c) => dump_class_def(out, c, depth),
    }
}

fn dump_try(out: &mut String, t: &TryStmt, depth: usize) {
    indent(out, depth);
    let _ = writeln!(out, "try");
    for s in &t.body {
        dump_stmt(out, s, depth + 1);
    }
    for h in &t.handlers {
        indent(out, depth);
        match (&h.ty, &h.name) {
            (None, _) => {
                let _ = writeln!(out, "except");
            }
            (Some(ty), None) => {
                let _ = writeln!(out, "except {}", dump_expr_inline(ty));
            }
            (Some(ty), Some(n)) => {
                let _ = writeln!(out, "except {} as {}", dump_expr_inline(ty), n);
            }
        }
        for s in &h.body {
            dump_stmt(out, s, depth + 1);
        }
    }
    if !t.orelse.is_empty() {
        indent(out, depth);
        let _ = writeln!(out, "elsetry");
        for s in &t.orelse {
            dump_stmt(out, s, depth + 1);
        }
    }
    if !t.finalbody.is_empty() {
        indent(out, depth);
        let _ = writeln!(out, "finally");
        for s in &t.finalbody {
            dump_stmt(out, s, depth + 1);
        }
    }
}

fn dump_match_case(out: &mut String, case: &MatchCase, depth: usize) {
    indent(out, depth);
    let _ = write!(out, "case {}", dump_pattern(&case.pattern));
    if let Some(g) = &case.guard {
        let _ = write!(out, " if {}", dump_expr_inline(g));
    }
    let _ = writeln!(out);
    for s in &case.body {
        dump_stmt(out, s, depth + 1);
    }
}

fn dump_pattern(p: &Pattern) -> String {
    match p {
        Pattern::Literal(e) => dump_expr_inline(e),
        Pattern::Capture(n) => n.clone(),
        Pattern::Wildcard => "_".into(),
        Pattern::Value(path) => path.join("."),
        Pattern::Group(inner) => format!("({})", dump_pattern(inner)),
        Pattern::Sequence(items, rest) => {
            let mut parts: Vec<String> = items.iter().map(dump_pattern).collect();
            if let Some(r) = rest {
                parts.push(format!("*{}", dump_pattern(r)));
            }
            format!("[{}]", parts.join(", "))
        }
        Pattern::Mapping(items, rest) => {
            let mut parts: Vec<String> = items
                .iter()
                .map(|(k, v)| format!("{}: {}", dump_pattern(k), dump_pattern(v)))
                .collect();
            if let Some(r) = rest {
                parts.push(format!("**{r}"));
            }
            format!("{{{}}}", parts.join(", "))
        }
        Pattern::Class(cls, pos, kw) => {
            let mut parts: Vec<String> = pos.iter().map(dump_pattern).collect();
            parts.extend(kw.iter().map(|(n, p)| format!("{n}={}", dump_pattern(p))));
            format!("{}({})", dump_expr_inline(cls), parts.join(", "))
        }
        Pattern::Or(alts) => alts
            .iter()
            .map(dump_pattern)
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

fn dump_import_alias(a: &ImportAlias) -> String {
    match &a.alias {
        Some(alias) => format!("{} as {}", a.path.join("."), alias),
        None => a.path.join("."),
    }
}

fn dump_target(t: &Target) -> String {
    match t {
        Target::Name(n) => n.clone(),
        Target::Subscript(o, k) => format!("{}[{}]", dump_expr_inline(o), dump_expr_inline(k)),
        Target::Attribute(o, a) => format!("{}.{}", dump_expr_inline(o), a),
        Target::Tuple(items) => format!(
            "({})",
            items.iter().map(dump_target).collect::<Vec<_>>().join(", ")
        ),
        Target::Starred(x) => format!("*{}", dump_expr_inline(x)),
    }
}

// ---- expressions (inline + tree) ------------------------------------------

/// Expressions dump inline (parenthesized by precedence shape, not by source).
/// Keeping them inline makes the dump compact while staying unambiguous.
fn dump_expr_inline(e: &Expr) -> String {
    let mut s = String::new();
    dump_expr(&mut s, e, 0);
    // single-line: strip any accidental newlines (f-strings never emit them)
    s
}

fn dump_expr(out: &mut String, e: &Expr, _depth: usize) {
    match e {
        Expr::LitInt(v) => {
            let _ = write!(out, "{v}");
        }
        Expr::LitBigInt(d) => {
            let _ = write!(out, "{d}");
        }
        Expr::LitFloat(v) => {
            let _ = write!(out, "{v:?}");
        }
        Expr::LitComplex(re, im) => {
            let _ = write!(out, "complex({re:?}, {im:?})");
        }
        Expr::LitStr(s) => {
            let _ = write!(out, "{}", quote_str(s));
        }
        Expr::LitBytes(b) => {
            let _ = write!(out, "{}", quote_bytes(b));
        }
        Expr::LitBool(v) => {
            let _ = write!(out, "{v}");
        }
        Expr::LitNone => {
            let _ = write!(out, "None");
        }
        Expr::LitEllipsis => {
            let _ = write!(out, "...");
        }
        Expr::Name(n) => {
            let _ = write!(out, "{n}");
        }
        Expr::BinOp(op, a, b) => {
            let _ = write!(
                out,
                "({} {} {})",
                dump_expr_inline(a),
                dump_binop(*op),
                dump_expr_inline(b)
            );
        }
        Expr::UnaryOp(op, x) => {
            let _ = write!(out, "({}{})", dump_unaryop(*op), dump_expr_inline(x));
        }
        Expr::BoolOp(op, xs) => {
            let sep = match op {
                BoolOp::And => " and ",
                BoolOp::Or => " or ",
            };
            let _ = write!(out, "({})", join_exprs(xs, sep));
        }
        Expr::Compare(ops, xs) => {
            let mut s = dump_expr_inline(&xs[0]);
            for (op, x) in ops.iter().zip(xs.iter().skip(1)) {
                let _ = write!(s, " {} {}", dump_cmpop(*op), dump_expr_inline(x));
            }
            let _ = write!(out, "({s})");
        }
        Expr::Call(f, args) => {
            let _ = write!(
                out,
                "{}({})",
                dump_expr_inline(f),
                args.iter().map(dump_arg).collect::<Vec<_>>().join(", ")
            );
        }
        Expr::Attribute(obj, a) => {
            let _ = write!(out, "{}.{a}", dump_expr_inline(obj));
        }
        Expr::Subscript(obj, k) => {
            let _ = write!(out, "{}[{}]", dump_expr_inline(obj), dump_expr_inline(k));
        }
        Expr::Slice(lo, hi, step) => {
            let part =
                |o: &Option<Box<Expr>>| o.as_ref().map(|e| dump_expr_inline(e)).unwrap_or_default();
            let _ = write!(out, "{}:{}:{}", part(lo), part(hi), part(step));
        }
        Expr::List(xs) => {
            let _ = write!(out, "[{}]", join_exprs(xs, ", "));
        }
        Expr::Tuple(xs) => {
            if xs.len() == 1 {
                let _ = write!(out, "({},)", dump_expr_inline(&xs[0]));
            } else {
                let _ = write!(out, "({})", join_exprs(xs, ", "));
            }
        }
        Expr::Set(xs) => {
            let _ = write!(out, "{{{}}}", join_exprs(xs, ", "));
        }
        Expr::Dict(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|i| {
                    format!(
                        "{}: {}",
                        dump_expr_inline(&i.key),
                        dump_expr_inline(&i.value)
                    )
                })
                .collect();
            let _ = write!(out, "{{{}}}", parts.join(", "));
        }
        Expr::DictExpand(x) => {
            let _ = write!(out, "**{}", dump_expr_inline(x));
        }
        Expr::Lambda(sig, body) => {
            let _ = write!(
                out,
                "(lambda {}: {})",
                sig.params
                    .iter()
                    .map(dump_param)
                    .collect::<Vec<_>>()
                    .join(", "),
                dump_expr_inline(body)
            );
        }
        Expr::ListComp(elt, clauses) => {
            let _ = write!(out, "[{} {}]", dump_expr_inline(elt), dump_clauses(clauses));
        }
        Expr::SetComp(elt, clauses) => {
            let _ = write!(
                out,
                "{{{} {}}}",
                dump_expr_inline(elt),
                dump_clauses(clauses)
            );
        }
        Expr::DictComp(k, v, clauses) => {
            let _ = write!(
                out,
                "{{{}: {} {}}}",
                dump_expr_inline(k),
                dump_expr_inline(v),
                dump_clauses(clauses)
            );
        }
        Expr::Generator(elt, clauses) => {
            let _ = write!(out, "({} {})", dump_expr_inline(elt), dump_clauses(clauses));
        }
        Expr::Yield(v) => match v {
            Some(v) => {
                let _ = write!(out, "(yield {})", dump_expr_inline(v));
            }
            None => {
                let _ = write!(out, "(yield)");
            }
        },
        Expr::YieldFrom(x) => {
            let _ = write!(out, "(yield from {})", dump_expr_inline(x));
        }
        Expr::Await(x) => {
            let _ = write!(out, "(await {})", dump_expr_inline(x));
        }
        Expr::FString(parts) => {
            let _ = write!(
                out,
                "f\"{}\"",
                parts.iter().map(dump_fstr_part).collect::<String>()
            );
        }
        Expr::IfExp(c, t, e) => {
            let _ = write!(
                out,
                "({} if {} else {})",
                dump_expr_inline(t),
                dump_expr_inline(c),
                dump_expr_inline(e)
            );
        }
        Expr::Starred(x) => {
            let _ = write!(out, "*{}", dump_expr_inline(x));
        }
        Expr::Named(t, v) => {
            let _ = write!(out, "({} := {})", dump_expr_inline(t), dump_expr_inline(v));
        }
    }
}

fn dump_arg(a: &Arg) -> String {
    match a {
        Arg::Pos(e) => dump_expr_inline(e),
        Arg::Star(e) => format!("*{}", dump_expr_inline(e)),
        Arg::Kw(k, e) => format!("{k}={}", dump_expr_inline(e)),
        Arg::KwStar(e) => format!("**{}", dump_expr_inline(e)),
    }
}

fn dump_clauses(clauses: &[crate::expr::CompClause]) -> String {
    clauses
        .iter()
        .map(|c| {
            let mut s = format!(
                "for {} in {}",
                dump_expr_inline(&c.target),
                dump_expr_inline(&c.iter)
            );
            for i in &c.ifs {
                let _ = write!(s, " if {}", dump_expr_inline(i));
            }
            s
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn dump_fstr_part(p: &FStrPart) -> String {
    match p {
        FStrPart::Lit(s) => s.replace('{', "{{").replace('}', "}}"),
        FStrPart::Interp { expr, conv, spec } => {
            let mut s = format!("{{{}}}", dump_expr_inline(expr));
            if let Some(c) = conv {
                let flag = match c {
                    Conversion::S => 's',
                    Conversion::R => 'r',
                    Conversion::A => 'a',
                };
                // rebuild: move the flag inside the braces
                s.pop(); // drop the closing brace we just wrote
                let _ = write!(s, "!{flag}}}");
            }
            if let Some(spec) = spec {
                s.pop(); // drop the closing brace
                let _ = write!(
                    s,
                    ":{}",
                    spec.iter().map(dump_fstr_part).collect::<String>()
                );
                s.push('}');
            }
            s
        }
    }
}

fn join_exprs(xs: &[Expr], sep: &str) -> String {
    xs.iter()
        .map(dump_expr_inline)
        .collect::<Vec<_>>()
        .join(sep)
}

fn dump_binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::MatMul => "@",
        BinOp::Div => "/",
        BinOp::FloorDiv => "//",
        BinOp::Mod => "%",
        BinOp::Pow => "**",
        BinOp::LShift => "<<",
        BinOp::RShift => ">>",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::BitAnd => "&",
    }
}

fn dump_unaryop(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Minus => "-",
        UnaryOp::Invert => "~",
        UnaryOp::Not => "not ",
    }
}

fn dump_cmpop(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::NotEq => "!=",
        CmpOp::Lt => "<",
        CmpOp::LtE => "<=",
        CmpOp::Gt => ">",
        CmpOp::GtE => ">=",
        CmpOp::Is => "is",
        CmpOp::IsNot => "is not",
        CmpOp::In => "in",
        CmpOp::NotIn => "not in",
    }
}

fn quote_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\x7f' => {
                let _ = write!(out, "\\x{:#04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn quote_bytes(b: &[u8]) -> String {
    let mut out = String::with_capacity(b.len() + 3);
    out.push_str("b'");
    for &byte in b {
        match byte {
            b'\'' => out.push_str("\\'"),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\x{byte:02x}");
            }
        }
    }
    out.push('\'');
    out
}

/// Convenience: implement Display for Module via the dump.
impl fmt::Display for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&dump_module(self))
    }
}

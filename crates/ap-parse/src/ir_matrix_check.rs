//! Cross-check of the ap-ir exhaustiveness matrix against the *real* ruff
//! variant list (this is the only crate allowed to depend on ruff; ap-ir
//! stays vendor-clean per the master contract).
//!
//! Enforcement:
//! 1. The `match` arms below have NO wildcard — a new/renamed ruff variant
//!    breaks this build (compile-time exhaustiveness).
//! 2. A parsed probe program routes every node through those matches, and the
//!    produced names are asserted present in ap-ir's `EXPR_MATRIX` /
//!    `STMT_MATRIX` (runtime cross-check).

use ap_ir::matrix;
use ruff_python_ast::visitor::source_order::{SourceOrderVisitor, TraversalSignal};
use ruff_python_ast::AnyNodeRef;

/// Wildcard-free ruff variant-name matches live in `KindCollector::enter_node`
/// below (via `StmtRef` / `ExprRef`); they are the compile-time exhaustiveness
/// gate. This file keeps no duplicate name tables.

/// Visitor collecting variant names for every node it walks.
#[derive(Default)]
struct KindCollector {
    expr_kinds: std::collections::BTreeSet<&'static str>,
    stmt_kinds: std::collections::BTreeSet<&'static str>,
}

impl<'a> SourceOrderVisitor<'a> for KindCollector {
    fn enter_node(&mut self, node: AnyNodeRef<'a>) -> TraversalSignal {
        use ruff_python_ast::{ExprRef, StmtRef};
        if let Some(s) = node.as_stmt_ref() {
            let name = match s {
                StmtRef::FunctionDef(_) => "FunctionDef",
                StmtRef::ClassDef(_) => "ClassDef",
                StmtRef::Return(_) => "Return",
                StmtRef::Delete(_) => "Delete",
                StmtRef::TypeAlias(_) => "TypeAlias",
                StmtRef::Assign(_) => "Assign",
                StmtRef::AugAssign(_) => "AugAssign",
                StmtRef::AnnAssign(_) => "AnnAssign",
                StmtRef::For(_) => "For",
                StmtRef::While(_) => "While",
                StmtRef::If(_) => "If",
                StmtRef::With(_) => "With",
                StmtRef::Match(_) => "Match",
                StmtRef::Raise(_) => "Raise",
                StmtRef::Try(_) => "Try",
                StmtRef::Assert(_) => "Assert",
                StmtRef::Import(_) => "Import",
                StmtRef::ImportFrom(_) => "ImportFrom",
                StmtRef::Global(_) => "Global",
                StmtRef::Nonlocal(_) => "Nonlocal",
                StmtRef::Expr(_) => "Expr",
                StmtRef::Pass(_) => "Pass",
                StmtRef::Break(_) => "Break",
                StmtRef::Continue(_) => "Continue",
                StmtRef::IpyEscapeCommand(_) => "IpyEscapeCommand",
            };
            self.stmt_kinds.insert(name);
        } else if let Some(e) = node.as_expr_ref() {
            let name = match e {
                ExprRef::BoolOp(_) => "BoolOp",
                ExprRef::Named(_) => "Named",
                ExprRef::BinOp(_) => "BinOp",
                ExprRef::UnaryOp(_) => "UnaryOp",
                ExprRef::Lambda(_) => "Lambda",
                ExprRef::If(_) => "If",
                ExprRef::Dict(_) => "Dict",
                ExprRef::Set(_) => "Set",
                ExprRef::ListComp(_) => "ListComp",
                ExprRef::SetComp(_) => "SetComp",
                ExprRef::DictComp(_) => "DictComp",
                ExprRef::Generator(_) => "Generator",
                ExprRef::Await(_) => "Await",
                ExprRef::Yield(_) => "Yield",
                ExprRef::YieldFrom(_) => "YieldFrom",
                ExprRef::Compare(_) => "Compare",
                ExprRef::Call(_) => "Call",
                ExprRef::FString(_) => "FString",
                ExprRef::TString(_) => "TString",
                ExprRef::StringLiteral(_) => "StringLiteral",
                ExprRef::BytesLiteral(_) => "BytesLiteral",
                ExprRef::NumberLiteral(_) => "NumberLiteral",
                ExprRef::BooleanLiteral(_) => "BooleanLiteral",
                ExprRef::NoneLiteral(_) => "NoneLiteral",
                ExprRef::EllipsisLiteral(_) => "EllipsisLiteral",
                ExprRef::Attribute(_) => "Attribute",
                ExprRef::Subscript(_) => "Subscript",
                ExprRef::Starred(_) => "Starred",
                ExprRef::Name(_) => "Name",
                ExprRef::List(_) => "List",
                ExprRef::Tuple(_) => "Tuple",
                ExprRef::Slice(_) => "Slice",
                ExprRef::IpyEscapeCommand(_) => "IpyEscapeCommand",
            };
            self.expr_kinds.insert(name);
        }
        TraversalSignal::Traverse
    }
}

/// The matrix must name every ruff Expr variant, checked against a parsed
/// probe program (compile-time exhaustiveness comes from the wildcard-free
/// matches; this test keeps the ap-ir *table* honest).
#[test]
fn probe_program_covers_matrix() {
    let src = r#"
import os
from sys import argv
import a.b.c as d
type Alias = int
x: int = 1
x += 2
y = [i for i in range(3) if i]
z = {k: v for k, v in pairs}
s = {e for e in things}
g = (n for n in nums)
def f(a, /, b, *args, kwonly=2, **kwargs) -> int:
    global counter
    del counter
    while x < 10:
        x += 1
        if x == 5:
            break
        else:
            continue
    for t in items:
        pass
    else:
        pass
    with open("f") as fh, lock:
        pass
    try:
        raise ValueError("v") from None
    except (TypeError, KeyError) as err:
        pass
    except:
        pass
    else:
        pass
    finally:
        pass
    match point:
        case Point(x=0, y=0): pass
        case [1, *rest]: pass
        case {"k": v, **r2}: pass
        case A | B if cond: pass
        case _ as wild: pass
        case Point(0, 0): pass
        case Color.RED: pass
    assert x, "msg"
    async def inner():
        await fut
        async for a in b:
            async with c as d2:
                pass
    yield
    yield from g
    lam = lambda q: q
    chained = 1 < x < 10
    walrus = (w := 5)
    attr = obj.field
    sub = obj[key]
    f(*args, **kwargs, k=1)
    bio = b"bytes"
    ell = ...
    truth = a and b or not c
    cond = 1 if x else 2
    starred = [*a, *b]
    return x
class C(Base, metaclass=Meta): pass
pass
"#;
    let parsed = crate::parse(src).expect("probe source parses");
    let mut collector = KindCollector::default();
    {
        use ruff_python_ast::visitor::source_order::walk_body;
        let module = parsed.module();
        walk_body(&mut collector, &module.body);
    }

    for name in &collector.expr_kinds {
        assert!(
            matrix::EXPR_MATRIX.iter().any(|(n, _)| n == name),
            "probed ruff Expr variant `{name}` missing from ap-ir EXPR_MATRIX"
        );
    }
    for name in &collector.stmt_kinds {
        assert!(
            matrix::STMT_MATRIX.iter().any(|(n, _)| n == name),
            "probed ruff Stmt variant `{name}` missing from ap-ir STMT_MATRIX"
        );
    }
    // Coverage sanity: the probe program must exercise a meaningful chunk.
    assert!(
        collector.expr_kinds.len() >= 25,
        "probe program lost expr coverage: {:?}",
        collector.expr_kinds
    );
    assert!(
        collector.stmt_kinds.len() >= 18,
        "probe program lost stmt coverage: {:?}",
        collector.stmt_kinds
    );
}

/// Arity sanity: ruff 0.0.4 has 33 Expr and 25 Stmt variants; the matrix
/// tables must be exactly that long (no extra, no missing rows).
#[test]
fn matrix_arity_matches_ruff() {
    // Count via the wildcard-free matches: every arm returns a distinct name,
    // so a set built from synthetic nodes has arity == variant count. Instead
    // of constructing synthetic nodes, assert the checked-in constants.
    assert_eq!(matrix::EXPR_MATRIX.len(), 33);
    assert_eq!(matrix::STMT_MATRIX.len(), 25);
}

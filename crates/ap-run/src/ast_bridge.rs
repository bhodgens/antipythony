//! AST bridge (W114): `compile(src, fn, mode, PyCF_ONLY_AST)` support.
//!
//! The native side walks the ruff syntax tree (via `ap_parse`) and emits a
//! GENERIC node tree — each node is a Python dict:
//!   {"_type": "<NodeName>", "lineno": N, "col_offset": N,
//!    "end_lineno": N, "end_col_offset": N, "<field>": <value>, ...}
//! Child nodes are nested dicts; field lists become Python lists.
//!
//! The vendored `_ast.py` converts these dicts into real class instances
//! (the class hierarchy lives there, so `isinstance` works and
//! `ast.walk` / `_fields` operate as upstream).
//!
//! Coverage note (A-3 works-best-effort): the walker covers the statement
//! and expression set pytest's traceback source rendering needs; unknown
//! ruff variants degrade to an "Unimplemented*" node carrying the source
//! range (never a crash).

use ap_obj::dict::PyDict;
use ap_obj::int::Int;
use ap_obj::value::{PyList, PyString, Value};

type R = Result<Value, String>;

fn s(v: &str) -> Value {
    Value::Str(PyString::new(v.to_string()))
}

fn i(v: usize) -> Value {
    Value::Int(Int::from_i64(v as i64))
}

fn dict_of(pairs: Vec<(&str, Value)>) -> Value {
    let d = PyDict::new();
    for (k, v) in pairs {
        let _ = d.set(s(k), v);
    }
    Value::Dict(d)
}

fn list_of(items: Vec<Value>) -> Value {
    Value::List(PyList::from_vec(items))
}

fn opt<T>(
    v: &Option<T>,
    line_col: &dyn Fn(usize) -> (usize, usize),
    f: impl Fn(&T) -> R,
) -> R {
    match v {
        Some(x) => f(x),
        None => Ok(Value::None),
    }
}

/// `Ranged` -> the four CPython position fields.
fn pos_fields(
    range: (usize, usize),
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> Vec<(&'static str, Value)> {
    let (sl, sc) = line_col(range.0);
    let (el, ec) = line_col(range.1);
    vec![
        ("lineno", i(sl)),
        ("col_offset", i(sc)),
        ("end_lineno", i(el)),
        ("end_col_offset", i(ec)),
    ]
}

fn rng(node: &impl ruff_text_size::Ranged) -> (usize, usize) {
    (
        node.range().start().to_u32() as usize,
        node.range().end().to_u32() as usize,
    )
}

/// Entry point: source text -> generic node dict tree (Module root).
pub fn native_parse_tree(source: &str) -> Result<Value, String> {
    let parsed = ap_parse::parse(source).map_err(|e| e.message)?;
    let line_col = |off: usize| parsed.line_col(off);
    let module = parsed.module();
    let mut body: Vec<Value> = Vec::new();
    for st in &module.body {
        body.push(stmt_node(st, &line_col)?);
    }
    let mut fields = pos_fields((0, source.len()), &line_col);
    fields.push(("_type", s("Module")));
    fields.push(("body", list_of(body)));
    Ok(dict_of(fields))
}

fn stmts_node(stmts: &[ruff_python_ast::Stmt], line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut out = Vec::new();
    for st in stmts {
        out.push(stmt_node(st, line_col)?);
    }
    Ok(list_of(out))
}

fn stmt_node(st: &ruff_python_ast::Stmt, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(st), line_col);
    match st {
        ruff_python_ast::Stmt::Pass(_) => {
            f.push(("_type", s("Pass")));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Break(_) => {
            f.push(("_type", s("Break")));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Continue(_) => {
            f.push(("_type", s("Continue")));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Expr(e) => {
            f.push(("_type", s("Expr")));
            f.push(("value", expr_node(&e.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Return(e) => {
            f.push(("_type", s("Return")));
            f.push((
                "value",
                opt(&e.value, line_col, |v| expr_node(v, line_col))?,
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Assign(a) => {
            f.push(("_type", s("Assign")));
            let mut targets = Vec::new();
            for t in &a.targets {
                targets.push(expr_node(t, line_col)?);
            }
            f.push(("targets", list_of(targets)));
            f.push(("value", expr_node(&a.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::AugAssign(a) => {
            f.push(("_type", s("AugAssign")));
            f.push(("target", expr_node(&a.target, line_col)?));
            f.push(("op", s(operator_name(&a.op))));
            f.push(("value", expr_node(&a.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::AnnAssign(a) => {
            f.push(("_type", s("AnnAssign")));
            f.push(("target", expr_node(&a.target, line_col)?));
            f.push(("annotation", expr_node(&a.annotation, line_col)?));
            f.push((
                "value",
                opt(&a.value, line_col, |v| expr_node(v, line_col))?,
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Delete(d) => {
            f.push(("_type", s("Delete")));
            let mut targets = Vec::new();
            for t in &d.targets {
                targets.push(expr_node(t, line_col)?);
            }
            f.push(("targets", list_of(targets)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::If(x) => {
            f.push(("_type", s("If")));
            f.push(("test", expr_node(&x.test, line_col)?));
            f.push(("body", stmts_node(&x.body, line_col)?));
            // ruff collapses elif chains into ElifElseClause list.
            let mut orelse: Vec<Value> = Vec::new();
            for clause in &x.elif_else_clauses {
                orelse.push(elif_clause_node(clause, line_col)?);
            }
            f.push(("orelse", list_of(orelse)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::While(x) => {
            f.push(("_type", s("While")));
            f.push(("test", expr_node(&x.test, line_col)?));
            f.push(("body", stmts_node(&x.body, line_col)?));
            f.push(("orelse", stmts_node(&x.orelse, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::For(x) => {
            f.push(("_type", s("For")));
            f.push(("target", expr_node(&x.target, line_col)?));
            f.push(("iter", expr_node(&x.iter, line_col)?));
            f.push(("body", stmts_node(&x.body, line_col)?));
            f.push(("orelse", stmts_node(&x.orelse, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::FunctionDef(fd) => {
            let name = if fd.is_async {
                "AsyncFunctionDef"
            } else {
                "FunctionDef"
            };
            f.push(("_type", s(name)));
            f.push(("name", s(fd.name.as_str())));
            f.push(("body", stmts_node(&fd.body, line_col)?));
            let mut decorators = Vec::new();
            for d in &fd.decorator_list {
                decorators.push(expr_node(&d.expression, line_col)?);
            }
            f.push(("decorator_list", list_of(decorators)));
            f.push(("args", params_node(&fd.parameters, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::ClassDef(cd) => {
            f.push(("_type", s("ClassDef")));
            f.push(("name", s(cd.name.as_str())));
            f.push(("body", stmts_node(&cd.body, line_col)?));
            let mut decorators = Vec::new();
            for d in &cd.decorator_list {
                decorators.push(expr_node(&d.expression, line_col)?);
            }
            f.push(("decorator_list", list_of(decorators)));
            let mut bases: Vec<Value> = Vec::new();
            let mut keywords: Vec<Value> = Vec::new();
            if let Some(args) = &cd.arguments {
                for b in args.args.iter() {
                    bases.push(expr_node(b, line_col)?);
                }
                for k in args.keywords.iter() {
                    keywords.push(keyword_node(k, line_col)?);
                }
            }
            f.push(("bases", list_of(bases)));
            f.push(("keywords", list_of(keywords)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Raise(x) => {
            f.push(("_type", s("Raise")));
            f.push((
                "exc",
                opt(&x.exc, line_col, |v| expr_node(v, line_col))?,
            ));
            f.push((
                "cause",
                opt(&x.cause, line_col, |v| expr_node(v, line_col))?,
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Try(x) => {
            f.push(("_type", s("Try")));
            f.push(("body", stmts_node(&x.body, line_col)?));
            let mut handlers = Vec::new();
            for h in &x.handlers {
                handlers.push(except_handler_node(h, line_col)?);
            }
            f.push(("handlers", list_of(handlers)));
            f.push(("orelse", stmts_node(&x.orelse, line_col)?));
            f.push(("finalbody", stmts_node(&x.finalbody, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Assert(x) => {
            f.push(("_type", s("Assert")));
            f.push(("test", expr_node(&x.test, line_col)?));
            f.push((
                "msg",
                opt(&x.msg, line_col, |v| expr_node(v, line_col))?,
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Import(x) => {
            f.push(("_type", s("Import")));
            let mut names = Vec::new();
            for a in &x.names {
                names.push(alias_node(a, line_col)?);
            }
            f.push(("names", list_of(names)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::ImportFrom(x) => {
            f.push(("_type", s("ImportFrom")));
            f.push((
                "module",
                match &x.module {
                    Some(m) => s(m.as_str()),
                    None => Value::None,
                },
            ));
            f.push(("level", i(x.level as usize)));
            let mut names = Vec::new();
            for a in &x.names {
                names.push(alias_node(a, line_col)?);
            }
            f.push(("names", list_of(names)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Global(x) => {
            f.push(("_type", s("Global")));
            let mut names = Vec::new();
            for n in &x.names {
                names.push(s(n.as_str()));
            }
            f.push(("names", list_of(names)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Nonlocal(x) => {
            f.push(("_type", s("Nonlocal")));
            let mut names = Vec::new();
            for n in &x.names {
                names.push(s(n.as_str()));
            }
            f.push(("names", list_of(names)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::With(x) => {
            f.push(("_type", s("With")));
            let mut items = Vec::new();
            for it in &x.items {
                items.push(with_item_node(it, line_col)?);
            }
            f.push(("items", list_of(items)));
            f.push(("body", stmts_node(&x.body, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Match(x) => {
            f.push(("_type", s("Match")));
            f.push(("subject", expr_node(&x.subject, line_col)?));
            let mut cases = Vec::new();
            for c in &x.cases {
                cases.push(match_case_node(c, line_col)?);
            }
            f.push(("cases", list_of(cases)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::TypeAlias(_) | ruff_python_ast::Stmt::IpyEscapeCommand(_) => {
            f.push(("_type", s("UnimplementedStmt")));
            Ok(dict_of(f))
        }
    }
}

/// ruff collapses elif chains: `elif` clauses carry a test, the bare
/// `else` does not. Represent elif as a nested If (CPython shape);
/// a bare else becomes a marker the Python side inlines.
fn elif_clause_node(
    c: &ruff_python_ast::ElifElseClause,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut f = pos_fields(rng(c), line_col);
    match &c.test {
        Some(test) => {
            f.push(("_type", s("If")));
            f.push(("test", expr_node(test, line_col)?));
            f.push(("body", stmts_node(&c.body, line_col)?));
            f.push(("orelse", list_of(Vec::new())));
            Ok(dict_of(f))
        }
        None => {
            f.push(("_type", s("__ElseBlock__")));
            f.push(("body", stmts_node(&c.body, line_col)?));
            Ok(dict_of(f))
        }
    }
}

fn with_item_node(it: &ruff_python_ast::WithItem, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(it), line_col);
    f.push(("_type", s("withitem")));
    f.push(("context_expr", expr_node(&it.context_expr, line_col)?));
    f.push((
        "optional_vars",
        opt(&it.optional_vars, line_col, |v| expr_node(v, line_col))?,
    ));
    Ok(dict_of(f))
}

fn except_handler_node(
    h: &ruff_python_ast::ExceptHandler,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let ruff_python_ast::ExceptHandler::ExceptHandler(eh) = h;
    let mut f = pos_fields(rng(eh), line_col);
    f.push(("_type", s("ExceptHandler")));
    f.push((
        "type",
        match &eh.type_ {
            Some(t) => expr_node(t, line_col)?,
            None => Value::None,
        },
    ));
    f.push((
        "name",
        match &eh.name {
            Some(n) => s(n.as_str()),
            None => Value::None,
        },
    ));
    f.push(("body", stmts_node(&eh.body, line_col)?));
    Ok(dict_of(f))
}

fn alias_node(a: &ruff_python_ast::Alias, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(a), line_col);
    f.push(("_type", s("alias")));
    f.push(("name", s(a.name.as_str())));
    f.push((
        "asname",
        match &a.asname {
            Some(n) => s(n.as_str()),
            None => Value::None,
        },
    ));
    Ok(dict_of(f))
}

fn keyword_node(k: &ruff_python_ast::Keyword, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(k), line_col);
    f.push(("_type", s("keyword")));
    f.push((
        "arg",
        match &k.arg {
            Some(a) => s(a.as_str()),
            None => Value::None,
        },
    ));
    f.push(("value", expr_node(&k.value, line_col)?));
    Ok(dict_of(f))
}

fn match_case_node(
    c: &ruff_python_ast::MatchCase,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut f = pos_fields(rng(c), line_col);
    f.push(("_type", s("match_case")));
    f.push(("pattern", s("<pattern>")));
    f.push(("body", stmts_node(&c.body, line_col)?));
    Ok(dict_of(f))
}

fn arg_node(a: &ruff_python_ast::Parameter, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(a), line_col);
    f.push(("_type", s("arg")));
    f.push(("arg", s(a.name.as_str())));
    Ok(dict_of(f))
}

fn param_with_default_node(
    p: &ruff_python_ast::ParameterWithDefault,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    arg_node(&p.parameter, line_col)
}

fn params_node(p: &ruff_python_ast::Parameters, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(p), line_col);
    f.push(("_type", s("arguments")));
    let mut args = Vec::new();
    for a in p.posonlyargs.iter().chain(p.args.iter()) {
        args.push(param_with_default_node(a, line_col)?);
    }
    f.push(("args", list_of(args)));
    f.push((
        "vararg",
        opt(&p.vararg, line_col, |a| arg_node(a, line_col))?,
    ));
    let mut kwonly = Vec::new();
    for a in p.kwonlyargs.iter() {
        kwonly.push(param_with_default_node(a, line_col)?);
    }
    f.push(("kwonlyargs", list_of(kwonly)));
    f.push((
        "kwarg",
        opt(&p.kwarg, line_col, |a| arg_node(a, line_col))?,
    ));
    Ok(dict_of(f))
}

fn operator_name(op: &ruff_python_ast::Operator) -> &'static str {
    use ruff_python_ast::Operator as O;
    match op {
        O::Add => "Add",
        O::Sub => "Sub",
        O::Mult => "Mult",
        O::MatMult => "MatMult",
        O::Div => "Div",
        O::Mod => "Mod",
        O::Pow => "Pow",
        O::LShift => "LShift",
        O::RShift => "RShift",
        O::BitOr => "BitOr",
        O::BitXor => "BitXor",
        O::BitAnd => "BitAnd",
        O::FloorDiv => "FloorDiv",
    }
}

fn cmpop_name(op: &ruff_python_ast::CmpOp) -> &'static str {
    use ruff_python_ast::CmpOp as C;
    match op {
        C::Eq => "Eq",
        C::NotEq => "NotEq",
        C::Lt => "Lt",
        C::LtE => "LtE",
        C::Gt => "Gt",
        C::GtE => "GtE",
        C::Is => "Is",
        C::IsNot => "IsNot",
        C::In => "In",
        C::NotIn => "NotIn",
    }
}

fn unaryop_name(op: &ruff_python_ast::UnaryOp) -> &'static str {
    use ruff_python_ast::UnaryOp as U;
    match op {
        U::Invert => "Invert",
        U::Not => "Not",
        U::UAdd => "UAdd",
        U::USub => "USub",
    }
}

fn ctx_name(ctx: &ruff_python_ast::ExprContext) -> &'static str {
    use ruff_python_ast::ExprContext as C;
    match ctx {
        C::Load => "Load",
        C::Store => "Store",
        C::Del => "Del",
        C::Invalid => "Load",
    }
}

fn boolop_name(op: &ruff_python_ast::BoolOp) -> &'static str {
    use ruff_python_ast::BoolOp as B;
    match op {
        B::And => "And",
        B::Or => "Or",
    }
}

fn expr_node(e: &ruff_python_ast::Expr, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(e), line_col);
    match e {
        ruff_python_ast::Expr::Name(n) => {
            f.push(("_type", s("Name")));
            f.push(("id", s(n.id.as_str())));
            f.push(("ctx", s(ctx_name(&n.ctx))));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::NumberLiteral(n) => {
            f.push(("_type", s("Constant")));
            match &n.value {
                ruff_python_ast::Number::Int(v) => {
                    f.push(("value", i(v.as_u64().unwrap_or(0) as usize)));
                }
                ruff_python_ast::Number::Float(v) => {
                    f.push(("value", Value::Float(*v)));
                }
                ruff_python_ast::Number::Complex { real, imag } => {
                    f.push(("value", s(&format!("{}j{}", real, imag))));
                }
            }
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::BooleanLiteral(b) => {
            f.push(("_type", s("Constant")));
            f.push(("value", Value::Bool(b.value)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::NoneLiteral(_) => {
            f.push(("_type", s("Constant")));
            f.push(("value", Value::None));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::EllipsisLiteral(_) => {
            f.push(("_type", s("Constant")));
            f.push(("value", s("Ellipsis")));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::StringLiteral(v) => {
            f.push(("_type", s("Constant")));
            f.push(("value", s(v.value.to_str())));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::BytesLiteral(v) => {
            f.push(("_type", s("Constant")));
            // best-effort: join the byte parts into a repr string the
            // Python side can decode
            let mut all: Vec<u8> = Vec::new();
            for part in v.value.as_slice() {
                all.extend_from_slice(part.as_slice());
            }
            f.push(("value", s(&format!("__bytes__:{:?}", all))));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::BinOp(x) => {
            f.push(("_type", s("BinOp")));
            f.push(("left", expr_node(&x.left, line_col)?));
            f.push(("op", s(operator_name(&x.op))));
            f.push(("right", expr_node(&x.right, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::UnaryOp(x) => {
            f.push(("_type", s("UnaryOp")));
            f.push(("op", s(unaryop_name(&x.op))));
            f.push(("operand", expr_node(&x.operand, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::BoolOp(x) => {
            f.push(("_type", s("BoolOp")));
            f.push(("op", s(boolop_name(&x.op))));
            let mut values = Vec::new();
            for v in &x.values {
                values.push(expr_node(v, line_col)?);
            }
            f.push(("values", list_of(values)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Compare(x) => {
            f.push(("_type", s("Compare")));
            f.push(("left", expr_node(&x.left, line_col)?));
            let mut ops = Vec::new();
            for op in x.ops.iter() {
                ops.push(s(cmpop_name(op)));
            }
            f.push(("ops", list_of(ops)));
            let mut comparators = Vec::new();
            for c in x.comparators.iter() {
                comparators.push(expr_node(c, line_col)?);
            }
            f.push(("comparators", list_of(comparators)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Call(x) => {
            f.push(("_type", s("Call")));
            f.push(("func", expr_node(&x.func, line_col)?));
            let mut args = Vec::new();
            for a in x.arguments.args.iter() {
                args.push(expr_node(a, line_col)?);
            }
            f.push(("args", list_of(args)));
            let mut kws = Vec::new();
            for k in x.arguments.keywords.iter() {
                kws.push(keyword_node(k, line_col)?);
            }
            f.push(("keywords", list_of(kws)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Attribute(x) => {
            f.push(("_type", s("Attribute")));
            f.push(("value", expr_node(&x.value, line_col)?));
            f.push(("attr", s(x.attr.as_str())));
            f.push(("ctx", s(ctx_name(&x.ctx))));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Subscript(x) => {
            f.push(("_type", s("Subscript")));
            f.push(("value", expr_node(&x.value, line_col)?));
            f.push(("slice", expr_node(&x.slice, line_col)?));
            f.push(("ctx", s(ctx_name(&x.ctx))));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::List(x) => {
            f.push(("_type", s("List")));
            f.push(("ctx", s(ctx_name(&x.ctx))));
            let mut elts = Vec::new();
            for e2 in &x.elts {
                elts.push(expr_node(e2, line_col)?);
            }
            f.push(("elts", list_of(elts)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Tuple(x) => {
            f.push(("_type", s("Tuple")));
            f.push(("ctx", s(ctx_name(&x.ctx))));
            let mut elts = Vec::new();
            for e2 in &x.elts {
                elts.push(expr_node(e2, line_col)?);
            }
            f.push(("elts", list_of(elts)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Set(x) => {
            f.push(("_type", s("Set")));
            let mut elts = Vec::new();
            for e2 in &x.elts {
                elts.push(expr_node(e2, line_col)?);
            }
            f.push(("elts", list_of(elts)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Dict(x) => {
            f.push(("_type", s("Dict")));
            let mut keys = Vec::new();
            let mut values = Vec::new();
            for item in &x.items {
                keys.push(match &item.key {
                    Some(k) => expr_node(k, line_col)?,
                    None => Value::None,
                });
                values.push(expr_node(&item.value, line_col)?);
            }
            f.push(("keys", list_of(keys)));
            f.push(("values", list_of(values)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Starred(x) => {
            f.push(("_type", s("Starred")));
            f.push(("value", expr_node(&x.value, line_col)?));
            f.push(("ctx", s(ctx_name(&x.ctx))));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::If(x) => {
            f.push(("_type", s("IfExp")));
            f.push(("test", expr_node(&x.test, line_col)?));
            f.push(("body", expr_node(&x.body, line_col)?));
            f.push(("orelse", expr_node(&x.orelse, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Named(x) => {
            f.push(("_type", s("NamedExpr")));
            f.push(("target", expr_node(&x.target, line_col)?));
            f.push(("value", expr_node(&x.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Await(x) => {
            f.push(("_type", s("Await")));
            f.push(("value", expr_node(&x.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Yield(x) => {
            f.push(("_type", s("Yield")));
            f.push((
                "value",
                opt(&x.value, line_col, |v| expr_node(v, line_col))?,
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::YieldFrom(x) => {
            f.push(("_type", s("YieldFrom")));
            f.push(("value", expr_node(&x.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Lambda(x) => {
            f.push(("_type", s("Lambda")));
            let lambda_params = x
                .parameters
                .as_ref()
                .map(|p| p.as_ref())
                .expect("lambda parameters");
            f.push(("args", params_node(lambda_params, line_col)?));
            f.push(("body", expr_node(&x.body, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::ListComp(x) => {
            f.push(("_type", s("ListComp")));
            f.push(("elt", expr_node(&x.elt, line_col)?));
            f.push(("generators", comprehension_list(&x.generators, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::SetComp(x) => {
            f.push(("_type", s("SetComp")));
            f.push(("elt", expr_node(&x.elt, line_col)?));
            f.push(("generators", comprehension_list(&x.generators, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::DictComp(x) => {
            f.push(("_type", s("DictComp")));
            f.push((
                "key",
                match &x.key {
                    Some(k) => expr_node(k, line_col)?,
                    None => Value::None,
                },
            ));
            f.push(("value", expr_node(&x.value, line_col)?));
            f.push(("generators", comprehension_list(&x.generators, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Generator(x) => {
            f.push(("_type", s("GeneratorExp")));
            f.push(("elt", expr_node(&x.elt, line_col)?));
            f.push(("generators", comprehension_list(&x.generators, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::Slice(x) => {
            f.push(("_type", s("Slice")));
            f.push(("lower", opt(&x.lower, line_col, |v| expr_node(v, line_col))?));
            f.push(("upper", opt(&x.upper, line_col, |v| expr_node(v, line_col))?));
            f.push(("step", opt(&x.step, line_col, |v| expr_node(v, line_col))?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::FString(x) => {
            // CPython shape: JoinedStr(values=[Constant(literal) |
            // FormattedValue(value, conversion, format_spec), ...])
            f.push(("_type", s("JoinedStr")));
            let mut values: Vec<Value> = Vec::new();
            for part in x.value.as_slice() {
                match part {
                    ruff_python_ast::FStringPart::Literal(lit) => {
                        let mut pf = pos_fields(rng(lit), line_col);
                        pf.push(("_type", s("Constant")));
                        pf.push(("value", s(&lit.value)));
                        values.push(dict_of(pf));
                    }
                    ruff_python_ast::FStringPart::FString(fs) => {
                        for el in fs.elements.iter() {
                            values.push(fstring_element_node(el, line_col)?);
                        }
                    }
                }
            }
            f.push(("values", list_of(values)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::TString(_) | ruff_python_ast::Expr::IpyEscapeCommand(_) => {
            f.push(("_type", s("UnimplementedExpr")));
            Ok(dict_of(f))
        }
    }
}

fn comprehension_node(
    c: &ruff_python_ast::Comprehension,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut f = pos_fields(rng(c), line_col);
    f.push(("_type", s("comprehension")));
    f.push(("target", expr_node(&c.target, line_col)?));
    f.push(("iter", expr_node(&c.iter, line_col)?));
    let mut ifs = Vec::new();
    for e in &c.ifs {
        ifs.push(expr_node(e, line_col)?);
    }
    f.push(("ifs", list_of(ifs)));
    f.push(("is_async", Value::Bool(c.is_async)));
    Ok(dict_of(f))
}

fn comprehension_list(
    gens: &[ruff_python_ast::Comprehension],
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut out = Vec::new();
    for c in gens {
        out.push(comprehension_node(c, line_col)?);
    }
    Ok(list_of(out))
}

fn fstring_element_node(
    el: &ruff_python_ast::InterpolatedStringElement,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    match el {
        ruff_python_ast::InterpolatedStringElement::Literal(lit) => {
            let mut f = pos_fields(rng(lit), line_col);
            f.push(("_type", s("Constant")));
            f.push(("value", s(&lit.value)));
            Ok(dict_of(f))
        }
        ruff_python_ast::InterpolatedStringElement::Interpolation(ip) => {
            let mut f = pos_fields(rng(ip), line_col);
            f.push(("_type", s("FormattedValue")));
            f.push(("value", expr_node(&ip.expression, line_col)?));
            // conversion: -1 (none) or the ord of the conversion char
            let conv: i64 = match ip.conversion {
                ruff_python_ast::ConversionFlag::None => -1,
                ruff_python_ast::ConversionFlag::Str => 115,
                ruff_python_ast::ConversionFlag::Ascii => 97,
                ruff_python_ast::ConversionFlag::Repr => 114,
            };
            f.push(("conversion", Value::Int(ap_obj::int::Int::from_i64(conv))));
            f.push((
                "format_spec",
                match &ip.format_spec {
                    Some(spec) => {
                        // CPython: JoinedStr of the spec elements
                        let mut spec_values: Vec<Value> = Vec::new();
                        for el2 in spec.elements.iter() {
                            spec_values.push(fstring_element_node(el2, line_col)?);
                        }
                        dict_of(vec![
                            ("_type", s("JoinedStr")),
                            ("values", list_of(spec_values)),
                        ])
                    }
                    None => Value::None,
                },
            ));
            Ok(dict_of(f))
        }
    }
}

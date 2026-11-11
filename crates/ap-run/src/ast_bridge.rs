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
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Value};

type R = Result<Value, String>;

fn s(v: &str) -> Value {
    Value::Str(PyString::new(v.to_string()))
}

// 02.09a P3: BRIDGE_SOURCE moved to the RuntimeState table
// (runtime_state.rs::BRIDGE_SOURCE_T). The thread_local let a worker
// AST-bridging a module clobber main's staged source mid-run (audit row:
// ast_bridge.rs:32, class (c)).

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

fn opt<T>(v: &Option<T>, _line_col: &dyn Fn(usize) -> (usize, usize), f: impl Fn(&T) -> R) -> R {
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
    {
        use crate::runtime_state as rs;
        rs::with_active_mut(rs::BRIDGE_SOURCE_T_acc(), |src| *src = source.to_string());
    }
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
            // CPython simple: 1 when the target is a plain unparenthesized
            // Name (`x: int` and `x: int = 1` are both simple=1); 0 for
            // attribute/subscript/tuple targets. ruff computes this exact
            // flag; it dumps as int 0/1 (native AST stores ints).
            let simple = a.simple;
            f.push((
                "simple",
                Value::Int(ap_obj::int::Int::from_i64(simple as i64)),
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
            // ruff collapses elif chains into a FLAT ElifElseClause
            // list: [elif(test), elif(test), else]. Rebuild the
            // CPython nested shape right-to-left: the LAST else
            // clause's body becomes orelse; each preceding elif
            // becomes an If whose orelse holds the next.
            let clauses = &x.elif_else_clauses;
            let mut orelse: Vec<Value> = Vec::new();
            if let Some(last) = clauses.last() {
                if last.test.is_none() {
                    orelse = match stmts_node(&last.body, line_col)? {
                        Value::List(l) => l.items().to_vec(),
                        other => vec![other],
                    };
                }
            }
            let elifs: Vec<&ruff_python_ast::ElifElseClause> =
                clauses.iter().filter(|c| c.test.is_some()).collect();
            for clause in elifs.iter().rev() {
                let mut cf = pos_fields(rng(clause), line_col);
                cf.push(("_type", s("If")));
                cf.push(("test", expr_node(clause.test.as_ref().unwrap(), line_col)?));
                cf.push(("body", stmts_node(&clause.body, line_col)?));
                cf.push(("orelse", list_of(orelse)));
                orelse = vec![dict_of(cf)];
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
            f.push(("_type", if x.is_async { s("AsyncFor") } else { s("For") }));
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
            if let Some(tp) = &fd.type_params {
                f.push(("type_params", type_params_list(tp, line_col)?));
            }
            f.push(("args", params_node(&fd.parameters, line_col)?));
            f.push((
                "returns",
                opt(&fd.returns, line_col, |v| expr_node(v, line_col))?,
            ));
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
            if let Some(tp) = &cd.type_params {
                f.push(("type_params", type_params_list(tp, line_col)?));
            }
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::Raise(x) => {
            f.push(("_type", s("Raise")));
            f.push(("exc", opt(&x.exc, line_col, |v| expr_node(v, line_col))?));
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
            f.push(("msg", opt(&x.msg, line_col, |v| expr_node(v, line_col))?));
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
            f.push((
                "_type",
                if x.is_async {
                    s("AsyncWith")
                } else {
                    s("With")
                },
            ));
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
        ruff_python_ast::Stmt::TypeAlias(x) => {
            f.push(("_type", s("TypeAlias")));
            f.push(("name", expr_node(&x.name, line_col)?));
            if let Some(tp) = &x.type_params {
                f.push(("type_params", type_params_list(tp, line_col)?));
            }
            f.push(("value", expr_node(&x.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Stmt::IpyEscapeCommand(_) => {
            f.push(("_type", s("UnimplementedStmt")));
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
    f.push(("pattern", pattern_node(&c.pattern, line_col)?));
    f.push((
        "guard",
        opt(&c.guard, line_col, |g| expr_node(g, line_col))?,
    ));
    f.push(("body", stmts_node(&c.body, line_col)?));
    Ok(dict_of(f))
}

fn pattern_node(p: &ruff_python_ast::Pattern, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(p), line_col);
    match p {
        ruff_python_ast::Pattern::MatchValue(x) => {
            f.push(("_type", s("MatchValue")));
            f.push(("value", expr_node(&x.value, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchSingleton(x) => {
            f.push(("_type", s("MatchSingleton")));
            let v = match x.value {
                ruff_python_ast::Singleton::None => Value::None,
                ruff_python_ast::Singleton::True => Value::Bool(true),
                ruff_python_ast::Singleton::False => Value::Bool(false),
            };
            f.push(("value", v));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchSequence(x) => {
            f.push(("_type", s("MatchSequence")));
            f.push(("patterns", pattern_list(&x.patterns, line_col)?));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchMapping(x) => {
            f.push(("_type", s("MatchMapping")));
            let mut keys = Vec::new();
            for k in x.keys.iter() {
                keys.push(expr_node(k, line_col)?);
            }
            f.push(("keys", list_of(keys)));
            f.push(("patterns", pattern_list(&x.patterns, line_col)?));
            f.push((
                "rest",
                match &x.rest {
                    Some(n) => s(n.as_str()),
                    None => Value::None,
                },
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchClass(x) => {
            f.push(("_type", s("MatchClass")));
            f.push(("cls", expr_node(&x.cls, line_col)?));
            let mut patterns = Vec::new();
            for pt in x.arguments.patterns.iter() {
                patterns.push(pattern_node(pt, line_col)?);
            }
            f.push(("patterns", list_of(patterns)));
            let mut kwd_attrs = Vec::new();
            let mut kwd_patterns = Vec::new();
            for k in x.arguments.keywords.iter() {
                kwd_attrs.push(s(k.attr.as_str()));
                kwd_patterns.push(pattern_node(&k.pattern, line_col)?);
            }
            f.push(("kwd_attrs", list_of(kwd_attrs)));
            f.push(("kwd_patterns", list_of(kwd_patterns)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchStar(x) => {
            f.push(("_type", s("MatchStar")));
            f.push((
                "name",
                match &x.name {
                    Some(n) => s(n.as_str()),
                    None => Value::None,
                },
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchAs(x) => {
            f.push(("_type", s("MatchAs")));
            f.push((
                "pattern",
                opt(&x.pattern, line_col, |p2| pattern_node(p2, line_col))?,
            ));
            f.push((
                "name",
                match &x.name {
                    Some(n) => s(n.as_str()),
                    None => Value::None,
                },
            ));
            Ok(dict_of(f))
        }
        ruff_python_ast::Pattern::MatchOr(x) => {
            f.push(("_type", s("MatchOr")));
            f.push(("patterns", pattern_list(&x.patterns, line_col)?));
            Ok(dict_of(f))
        }
    }
}

fn pattern_list(ps: &[ruff_python_ast::Pattern], line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut out = Vec::new();
    for p in ps {
        out.push(pattern_node(p, line_col)?);
    }
    Ok(list_of(out))
}

fn arg_node(a: &ruff_python_ast::Parameter, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(a), line_col);
    f.push(("_type", s("arg")));
    f.push(("arg", s(a.name.as_str())));
    f.push((
        "annotation",
        opt(&a.annotation, line_col, |v| expr_node(v, line_col))?,
    ));
    Ok(dict_of(f))
}

fn param_with_default_node(
    p: &ruff_python_ast::ParameterWithDefault,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    // the default itself is emitted via arguments.defaults
    // (CPython shape); do NOT duplicate it on the arg node.
    arg_node(&p.parameter, line_col)
}

fn params_node(p: &ruff_python_ast::Parameters, line_col: &dyn Fn(usize) -> (usize, usize)) -> R {
    let mut f = pos_fields(rng(p), line_col);
    f.push(("_type", s("arguments")));
    let mut posonly = Vec::new();
    for a in p.posonlyargs.iter() {
        posonly.push(param_with_default_node(a, line_col)?);
    }
    f.push(("posonlyargs", list_of(posonly)));
    let mut args = Vec::new();
    for a in p.args.iter() {
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
    f.push(("kwarg", opt(&p.kwarg, line_col, |a| arg_node(a, line_col))?));
    // CPython arguments._fields carries defaults (right-aligned
    // over posonly+args) and kw_defaults (None-padded per kwonly)
    let mut defaults = Vec::new();
    for a in p.posonlyargs.iter().chain(p.args.iter()) {
        if let Some(d) = &a.default {
            defaults.push(expr_node(d, line_col)?);
        }
    }
    f.push(("defaults", list_of(defaults)));
    let mut kw_defaults = Vec::new();
    for a in p.kwonlyargs.iter() {
        kw_defaults.push(match &a.default {
            Some(d) => expr_node(d, line_col)?,
            None => Value::None,
        });
    }
    f.push(("kw_defaults", list_of(kw_defaults)));
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
                    // BigInt literals (e.g. 0xffffffffffff0fff) do
                    // not fit i64 - build via decimal parse so the
                    // full magnitude survives
                    let digits = v.to_string();
                    match ap_obj::int::Int::parse_base10(&digits) {
                        Ok(big) => f.push(("value", Value::Int(big))),
                        Err(()) => f.push(("value", i(v.as_u64().unwrap_or(0) as usize))),
                    }
                }
                ruff_python_ast::Number::Float(v) => {
                    f.push(("value", Value::Float(*v)));
                }
                ruff_python_ast::Number::Complex { real, imag } => {
                    // CPython AST: complex literals are Constant nodes whose
                    // value is a real complex object (repr "1j", "(1+2j)").
                    f.push((
                        "value",
                        crate::builtins_modules::make_complex_value(*real, *imag),
                    ));
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
            f.push(("value", Value::Ellipsis));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::StringLiteral(v) => {
            f.push(("_type", s("Constant")));
            f.push(("value", s(v.value.to_str())));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::BytesLiteral(v) => {
            f.push(("_type", s("Constant")));
            let mut all: Vec<u8> = Vec::new();
            for part in v.value.as_slice() {
                all.extend_from_slice(part.as_slice());
            }
            f.push(("value", Value::Bytes(Gc::new(all))));
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
            match x.parameters.as_ref() {
                Some(p) => f.push(("args", params_node(p, line_col)?)),
                None => f.push((
                    "args",
                    dict_of(vec![
                        ("_type", s("arguments")),
                        ("args", list_of(Vec::new())),
                        ("vararg", Value::None),
                        ("kwonlyargs", list_of(Vec::new())),
                        ("kwarg", Value::None),
                    ]),
                )),
            }
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
            f.push((
                "lower",
                opt(&x.lower, line_col, |v| expr_node(v, line_col))?,
            ));
            f.push((
                "upper",
                opt(&x.upper, line_col, |v| expr_node(v, line_col))?,
            ));
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
                        // CPython drops empty-string literals when
                        // building JoinedStr values
                        if !lit.value.is_empty() {
                            let mut pf = pos_fields(rng(lit), line_col);
                            pf.push(("_type", s("Constant")));
                            pf.push(("value", s(&lit.value)));
                            values.push(dict_of(pf));
                        }
                    }
                    ruff_python_ast::FStringPart::FString(fs) => {
                        for el in fs.elements.iter() {
                            let v = fstring_element_node(el, &mut values, line_col)?;
                            values.push(v);
                        }
                    }
                }
            }
            f.push(("values", list_of(values)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::TString(x) => {
            // PEP 750 (CPython 3.14) shape:
            // TemplateStr(values=[Constant(literal) |
            // Interpolation(value, str, conversion, format_spec), ...])
            f.push(("_type", s("TemplateStr")));
            let mut values: Vec<Value> = Vec::new();
            for tstr in x.value.as_slice() {
                for el in tstr.elements.iter() {
                    match tstring_element_node(el, &mut values, line_col)? {
                        Value::None => {} // skip empty literal parts
                        v => values.push(v),
                    }
                }
            }
            f.push(("values", list_of(values)));
            Ok(dict_of(f))
        }
        ruff_python_ast::Expr::IpyEscapeCommand(_) => {
            f.push(("_type", s("UnimplementedExpr")));
            Ok(dict_of(f))
        }
    }
}

fn type_param_node(
    tp: &ruff_python_ast::TypeParam,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut f = pos_fields(rng(tp), line_col);
    let (kind, name, bound) = match tp {
        ruff_python_ast::TypeParam::TypeVar(tv) => ("TypeVar", tv.name.as_str(), &tv.bound),
        ruff_python_ast::TypeParam::ParamSpec(ps) => ("ParamSpec", ps.name.as_str(), &None),
        ruff_python_ast::TypeParam::TypeVarTuple(tvt) => {
            ("TypeVarTuple", tvt.name.as_str(), &tvt.default)
        }
    };
    f.push(("_type", s(kind)));
    f.push(("name", s(name)));
    if let Some(b) = bound {
        // ruff TypeVar.bound is Box<Expr> (single) or Tuple of exprs
        f.push(("bound", expr_node(b, line_col)?));
    }
    Ok(dict_of(f))
}

fn type_params_list(
    tp: &ruff_python_ast::TypeParams,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    let mut out = Vec::new();
    for t in tp.type_params.iter() {
        out.push(type_param_node(t, line_col)?);
    }
    Ok(list_of(out))
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
    f.push(("is_async", Value::Int(Int::from_i64(c.is_async as i64))));
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
    values: &mut Vec<Value>,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    match el {
        ruff_python_ast::InterpolatedStringElement::Literal(lit) => {
            if lit.value.is_empty() {
                return Ok(Value::None);
            }
            let mut f = pos_fields(rng(lit), line_col);
            f.push(("_type", s("Constant")));
            f.push(("value", s(&lit.value)));
            Ok(dict_of(f))
        }
        ruff_python_ast::InterpolatedStringElement::Interpolation(ip) => {
            // Self-documenting f-string interpolations ({x=}): CPython 3.14
            // emits the debug text ("x=" / "x = ") as a Constant BEFORE the
            // FormattedValue, same as the t-string arm.
            if let Some(dt) = &ip.debug_text {
                let mut cf = pos_fields(rng(ip), line_col);
                cf.push(("_type", s("Constant")));
                cf.push(("value", s(dt.as_str())));
                values.push(dict_of(cf));
            }
            let mut f = pos_fields(rng(ip), line_col);
            f.push(("_type", s("FormattedValue")));
            f.push(("value", expr_node(&ip.expression, line_col)?));
            // conversion: -1 (none) or the ord of the conversion char
            let mut conv: i64 = match ip.conversion {
                ruff_python_ast::ConversionFlag::None => -1,
                ruff_python_ast::ConversionFlag::Str => 115,
                ruff_python_ast::ConversionFlag::Ascii => 97,
                ruff_python_ast::ConversionFlag::Repr => 114,
            };
            // Self-documenting f-string interpolations ({x=}) without an
            // explicit conversion or format spec report repr (!r, 114) in
            // CPython 3.14 — same rule as the t-string arm below.
            if conv == -1 && ip.debug_text.is_some() && ip.format_spec.is_none() {
                conv = 114;
            }
            f.push(("conversion", Value::Int(ap_obj::int::Int::from_i64(conv))));
            f.push((
                "format_spec",
                match &ip.format_spec {
                    Some(spec) => {
                        // CPython: JoinedStr of the spec elements
                        let mut spec_values: Vec<Value> = Vec::new();
                        for el2 in spec.elements.iter() {
                            spec_values.push(fstring_element_node(el2, values, line_col)?);
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

/// One element of a t-string -> CPython 3.14 Interpolation or Constant.
/// Empty literal parts return Value::None; self-documenting
/// interpolations additionally push a debug-text Constant into
/// `values` (CPython 3.14 emits it before the Interpolation).
fn tstring_element_node(
    el: &ruff_python_ast::InterpolatedStringElement,
    values: &mut Vec<Value>,
    line_col: &dyn Fn(usize) -> (usize, usize),
) -> R {
    match el {
        ruff_python_ast::InterpolatedStringElement::Literal(lit) => {
            if lit.value.is_empty() {
                return Ok(Value::None);
            }
            let mut f = pos_fields(rng(lit), line_col);
            f.push(("_type", s("Constant")));
            f.push(("value", s(&lit.value)));
            Ok(dict_of(f))
        }
        ruff_python_ast::InterpolatedStringElement::Interpolation(ip) => {
            let mut f = pos_fields(rng(ip), line_col);
            f.push(("_type", s("Interpolation")));
            // CPython 3.14: "str" holds the raw source text of the
            // expression inside the braces (leading whitespace kept,
            // nothing after the expression). For self-documenting
            // interpolations (t"{x=}") ruff stores the full text in
            // debug_text; CPython also emits a Constant holding that
            // debug text ("x=" / "x = ") before the Interpolation.
            let (estart, eend) = rng(ip.expression.as_ref());
            let (istart, iend) = rng(ip);
            let raw = match &ip.debug_text {
                Some(dt) => {
                    let dbg = dt.as_str().to_string();
                    let mut cf = pos_fields(rng(ip), line_col);
                    cf.push(("_type", s("Constant")));
                    cf.push(("value", s(&dbg)));
                    values.push(dict_of(cf));
                    format!("{}{}", dt.leading(), dt.expression())
                }
                None => {
                    use crate::runtime_state as rs;
                    rs::with_active(rs::BRIDGE_SOURCE_T_acc(), |src| {
                        let src = src.as_str();
                        src.get(istart + 1..estart.min(iend))
                            .unwrap_or("")
                            .to_string()
                            + src.get(estart..eend).unwrap_or("")
                    })
                }
            };
            f.push(("str", s(&raw)));
            // conversion: -1 (none) or the ord of the conversion char.
            // Self-documenting interpolations without an explicit
            // conversion or format spec report repr (!r, 114) in
            // CPython 3.14; with a format spec they stay -1.
            let mut conv: i64 = match ip.conversion {
                ruff_python_ast::ConversionFlag::None => -1,
                ruff_python_ast::ConversionFlag::Str => 115,
                ruff_python_ast::ConversionFlag::Ascii => 97,
                ruff_python_ast::ConversionFlag::Repr => 114,
            };
            if conv == -1 && ip.debug_text.is_some() && ip.format_spec.is_none() {
                conv = 114;
            }
            f.push(("conversion", Value::Int(ap_obj::int::Int::from_i64(conv))));
            f.push((
                "format_spec",
                match &ip.format_spec {
                    Some(spec) => {
                        // CPython: JoinedStr of the spec elements
                        // (nested FormattedValue shape for interpolations)
                        let mut spec_values: Vec<Value> = Vec::new();
                        for el2 in spec.elements.iter() {
                            match fstring_element_node(el2, values, line_col)? {
                                Value::None => {}
                                v => spec_values.push(v),
                            }
                        }
                        dict_of(vec![
                            ("_type", s("JoinedStr")),
                            ("values", list_of(spec_values)),
                        ])
                    }
                    None => Value::None,
                },
            ));
            f.push(("value", expr_node(&ip.expression, line_col)?));
            Ok(dict_of(f))
        }
    }
}

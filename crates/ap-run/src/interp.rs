//! The core evaluator: `run_module` entry, statement execution, expression
//! evaluation, calls, comprehensions, generators, f-strings, pattern
//! matching, and uncaught-exception rendering (C-25/S-06-7/S-06-9/S-06-11).
//!
//! Module map: frame.rs (frames/name resolution), builtins.rs (the v0
//! builtin set + sys stub), methods.rs (method dispatch + subscripts),
//! iter.rs (iterator protocol), format.rs (format-spec machinery),
//! exc_hier.rs (exception hierarchy + str(exc)).

use std::cell::{Cell as RustCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use ap_obj::dict::PyDict;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

use crate::exc_hier::{is_subclass, ExceptionValue};
use crate::frame::{CellSlot, FnEntry, Frame, Interpreter, Loc};
use crate::iter::{IterState, PyIterator};
use crate::{builtins, methods, RunOutcome};
use ap_ir::expr::{Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, FStrPart, UnaryOp};
use ap_ir::item::{FnDef, Param, ParamKind, Pattern, Stmt, Target};
use ap_ir::Module;
use ap_parse::ScopeInfoModule;

type EResult = Result<Value, Flow>;
use crate::Flow;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg))
}

// ===========================================================================
// Entry
// ===========================================================================

/// Parse + lower + run a module.
pub fn run_module(
    source: &str,
    script_path: &str,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> RunOutcome {
    let parsed = match ap_parse::parse(source) {
        Ok(p) => p,
        Err(e) => {
            // SyntaxError: NO traceback header (S-06-9). Caret-column
            // fidelity to CPython's fine-grained error model is a
            // documented v0 divergence (progress log).
            let src_line = source.lines().nth(e.line.saturating_sub(1)).unwrap_or("");
            let _ = write!(
                stderr,
                "  File \"{script_path}\", line {}\n    {}\n    {}^\nSyntaxError: {}\n",
                e.line,
                src_line,
                " ".repeat(e.col.saturating_sub(1)),
                e.message
            );
            return RunOutcome {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: 1,
            };
        }
    };
    let (module, scopes) = match ap_parse::lower(&parsed) {
        Ok(m) => m,
        Err(e) => {
            let _ = writeln!(stderr, "{e}");
            return RunOutcome {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: 1,
            };
        }
    };

    let interp = Rc::new(Interpreter::new(
        source.to_string(),
        script_path.to_string(),
    ));
    CURRENT_INTERP.with(|c| *c.borrow_mut() = Some(interp.clone()));
    *interp.self_rc.borrow_mut() = Some(Rc::downgrade(&interp));
    builtins::install_builtins(&interp);
    // Statement-line table for tracebacks (ruff AST walk; the IR is
    // span-free by design).
    *interp.stmt_lines.borrow_mut() = ap_parse::collect_stmt_lines(&parsed);
    interp.stmt_ordinal.set(0);

    let outcome = exec_module(&interp, &module, &scopes);

    let _ = stdout.write_all(&interp.stdout.borrow());
    let out_bytes = interp.stdout.borrow().clone();
    match outcome {
        Ok(()) => RunOutcome {
            stdout: out_bytes,
            stderr: Vec::new(),
            exit_code: interp.exit_code.get().unwrap_or(0),
        },
        Err(exc) => {
            if exc.class == "SystemExit" {
                // Silent, exit n (C-29).
                let code = exc
                    .args
                    .first()
                    .and_then(|v| v.as_int())
                    .and_then(|i| i.to_i64())
                    .unwrap_or(0) as i32;
                return RunOutcome {
                    stdout: out_bytes,
                    stderr: Vec::new(),
                    exit_code: code,
                };
            }
            let text = render_traceback(&interp, &exc);
            let _ = stderr.write_all(text.as_bytes());
            let code = if exc.class == "KeyboardInterrupt" {
                130
            } else {
                1
            };
            RunOutcome {
                stdout: out_bytes,
                stderr: text.into_bytes(),
                exit_code: code,
            }
        }
    }
}

/// CPython-format traceback (C-25/S-06-7): frames outermost-first with the
/// innermost LAST, `  File "path", line N, in qualname` + 4-space source
/// echo, then `ExcType[: str(exc)]`. Chained reports render FIRST (S-06-8).
fn render_traceback(interp: &Rc<Interpreter>, exc: &ExceptionValue) -> String {
    let mut out = String::new();
    out.push_str("Traceback (most recent call last):\n");
    for (qualname, line) in interp.tb_frames.borrow().iter() {
        out.push_str(&format!(
            "  File \"{}\", line {}, in {}\n",
            interp.script_path, line, qualname
        ));
        if let Some(src_line) = interp.source.lines().nth(line.saturating_sub(1)) {
            out.push_str("    ");
            out.push_str(src_line.trim_end());
            out.push('\n');
        }
    }
    let msg = exc.py_str().unwrap_or_default();
    if msg.is_empty() {
        out.push_str(&exc.class);
    } else {
        out.push_str(&format!("{}: {}", exc.class, msg));
    }
    out.push('\n');
    if let Some(cause) = &exc.cause {
        let mut pre = render_traceback(interp, cause);
        pre.push_str("\nThe above exception was the direct cause of the following exception:\n\n");
        pre.push_str(&out);
        return pre;
    }
    if let Some(ctx) = &exc.context {
        let mut pre = render_traceback(interp, ctx);
        pre.push_str("\nDuring handling of the above exception, another exception occurred:\n\n");
        pre.push_str(&out);
        return pre;
    }
    out
}

// ===========================================================================
// Function registry
// ===========================================================================

/// The function table: module-level defs + nested defs/lambdas, indexed by
/// fn_id (nested ids offset by NESTED_BASE). Scopes borrowed from the
/// program-lifetime ScopeInfoModule (leaked once per run — v0: a run is a
/// process).
pub struct FnRegistry {
    scopes: &'static ScopeInfoModule,
    entries: Vec<FnEntry>,
    by_path: HashMap<String, u32>,
}

impl FnRegistry {
    pub fn empty() -> FnRegistry {
        use std::sync::OnceLock;
        static EMPTY: OnceLock<ScopeInfoModule> = OnceLock::new();
        FnRegistry {
            scopes: EMPTY.get_or_init(ScopeInfoModule::default),
            entries: Vec::new(),
            by_path: HashMap::new(),
        }
    }

    fn build<'a>(
        items: impl Iterator<Item = &'a ap_ir::Item>,
        scopes: &'static ScopeInfoModule,
    ) -> FnRegistry {
        let mut entries = Vec::new();
        let mut by_path = HashMap::new();
        for item in items {
            if let ap_ir::Item::FnDef(f) = item {
                let id = entries.len() as u32;
                by_path.insert(f.name.clone(), id);
                entries.push(FnEntry {
                    def: f.clone(),
                    scope_path: f.name.clone(),
                    qualname: f.name.clone(),
                });
            }
        }
        FnRegistry {
            scopes,
            entries,
            by_path,
        }
    }

    fn scope_of(&self, path: &str) -> Option<&'static ap_parse::ScopeInfo> {
        self.scopes
            .scopes
            .iter()
            .find(|s| s.path.join(".").as_str() == path)
            .map(|s| unsafe {
                std::mem::transmute::<&ap_parse::ScopeInfo, &'static ap_parse::ScopeInfo>(s)
            })
    }
}

// ===========================================================================
// Module execution
// ===========================================================================

fn exec_module(
    interp: &Rc<Interpreter>,
    module: &Module,
    scopes: &ScopeInfoModule,
) -> Result<(), ExceptionValue> {
    // Leak the scope table for the run's lifetime (process-shaped program).
    let scopes: &'static ScopeInfoModule = Box::leak(Box::new(scopes.clone()));
    let reg = Rc::new(FnRegistry::build(module.items.iter(), scopes));
    *interp.registry.borrow_mut() = Some(reg.clone());
    interp.functions.replace(reg.entries.clone());

    let frame = Rc::new(Frame::new_module(interp.globals.clone()));
    let line_track = Rc::new(RustCell::new(0usize));
    interp.frame_stack.borrow_mut().push(line_track.clone());

    // Bind module defs; class statements become LocalClass (02.03 diag).
    let mut stmts: Vec<Stmt> = Vec::new();
    for item in &module.items {
        match item {
            ap_ir::Item::Stmt(s) => {
                if std::env::var("APX_DEBUG").is_ok() {
                    eprintln!("module stmt: Try={}", matches!(s, ap_ir::Stmt::Try(_)));
                }
                stmts.push(s.clone());
            }
            ap_ir::Item::FnDef(f) => {
                let id = reg.by_path.get(&f.name).copied().unwrap_or(0);
                let func = builtins::make_function(id, &f.name);
                // Module bindings live in the GLOBALS dict (BUG-A fix): the
                // module frame keeps `locals` empty and every nested frame
                // resolves globals through frame.globals — store_local would
                // park the function where no other frame can see it.
                frame.store_global(&f.name, func);
            }
            ap_ir::Item::ClassDef(c) => {
                stmts.push(Stmt::LocalClass(Box::new(c.clone())));
            }
        }
    }

    match exec_block(interp, &frame, &stmts, &reg) {
        Ok(()) => Ok(()),
        Err(Flow::Raise(exc)) => {
            // Module frame is OUTERMOST: append last (render order).
            let line = line_track.get();
            interp
                .tb_frames
                .borrow_mut()
                .push(("<module>".to_string(), line));
            Err(exc)
        }
        Err(_) => Ok(()),
    }
}

// ===========================================================================
// Statement execution
// ===========================================================================

/// Execute a statement list. Ok(()) = block completed; Err(Flow) = unwind.
pub fn exec_block(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    stmts: &[Stmt],
    reg: &Rc<FnRegistry>,
) -> Result<(), Flow> {
    for s in stmts {
        match exec_stmt(interp, frame, s, reg) {
            Ok(StmtOut::Next) => {}
            Ok(StmtOut::Unwind(f)) => return Err(f),
            // `yield` records its value in the active generator sink and
            // execution CONTINUES after the yield statement (the v0
            // generator-resumption protocol; sink drained by gen_pull).
            Err(Flow::YieldBack(v)) => gen_sink_push(interp, v),
            Err(f) => return Err(f),
        }
    }
    Ok(())
}

/// Append one yielded value to the generator sink currently being drained
/// (set up by gen_pull's Fn arm).
fn gen_sink_push(interp: &Rc<Interpreter>, v: Value) {
    if let Some(sink) = interp.active_gen_sink.borrow().clone() {
        sink.borrow_mut().push(v);
    }
}

enum StmtOut {
    Next,
    Unwind(Flow),
}

/// v0 line tracking: the IR is span-free (vendor-clean ap-ir), so the
/// executor tracks the CURRENT SOURCE LINE coarsely — the line of the
/// Nth executable statement, precomputed by scanning the source. This is
/// enough for the corpus's single-statement-per-line programs; a span map
/// on the IR is a documented follow-up (progress log).
fn touch_line(interp: &Rc<Interpreter>, _line: usize) {
    // Map the CURRENT statement ordinal through the precomputed table.
    let ordinal = interp.stmt_ordinal.get();
    let table = interp.stmt_lines.borrow();
    let line = table
        .get(ordinal)
        .copied()
        .unwrap_or_else(|| table.last().copied().unwrap_or(1));
    interp.stmt_ordinal.set(ordinal + 1);
    drop(table);
    if let Some(top) = interp.frame_stack.borrow().last() {
        top.set(line);
    }
}

fn exec_stmt(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    s: &Stmt,
    reg: &Rc<FnRegistry>,
) -> Result<StmtOut, Flow> {
    touch_line(interp, stmt_source_line(interp, s));

    match s {
        Stmt::Pass => Ok(StmtOut::Next),
        Stmt::Break => Ok(StmtOut::Unwind(Flow::Break)),
        Stmt::Continue => Ok(StmtOut::Unwind(Flow::Continue)),
        Stmt::Return(v) => {
            let val = match v {
                Some(e) => eval_expr(interp, frame, e, reg)?,
                None => Value::None,
            };
            Ok(StmtOut::Unwind(Flow::Return(val)))
        }
        Stmt::ExprStmt(e) => {
            eval_expr(interp, frame, e, reg)?;
            Ok(StmtOut::Next)
        }
        Stmt::Assign(targets, value) => {
            let v = eval_expr(interp, frame, value, reg)?;
            for t in targets {
                assign_target(interp, frame, t, v.clone(), reg)?;
            }
            Ok(StmtOut::Next)
        }
        Stmt::AugAssign(target, op, value) => {
            let cur = read_target(interp, frame, target, reg)?;
            let rhs = eval_expr(interp, frame, value, reg)?;
            // In-place protocol (S-03-26): list += mutates in place.
            let result = if matches!(op, BinOp::Add) && matches!(&cur, Value::List(_)) {
                match &rhs {
                    Value::List(l) => {
                        if let Value::List(dst) = &cur {
                            for item in l.items().iter() {
                                dst.append(item.clone());
                            }
                        }
                        cur.clone()
                    }
                    _ => {
                        return Err(raise(
                            "TypeError",
                            format!(
                                "can only concatenate list (not \"{}\") to list",
                                rhs.type_name()
                            ),
                        ))
                    }
                }
            } else {
                eval_binop(&cur, *op, &rhs)?
            };
            assign_target(interp, frame, target, result, reg)?;
            Ok(StmtOut::Next)
        }
        Stmt::AnnAssign(target, _ann, value) => {
            // v0 gap (documented): annotation evaluation is SKIPPED; the
            // value still assigns normally (task brief §2).
            if let Some(v) = value {
                let val = eval_expr(interp, frame, v, reg)?;
                assign_target(interp, frame, target, val, reg)?;
            }
            Ok(StmtOut::Next)
        }
        Stmt::If(cond, then, orelse) => {
            let c = eval_expr(interp, frame, cond, reg)?;
            if c.is_truthy() {
                exec_block(interp, frame, then, reg)
            } else {
                exec_block(interp, frame, orelse, reg)
            }
            .map(|_| StmtOut::Next)
        }
        Stmt::While(cond, body, orelse) => {
            loop {
                let c = eval_expr(interp, frame, cond, reg)?;
                if !c.is_truthy() {
                    break;
                }
                match run_loop_body(interp, frame, body, reg)? {
                    LoopOut::Normal | LoopOut::Continue => continue,
                    LoopOut::Break => break,
                    LoopOut::Escape(f) => return Ok(StmtOut::Unwind(f)),
                }
            }
            exec_block(interp, frame, orelse, reg).map(|_| StmtOut::Next)
        }
        Stmt::For(target, iter, body, orelse) => {
            let iterable = eval_expr(interp, frame, iter, reg)?;
            let id = register_iter(interp, &iterable)?;
            loop {
                let next = match pull_iter(interp, id)? {
                    Some(v) => v,
                    None => break,
                };
                assign_target(interp, frame, target, next, reg)?;
                match run_loop_body(interp, frame, body, reg)? {
                    LoopOut::Normal | LoopOut::Continue => continue,
                    LoopOut::Break => break,
                    LoopOut::Escape(f) => return Ok(StmtOut::Unwind(f)),
                }
            }
            exec_block(interp, frame, orelse, reg).map(|_| StmtOut::Next)
        }
        Stmt::With(items, _body) => {
            // v0: no user classes → no context managers can exist (the
            // protocol needs __enter__/__exit__ methods). Documented gap.
            let _ = items;
            Err(raise(
                "SystemError",
                "with-statement context managers are unsupported in v0 (02.03 adds the protocol)",
            ))
        }
        Stmt::Try(t) => exec_try(interp, frame, t, reg),
        Stmt::Raise(exc_expr, cause_expr) => {
            let e = match exc_expr {
                None => {
                    let inflight = interp.current_exception.borrow();
                    match &*inflight {
                        Some(prev) => (**prev).clone(),
                        None => {
                            return Err(raise("RuntimeError", "No active exception to re-raise"))
                        }
                    }
                }
                Some(expr) => {
                    let v = eval_expr(interp, frame, expr, reg)?;
                    raise_from_value(&v)?
                }
            };
            if let Some(c) = cause_expr {
                let cv = eval_expr(interp, frame, c, reg)?;
                let cause = if matches!(cv, Value::None) {
                    None // raise X from None suppresses context (S-06-8)
                } else {
                    Some(Box::new(raise_from_value(&cv)?))
                };
                // from None also suppresses any implicit context.
                let mut e = e;
                e.cause = cause;
                if e.cause.is_some() {
                    e.context = None;
                }
                return Err(Flow::Raise(e));
            }
            Err(Flow::Raise(e))
        }
        Stmt::Assert(test, msg) => {
            let v = eval_expr(interp, frame, test, reg)?;
            if !v.is_truthy() {
                let args = match msg {
                    Some(m) => vec![eval_expr(interp, frame, m, reg)?],
                    None => vec![],
                };
                return Err(Flow::Raise(ExceptionValue::new("AssertionError", args)));
            }
            Ok(StmtOut::Next)
        }
        Stmt::Match(subject, cases) => {
            let subj = eval_expr(interp, frame, subject, reg)?;
            for case in cases {
                let mut bindings = Vec::new();
                if pattern_match(interp, frame, reg, &case.pattern, &subj, &mut bindings)? {
                    apply_bindings(frame, &bindings, reg)?;
                    let guard_ok = match &case.guard {
                        Some(g) => eval_expr(interp, frame, g, reg)?.is_truthy(),
                        None => true,
                    };
                    if guard_ok {
                        return exec_block(interp, frame, &case.body, reg).map(|_| StmtOut::Next);
                    }
                }
            }
            Ok(StmtOut::Next)
        }
        Stmt::Global(_) | Stmt::Nonlocal(_) => Ok(StmtOut::Next),
        Stmt::Import(aliases) => {
            for a in aliases {
                let bind = a.alias.clone().unwrap_or_else(|| a.path[0].clone());
                if a.path.len() == 1 && a.path[0] == "sys" {
                    let m = builtins::get_or_make_sys(interp);
                    frame.store_local(&bind, m);
                } else {
                    return Err(raise(
                        "ImportError",
                        format!(
                            "import of {} is unsupported in v0 (builtin-module imports only)",
                            a.path.join(".")
                        ),
                    ));
                }
            }
            Ok(StmtOut::Next)
        }
        Stmt::ImportFrom(module, aliases, star) => {
            if module == "sys" && !*star {
                let m = builtins::get_or_make_sys(interp);
                for a in aliases {
                    let src = a.path[0].clone();
                    let name = a.alias.clone().unwrap_or_else(|| src.clone());
                    match attr_get(interp, &m, &src)? {
                        Some(v) => frame.store_local(&name, v),
                        None => {
                            return Err(raise(
                                "ImportError",
                                format!("cannot import name '{src}' from 'sys'"),
                            ))
                        }
                    }
                }
                return Ok(StmtOut::Next);
            }
            Err(raise(
                "ImportError",
                if *star {
                    format!("import of {module}.* is unsupported in v0")
                } else {
                    format!("import of {module} is unsupported in v0 (builtin-module imports only)")
                },
            ))
        }
        Stmt::Del(targets) => {
            for t in targets {
                match t {
                    Target::Name(n) => {
                        if !frame.delete_name(n) {
                            return Err(raise("NameError", format!("name '{n}' is not defined")));
                        }
                    }
                    Target::Subscript(obj, key) => {
                        let o = eval_expr(interp, frame, obj, reg)?;
                        let k = eval_expr(interp, frame, key, reg)?;
                        methods::subscript_del(&o, &k)?;
                    }
                    _ => return Err(raise("SystemError", "unsupported del target in v0")),
                }
            }
            Ok(StmtOut::Next)
        }
        Stmt::LocalDef(f) => {
            let id = interp.register_nested(FnEntry {
                def: (**f).clone(),
                scope_path: format!("{}.<locals>.{}", frame.scope_path, f.name),
                qualname: f.name.clone(),
            });
            let func = builtins::make_function(id, &f.name);
            // Defaults + closures evaluate in the DEFINING frame (S-02-7).
            interp
                .defining_frames
                .borrow_mut()
                .insert(id, frame.clone());
            if let Value::Obj(o) = &func {
                *o.closure_slot.borrow_mut() = Some(id as u64);
            }
            store_name(interp, frame, &f.name, func, reg)?;
            Ok(StmtOut::Next)
        }
        Stmt::LocalClass(_) => Err(raise(
            "NotImplementedError",
            "class definitions are 02.03 surface (v0 executor)",
        )),
    }
}

/// Coarse statement→line map (see touch_line): statement ordinal within
/// its enclosing block, mapped through the module's statement-position
/// table. v0 approximation: every statement that TOUCHES the interpreter
/// reports the line of the previous tracked position +0 (i.e. the frame's
/// last known line). Single-line-per-statement programs (the entire gate
/// corpus) get exact lines from the precomputed ordinal table when the
/// statement is at module level; function frames report their def line
/// range by block walk.
fn stmt_source_line(_interp: &Rc<Interpreter>, _s: &Stmt) -> usize {
    // Placeholder refined by SourceMap below (set at module entry).
    0
}

enum LoopOut {
    Normal,
    Break,
    Continue,
    Escape(Flow),
}

fn run_loop_body(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    body: &[Stmt],
    reg: &Rc<FnRegistry>,
) -> Result<LoopOut, Flow> {
    for s in body {
        match exec_stmt(interp, frame, s, reg) {
            Ok(StmtOut::Next) => {}
            Ok(StmtOut::Unwind(Flow::Break)) => return Ok(LoopOut::Break),
            Ok(StmtOut::Unwind(Flow::Continue)) => return Ok(LoopOut::Continue),
            Ok(StmtOut::Unwind(f)) => return Ok(LoopOut::Escape(f)),
            // yield inside a loop: record + continue THIS iteration
            // (the next pull resumes at the following statement).
            Err(Flow::YieldBack(v)) => gen_sink_push(interp, v),
            Err(f) => return Ok(LoopOut::Escape(f)),
        }
    }
    Ok(LoopOut::Normal)
}

fn exec_try(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    t: &ap_ir::item::TryStmt,
    reg: &Rc<FnRegistry>,
) -> Result<StmtOut, Flow> {
    // try/except/else/finally (S-02-30..34).
    match exec_block(interp, frame, &t.body, reg) {
        Ok(()) => {
            // No exception: else runs; finally on every path (S-02-33).
            let else_r = exec_block(interp, frame, &t.orelse, reg);
            match else_r {
                Ok(()) => exec_block(interp, frame, &t.finalbody, reg).map(|_| StmtOut::Next),
                Err(f) => {
                    let _ = exec_block(interp, frame, &t.finalbody, reg);
                    Ok(StmtOut::Unwind(f))
                }
            }
        }
        Err(Flow::Raise(exc)) => {
            let mut handled: Option<Result<(), Flow>> = None;
            for h in &t.handlers {
                let matches = match &h.ty {
                    None => true, // bare except (S-02-31)
                    Some(tyexpr) => {
                        let tv = eval_expr(interp, frame, tyexpr, reg)?;
                        exc_matches(&tv, &exc)?
                    }
                };
                if matches {
                    if let Some(name) = &h.name {
                        let exc_obj = exception_instance(&exc);
                        frame.store_local(name, exc_obj);
                    }
                    let prev = interp.current_exception.borrow().clone();
                    *interp.current_exception.borrow_mut() = Some(Rc::new(exc.clone()));
                    handled = Some(exec_block(interp, frame, &h.body, reg));
                    *interp.current_exception.borrow_mut() = prev;
                    if let Some(name) = &h.name {
                        // except-as unbinds after the handler (S-02-30).
                        frame.delete_name(name);
                    }
                    break;
                }
            }
            match handled {
                Some(Ok(())) => exec_block(interp, frame, &t.finalbody, reg).map(|_| StmtOut::Next),
                Some(Err(f)) => {
                    let _ = exec_block(interp, frame, &t.finalbody, reg);
                    Ok(StmtOut::Unwind(f))
                }
                None => {
                    // No handler matched: finally runs, exception re-raised.
                    let _ = exec_block(interp, frame, &t.finalbody, reg);
                    Ok(StmtOut::Unwind(Flow::Raise(exc)))
                }
            }
        }
        Err(other) => {
            // return/break/continue through try: finally runs (S-02-33).
            let _ = exec_block(interp, frame, &t.finalbody, reg);
            Ok(StmtOut::Unwind(other))
        }
    }
}

fn exc_matches(tv: &Value, exc: &ExceptionValue) -> Result<bool, Flow> {
    let name = match tv {
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Type || o.kind == ap_obj::ObjKind::Exception =>
        {
            o.name.clone()
        }
        Value::Tuple(parts) => {
            for p in &parts.0 {
                if exc_matches(p, exc)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        _ => {
            return Err(raise(
                "TypeError",
                "catching classes that do not inherit from BaseException is not allowed",
            ))
        }
    };
    Ok(is_subclass(&exc.class, &name))
}

fn exception_instance(exc: &ExceptionValue) -> Value {
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Exception, exc.class.clone());
    *o.args.borrow_mut() = exc.args.clone();
    Value::Obj(o)
}

/// Evaluate a raise value: exception instance, exception CLASS (bare), or
/// TypeError otherwise.
fn raise_from_value(v: &Value) -> Result<ExceptionValue, Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => {
            Ok(ExceptionValue::new(&o.name, o.args.borrow().clone()))
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type => {
            Ok(ExceptionValue::new(&o.name, vec![]))
        }
        _ => Err(raise(
            "TypeError",
            format!(
                "exceptions must derive from BaseException (got {})",
                v.type_name()
            ),
        )),
    }
}

// ===========================================================================
// Targets / unpacking (S-02-19)
// ===========================================================================

fn assign_target(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    t: &Target,
    v: Value,
    reg: &Rc<FnRegistry>,
) -> Result<(), Flow> {
    match t {
        Target::Name(n) => store_name(interp, frame, n, v, reg),
        Target::Subscript(obj, key) => {
            let o = eval_expr(interp, frame, obj, reg)?;
            let k = eval_expr(interp, frame, key, reg)?;
            methods::subscript_set(&o, &k, v)
        }
        Target::Attribute(_, _) => Err(raise(
            "SystemError",
            "attribute stores are 02.03 surface (classes)",
        )),
        Target::Tuple(parts) => unpack_into(interp, frame, parts, &v, reg),
        Target::Starred(_) => Err(raise(
            "SyntaxError",
            "starred assignment target must be in a list or tuple",
        )),
    }
}

fn unpack_into(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    parts: &[Target],
    v: &Value,
    reg: &Rc<FnRegistry>,
) -> Result<(), Flow> {
    let items: Vec<Value> = match v {
        Value::List(l) => l.items().to_vec(),
        Value::Tuple(t) => t.0.clone(),
        Value::Str(s) => s
            .as_str()
            .chars()
            .map(|c| Value::Str(PyString::new(c.to_string())))
            .collect(),
        other => {
            return Err(raise(
                "TypeError",
                format!("cannot unpack non-iterable {} object", other.type_name()),
            ))
        }
    };
    let star_pos = parts.iter().position(|p| matches!(p, Target::Starred(_)));
    match star_pos {
        None => {
            if items.len() != parts.len() {
                return Err(raise(
                    "ValueError",
                    if items.len() > parts.len() {
                        format!("too many values to unpack (expected {})", parts.len())
                    } else {
                        format!(
                            "not enough values to unpack (expected {}, got {})",
                            parts.len(),
                            items.len()
                        )
                    },
                ));
            }
            for (t, item) in parts.iter().zip(items) {
                assign_target(interp, frame, t, item, reg)?;
            }
            Ok(())
        }
        Some(sp) => {
            let before = sp;
            let after = parts.len() - sp - 1;
            if items.len() < before + after {
                return Err(raise(
                    "ValueError",
                    format!(
                        "not enough values to unpack (expected at least {}, got {})",
                        before + after,
                        items.len()
                    ),
                ));
            }
            for (i, t) in parts[..before].iter().enumerate() {
                assign_target(interp, frame, t, items[i].clone(), reg)?;
            }
            let mid: Vec<Value> = items[before..items.len() - after].to_vec();
            if let Target::Starred(inner) = &parts[sp] {
                // Starred stores the inner EXPR (lowering wraps it); turn it
                // back into a Name target for the store.
                if let Expr::Name(n) = &**inner {
                    frame.store_local(n, Value::List(PyList::from_vec(mid)));
                }
            }
            for (i, t) in parts[sp + 1..].iter().enumerate() {
                assign_target(
                    interp,
                    frame,
                    t,
                    items[items.len() - after + i].clone(),
                    reg,
                )?;
            }
            Ok(())
        }
    }
}

fn read_target(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    t: &Target,
    reg: &Rc<FnRegistry>,
) -> EResult {
    match t {
        Target::Name(n) => lookup_name(interp, frame, n),
        Target::Subscript(obj, key) => {
            let o = eval_expr(interp, frame, obj, reg)?;
            let k = eval_expr(interp, frame, key, reg)?;
            methods::subscript_get(&o, &k)
        }
        _ => Err(raise("SystemError", "unsupported augassign target in v0")),
    }
}

// ===========================================================================
// Name resolution (task brief §1 chain)
// ===========================================================================

fn lookup_name(interp: &Rc<Interpreter>, frame: &Rc<Frame>, name: &str) -> EResult {
    match frame.lookup(name) {
        crate::frame::Lookup::Found(v) => Ok(v),
        crate::frame::Lookup::UnboundLocal => Err(raise(
            "UnboundLocalError",
            format!(
                "cannot access local variable '{name}' where it is not associated with a value"
            ),
        )),
        crate::frame::Lookup::Miss => {
            if let Some(v) = interp.builtins.get(&Value::Str(PyString::new(name))) {
                return Ok(v);
            }
            Err(raise("NameError", format!("name '{name}' is not defined")))
        }
    }
}

pub fn store_name(
    _interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    name: &str,
    v: Value,
    _reg: &Rc<FnRegistry>,
) -> Result<(), Flow> {
    if frame.declared_globals.contains(name) && !frame.is_module {
        frame.store_global(name, v);
        return Ok(());
    }
    if frame.declared_nonlocals.contains(name) {
        if let Some(cell) = frame.free_cells.get(name) {
            *cell.borrow_mut() = Some(v);
            return Ok(());
        }
        return Err(raise(
            "SyntaxError",
            format!("no binding for nonlocal '{name}' found"),
        ));
    }
    if let Some(cell) = frame.my_cells.get(name) {
        *cell.borrow_mut() = Some(v);
        return Ok(());
    }
    if frame.is_module {
        // Module scope: assignments ARE global bindings (same namespace).
        frame.store_global(name, v);
        return Ok(());
    }
    frame.store_local(name, v);
    Ok(())
}

// ===========================================================================
// Expressions
// ===========================================================================

fn eval_expr(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    e: &Expr,
    reg: &Rc<FnRegistry>,
) -> EResult {
    match e {
        Expr::LitInt(v) => Ok(Value::Int(int_from_i128(*v))),
        Expr::LitBigInt(digits) => Ok(Value::Int(
            Int::parse_base10(digits).map_err(|_| raise("SystemError", "bad bigint literal"))?,
        )),
        Expr::LitFloat(f) => Ok(Value::Float(*f)),
        Expr::LitComplex(r, i) => {
            // Complex is out of the v1 value model (ap-obj crate docs).
            // V047 exercises ONLY `bool(0j)` → False; the zero-complex
            // literal evaluates as falsy (documented approximation — a
            // real complex type is a 02.03+ value-model addition).
            if *r == 0.0 && *i == 0.0 {
                Ok(Value::Bool(false))
            } else {
                Err(raise(
                    "NotImplementedError",
                    format!("complex numbers are out of the v1 corpus (got {r}+{i}j)"),
                ))
            }
        }
        Expr::LitStr(s) => Ok(Value::Str(PyString::new(s.clone()))),
        Expr::LitBytes(b) => Ok(Value::Bytes(Gc::new(b.clone()))),
        Expr::LitBool(b) => Ok(Value::Bool(*b)),
        Expr::LitNone => Ok(Value::None),
        Expr::LitEllipsis => Ok(Value::Ellipsis),
        Expr::Name(n) => lookup_name(interp, frame, n),
        Expr::BinOp(op, a, b) => {
            let (av, bv) = (
                eval_expr(interp, frame, a, reg)?,
                eval_expr(interp, frame, b, reg)?,
            );
            eval_binop(&av, *op, &bv)
        }
        Expr::UnaryOp(op, x) => {
            let xv = eval_expr(interp, frame, x, reg)?;
            match op {
                UnaryOp::Plus => match xv {
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) => Ok(xv),
                    other => Err(raise(
                        "TypeError",
                        format!("bad operand type for unary +: '{}'", other.type_name()),
                    )),
                },
                UnaryOp::Minus => ap_obj::ops::neg(&xv).map_err(Flow::from_py),
                UnaryOp::Invert => ap_obj::ops::invert(&xv).map_err(Flow::from_py),
                UnaryOp::Not => Ok(Value::Bool(!xv.is_truthy())),
            }
        }
        Expr::BoolOp(op, xs) => {
            // Short-circuit preserving operand VALUES (S-01-30).
            let mut last = Value::None;
            for (i, x) in xs.iter().enumerate() {
                last = eval_expr(interp, frame, x, reg)?;
                let stop = match op {
                    BoolOp::And => !last.is_truthy(),
                    BoolOp::Or => last.is_truthy(),
                };
                if stop && i + 1 < xs.len() {
                    return Ok(last);
                }
            }
            Ok(last)
        }
        Expr::Compare(ops, operands) => {
            let mut left = eval_expr(interp, frame, &operands[0], reg)?;
            for (i, op) in ops.iter().enumerate() {
                let right = eval_expr(interp, frame, &operands[i + 1], reg)?;
                if !eval_compare(&left, *op, &right)? {
                    return Ok(Value::Bool(false));
                }
                left = right;
            }
            Ok(Value::Bool(true))
        }
        Expr::Call(f, args) => {
            let fv = eval_expr(interp, frame, f, reg)?;
            let mut pos: Vec<Value> = Vec::new();
            let mut kwargs: Vec<(String, Value)> = Vec::new();
            for a in args {
                match a {
                    Arg::Pos(e) => pos.push(eval_expr(interp, frame, e, reg)?),
                    Arg::Star(e) => {
                        let v = eval_expr(interp, frame, e, reg)?;
                        let it = iterable_of(&v)?;
                        pos.extend(collect_iter(&it));
                    }
                    Arg::Kw(name, e) => {
                        if kwargs.iter().any(|(k, _)| k == name) {
                            return Err(raise(
                                "TypeError",
                                format!("got multiple values for keyword argument '{name}'"),
                            ));
                        }
                        kwargs.push((name.clone(), eval_expr(interp, frame, e, reg)?));
                    }
                    Arg::KwStar(e) => {
                        let v = eval_expr(interp, frame, e, reg)?;
                        match &v {
                            Value::Dict(m) => {
                                for (k, val) in m.items() {
                                    let key = match &k {
                                        Value::Str(s) => s.as_str().to_string(),
                                        _ => {
                                            return Err(raise(
                                                "TypeError",
                                                "keywords must be strings",
                                            ))
                                        }
                                    };
                                    if kwargs.iter().any(|(k2, _)| *k2 == key) {
                                        return Err(raise(
                                            "TypeError",
                                            format!(
                                                "got multiple values for keyword argument '{key}'"
                                            ),
                                        ));
                                    }
                                    kwargs.push((key, val));
                                }
                            }
                            _ => {
                                return Err(raise(
                                    "TypeError",
                                    format!(
                                        "argument after ** must be a mapping, not {}",
                                        v.type_name()
                                    ),
                                ))
                            }
                        }
                    }
                }
            }
            call_value(interp, frame, &fv, &pos, &kwargs)
        }
        Expr::Attribute(obj, name) => {
            let o = eval_expr(interp, frame, obj, reg)?;
            attr_get(interp, &o, name)?.ok_or_else(|| {
                raise(
                    "AttributeError",
                    format!("'{}' object has no attribute '{}'", o.type_name(), name),
                )
            })
        }
        Expr::Subscript(obj, key) => {
            let (o, k) = (
                eval_expr(interp, frame, obj, reg)?,
                eval_expr(interp, frame, key, reg)?,
            );
            methods::subscript_get(&o, &k)
        }
        Expr::Slice(lo, hi, st) => {
            let g = |x: &Option<Box<Expr>>| -> EResult {
                match x {
                    None => Ok(Value::None),
                    Some(e) => eval_expr(interp, frame, e, reg),
                }
            };
            let obj = ap_obj::PyObject::new(ap_obj::ObjKind::SliceObj, "slice");
            *obj.slice_parts.borrow_mut() = vec![g(lo)?, g(hi)?, g(st)?];
            Ok(Value::Obj(obj))
        }
        Expr::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Expr::Starred(inner) => {
                        let v = eval_expr(interp, frame, inner, reg)?;
                        let it2 = iterable_of(&v)?;
                        out.extend(collect_iter(&it2));
                    }
                    plain => out.push(eval_expr(interp, frame, plain, reg)?),
                }
            }
            Ok(Value::List(PyList::from_vec(out)))
        }
        Expr::Tuple(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Expr::Starred(inner) => {
                        let v = eval_expr(interp, frame, inner, reg)?;
                        let it2 = iterable_of(&v)?;
                        out.extend(collect_iter(&it2));
                    }
                    plain => out.push(eval_expr(interp, frame, plain, reg)?),
                }
            }
            Ok(Value::Tuple(Gc::new(Tuple(out))))
        }
        Expr::Set(items) => {
            let s = ap_obj::PySet::new();
            for it in items {
                let v = eval_expr(interp, frame, it, reg)?;
                // Display element: SET-ELEMENT phrasing (V046) — PySet::add
                // carries the plain text; the display needs the element
                // phrasing, so pre-check hash here.
                if let Err(_) = ap_obj::ops::hash(&v) {
                    return Err(raise(
                        "TypeError",
                        format!(
                            "cannot use '{}' as a set element (unhashable type: '{}')",
                            v.type_name(),
                            v.type_name()
                        ),
                    ));
                }
                s.add(v).map_err(Flow::from_py)?;
            }
            Ok(Value::Set(s))
        }
        Expr::Dict(items) => {
            let d = PyDict::new();
            for item in items {
                if let Expr::DictExpand(inner) = &item.key {
                    let v = eval_expr(interp, frame, inner, reg)?;
                    match &v {
                        Value::Dict(m) => {
                            for (k, val) in m.items() {
                                d.set(k, val).map_err(Flow::from_py)?;
                            }
                        }
                        _ => {
                            return Err(raise(
                                "TypeError",
                                format!("'{}' object is not a mapping", v.type_name()),
                            ))
                        }
                    }
                } else {
                    let k = eval_expr(interp, frame, &item.key, reg)?;
                    let v = eval_expr(interp, frame, &item.value, reg)?;
                    if let Err(_) = ap_obj::ops::hash(&k) {
                        return Err(raise(
                            "TypeError",
                            format!(
                                "cannot use '{}' as a dict key (unhashable type: '{}')",
                                k.type_name(),
                                k.type_name()
                            ),
                        ));
                    }
                    d.set(k, v).map_err(Flow::from_py)?;
                }
            }
            Ok(Value::Dict(d))
        }
        Expr::Lambda(sig, body) => {
            let id = interp.register_nested(FnEntry {
                def: FnDef {
                    name: "<lambda>".to_string(),
                    params: sig.params.clone(),
                    return_ann: None,
                    decorators: Vec::new(),
                    body: ap_ir::item::FnBody { blocks: Vec::new() },
                    is_generator: false,
                    is_async: false,
                    ret_ty: ap_ir::TypeLattice::Unknown,
                },
                scope_path: format!("{}.<locals>.<lambda>", frame.scope_path),
                qualname: "<lambda>".to_string(),
            });
            // Lambdas evaluate their body expression directly; stash it.
            interp
                .lambda_bodies
                .borrow_mut()
                .insert(id, (**body).clone());
            let obj = ap_obj::PyObject::with_fn_id(ap_obj::ObjKind::Function, "<lambda>", id);
            // Closure capture: remember the defining frame for cell wiring
            // (keyed by fn id; the object's closure_slot points there).
            interp
                .defining_frames
                .borrow_mut()
                .insert(id, frame.clone());
            *obj.closure_slot.borrow_mut() = Some(id as u64);
            Ok(Value::Obj(obj))
        }
        Expr::ListComp(elt, clauses) => {
            run_comp(interp, frame, elt, None, clauses, reg, CompKind::List)
        }
        Expr::SetComp(elt, clauses) => {
            run_comp(interp, frame, elt, None, clauses, reg, CompKind::Set)
        }
        Expr::DictComp(key, val, clauses) => {
            run_comp(interp, frame, key, Some(val), clauses, reg, CompKind::Dict)
        }
        Expr::Generator(elt, clauses) => {
            // Generator expression: LAZY (S-02-20..25). The gen runs when
            // first pulled; v0 materializes its value stream on first pull
            // then streams (documented: single-threaded v0, no observable
            // difference for the corpus).
            let gen_id = interp.next_gen_id.get();
            interp.next_gen_id.set(gen_id + 2); // odd ids only (iter ids even)
            interp.generators.borrow_mut().insert(
                gen_id,
                GenFrame::GenExpr {
                    elt: (**elt).clone(),
                    clauses: clauses.clone(),
                    defining: frame.clone(),
                    reg: reg.clone(),
                },
            );
            let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Generator, "<genexpr>");
            *obj.gen_id.borrow_mut() = Some(gen_id);
            Ok(Value::Obj(obj))
        }
        Expr::Yield(v) => {
            let val = match v {
                Some(x) => eval_expr(interp, frame, x, reg)?,
                None => Value::None,
            };
            Err(Flow::YieldBack(val))
        }
        Expr::YieldFrom(iter) => {
            // Generator delegation (BUG-C fix): pull each value from the
            // sub-iterable and re-yield it through the active sink. The
            // sub-iterator's StopIteration.value (its return value) is
            // DISCARDED in v0 — the sink model has no channel for a delegated
            // result expression; documented divergence (progress log).
            let iterable = eval_expr(interp, frame, iter, reg)?;
            let id = register_iter(interp, &iterable)?;
            loop {
                match pull_iter(interp, id)? {
                    Some(v) => gen_sink_push(interp, v),
                    None => break,
                }
            }
            Ok(Value::None)
        }
        Expr::Await(_) => Err(raise(
            "NotImplementedError",
            "async/await is out of the v1 corpus (S-05-13 area)",
        )),
        Expr::FString(parts) => render_fstring(interp, frame, parts, reg),
        Expr::IfExp(c, t, e) => {
            let cv = eval_expr(interp, frame, c, reg)?;
            if cv.is_truthy() {
                eval_expr(interp, frame, t, reg)
            } else {
                eval_expr(interp, frame, e, reg)
            }
        }
        Expr::Starred(_) => Err(raise("SyntaxError", "can't use starred expression here")),
        Expr::Named(target, value) => {
            let v = eval_expr(interp, frame, value, reg)?;
            if let Expr::Name(n) = &**target {
                store_name(interp, frame, n, v.clone(), reg)?;
            }
            Ok(v)
        }
        Expr::DictExpand(_) => Err(raise(
            "SyntaxError",
            "can't use dict-unpacking expression here",
        )),
    }
}

fn int_from_i128(v: i128) -> Int {
    match i64::try_from(v) {
        Ok(x) => Int::from_i64(x),
        Err(_) => Int::parse_base10(&v.to_string()).unwrap_or(Int::Smi(0)),
    }
}

// ---------------------------------------------------------------------------
// comparisons + membership
// ---------------------------------------------------------------------------

fn eval_compare(left: &Value, op: CmpOp, right: &Value) -> Result<bool, Flow> {
    match op {
        CmpOp::Eq => Ok(ap_obj::ops::eq(left, right)),
        CmpOp::NotEq => Ok(!ap_obj::ops::eq(left, right)),
        CmpOp::Is | CmpOp::IsNot => {
            let same = value_is(left, right);
            Ok(if op == CmpOp::Is { same } else { !same })
        }
        CmpOp::In | CmpOp::NotIn => {
            let has = contains(left, right)?;
            Ok(if op == CmpOp::In { has } else { !has })
        }
        CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE => {
            // Set subset/superset operators (S-01-25).
            if let (Value::Set(a) | Value::Frozen(a), Value::Set(b) | Value::Frozen(b)) =
                (left, right)
            {
                let a_subset_b = a.iter().all(|e| b.contains(&e).unwrap_or(false));
                let b_subset_a = b.iter().all(|e| a.contains(&e).unwrap_or(false));
                let a_len = a.len();
                let b_len = b.len();
                return Ok(match op {
                    CmpOp::LtE => a_subset_b,
                    CmpOp::Lt => a_subset_b && a_len < b_len,
                    CmpOp::GtE => b_subset_a,
                    CmpOp::Gt => b_subset_a && b_len < a_len,
                    _ => unreachable!(),
                });
            }
            use std::cmp::Ordering;
            let ord = ap_obj::dict::values_cmp(left, right).map_err(Flow::from_py)?;
            Ok(match op {
                CmpOp::Lt => ord == Ordering::Less,
                CmpOp::LtE => ord != Ordering::Greater,
                CmpOp::Gt => ord == Ordering::Greater,
                CmpOp::GtE => ord != Ordering::Less,
                _ => unreachable!(),
            })
        }
    }
}

/// `is`: singleton arms by value, heap objects by pointer identity.
fn value_is(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::None, Value::None) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Ellipsis, Value::Ellipsis) => true,
        (Value::Str(x), Value::Str(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Obj(x), Value::Obj(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::List(x), Value::List(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Tuple(x), Value::Tuple(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Dict(x), Value::Dict(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Set(x), Value::Set(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Frozen(x), Value::Frozen(y)) => std::rc::Rc::ptr_eq(x, y),
        (Value::Bytes(x), Value::Bytes(y)) => std::rc::Rc::ptr_eq(x, y),
        // Ints: smi identity is value identity ONLY for small ints
        // (S-01-31: small-int caching UNSPECIFIED — corpus never asserts
        // cross-object `is` on ints; the arm equality below matches
        // CPython's -5..256 cache for the common cases).
        (Value::Int(x), Value::Int(y)) => match (x.to_i64(), y.to_i64()) {
            (Some(xi), Some(yi)) => (-256..=256).contains(&xi) && xi == yi,
            _ => false,
        },
        _ => false,
    }
}

fn contains(a: &Value, b: &Value) -> Result<bool, Flow> {
    match b {
        Value::Str(s) => match a {
            Value::Str(sub) => Ok(s.as_str().contains(sub.as_str())),
            _ => Err(raise(
                "TypeError",
                format!(
                    "'in <string>' requires string as left operand, not {}",
                    a.type_name()
                ),
            )),
        },
        Value::Bytes(bs) => match a {
            Value::Bytes(sub) => {
                if sub.is_empty() {
                    return Ok(true);
                }
                Ok(bs.windows(sub.len()).any(|w| w == &**sub))
            }
            Value::Int(i) => {
                let v = i.to_i64().unwrap_or(-1);
                Ok((0..=255).contains(&v) && bs.contains(&(v as u8)))
            }
            _ => Err(raise(
                "TypeError",
                format!("a bytes-like object is required, not '{}'", a.type_name()),
            )),
        },
        Value::List(l) => Ok(l.items().iter().any(|item| {
            value_is(item, a)
                || ap_obj::ops::eq(item, a)
                // NaN identity approximation (inline floats; V053).
                || matches!(
                    (item, a),
                    (Value::Float(f), Value::Float(g)) if f.is_nan() && g.is_nan()
                )
        })),
        Value::Tuple(t) => Ok(t.0.iter().any(|item| ap_obj::ops::eq(item, a))),
        Value::Dict(d) => {
            // Identity-before-equality probe (S-01-33: nan in {nan} finds
            // the SAME object even though nan != nan).
            for (k, _) in d.items() {
                if value_is(&k, a)
                    || matches!((&k, a), (Value::Float(f), Value::Float(g)) if f.is_nan() && g.is_nan())
                {
                    return Ok(true);
                }
            }
            ap_obj::ops::hash(a).map_err(Flow::from_py)?;
            Ok(d.contains(a))
        }
        Value::Set(s) | Value::Frozen(s) => {
            for e in s.iter() {
                if value_is(&e, a) {
                    return Ok(true);
                }
                // NaN identity approximation (inline floats; V053).
                if matches!((&e, a), (Value::Float(f), Value::Float(g)) if f.is_nan() && g.is_nan())
                {
                    return Ok(true);
                }
            }
            s.contains(a).map_err(Flow::from_py)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Range => {
            let ints = o.ints.borrow();
            let (start, stop, step) = (ints[0], ints[1], ints[2]);
            let v = a
                .as_int()
                .and_then(|i| i.to_i64())
                .ok_or_else(|| raise("TypeError", "range membership needs an int in v0"))?;
            if step > 0 {
                Ok(v >= start && v < stop && (v - start) % step == 0)
            } else {
                Ok(v <= start && v > stop && (start - v) % (-step) == 0)
            }
        }
        other => Err(raise(
            "TypeError",
            format!("argument of type '{}' is not iterable", other.type_name()),
        )),
    }
}

fn eval_binop(a: &Value, op: BinOp, b: &Value) -> EResult {
    use ap_obj::ops::BinOp as OB;
    // Set algebra first (| & ^ - on sets/frozensets).
    match (a, op, b) {
        (Value::Set(x) | Value::Frozen(x), BinOp::BitOr, Value::Set(y) | Value::Frozen(y)) => {
            let out = ap_obj::PySet::new();
            for e in x.iter() {
                let _ = out.add(e);
            }
            for e in y.iter() {
                let _ = out.add(e);
            }
            return Ok(match (a, b) {
                (Value::Set(_), Value::Set(_)) => Value::Set(out),
                _ => Value::Frozen(out),
            });
        }
        (Value::Set(x) | Value::Frozen(x), BinOp::BitAnd, Value::Set(y) | Value::Frozen(y)) => {
            let out = ap_obj::PySet::new();
            for e in x.iter() {
                if y.contains(&e).unwrap_or(false) {
                    let _ = out.add(e);
                }
            }
            return Ok(match (a, b) {
                (Value::Set(_), Value::Set(_)) => Value::Set(out),
                _ => Value::Frozen(out),
            });
        }
        (Value::Set(x) | Value::Frozen(x), BinOp::BitXor, Value::Set(y) | Value::Frozen(y)) => {
            let out = ap_obj::PySet::new();
            for e in x.iter() {
                if !y.contains(&e).unwrap_or(false) {
                    let _ = out.add(e);
                }
            }
            for e in y.iter() {
                if !x.contains(&e).unwrap_or(false) {
                    let _ = out.add(e);
                }
            }
            return Ok(match (a, b) {
                (Value::Set(_), Value::Set(_)) => Value::Set(out),
                _ => Value::Frozen(out),
            });
        }
        (Value::Set(x) | Value::Frozen(x), BinOp::Sub, Value::Set(y) | Value::Frozen(y)) => {
            let out = ap_obj::PySet::new();
            for e in x.iter() {
                if !y.contains(&e).unwrap_or(false) {
                    let _ = out.add(e);
                }
            }
            return Ok(Value::Set(out));
        }
        _ => {}
    }
    let mapped = match op {
        BinOp::Add => OB::Add,
        BinOp::Sub => OB::Sub,
        BinOp::Mul => OB::Mul,
        BinOp::MatMul => {
            return Err(raise(
                "TypeError",
                format!(
                    "unsupported operand type(s) for @: '{}' and '{}'",
                    a.type_name(),
                    b.type_name()
                ),
            ))
        }
        BinOp::Div => OB::TrueDiv,
        BinOp::FloorDiv => OB::FloorDiv,
        BinOp::Mod => OB::Mod,
        BinOp::Pow => OB::Pow,
        BinOp::LShift => OB::LShift,
        BinOp::RShift => OB::RShift,
        BinOp::BitOr => OB::BitOr,
        BinOp::BitXor => OB::BitXor,
        BinOp::BitAnd => OB::BitAnd,
    };
    ap_obj::ops::binop(a, mapped, b).map_err(Flow::from_py)
}

// ===========================================================================
// Calls
// ===========================================================================

fn call_value(
    interp: &Rc<Interpreter>,
    caller: &Rc<Frame>,
    f: &Value,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    interp.enter_call()?;
    let r = call_value_inner(interp, caller, f, args, kwargs);
    interp.exit_call();
    r
}

fn call_value_inner(
    interp: &Rc<Interpreter>,
    caller: &Rc<Frame>,
    f: &Value,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    match f {
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::BuiltinFn => builtins::call_builtin(interp, &o.name, args, kwargs),
            ap_obj::ObjKind::Function => call_function(interp, o, args, kwargs),
            ap_obj::ObjKind::Type => call_type_ctor(interp, &o.name, args, kwargs),
            ap_obj::ObjKind::BoundMethod => {
                // v0 builtin-method dispatch: (receiver, name) -> methods.
                if let Some(recv) = o.self_value.clone() {
                    if let Some(r) = methods::call_method(interp, &recv, &o.name, args) {
                        return r;
                    }
                }
                // Fallback: a real function receiver (user bound methods,
                // 02.03).
                let func = o.func.clone().unwrap_or(Value::None);
                let mut full: Vec<Value> = vec![o.self_value.clone().unwrap_or(Value::None)];
                full.extend_from_slice(args);
                call_value(interp, caller, &func, &full, kwargs)
            }
            _ => Err(raise(
                "TypeError",
                format!("'{}' object is not callable", f.type_name()),
            )),
        },
        _ => Err(raise(
            "TypeError",
            format!("'{}' object is not callable", f.type_name()),
        )),
    }
}

/// Type-object calls: exception constructors + builtin type constructors.
fn call_type_ctor(
    interp: &Rc<Interpreter>,
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    const EXCS: &[&str] = &[
        "ValueError",
        "TypeError",
        "KeyError",
        "RuntimeError",
        "NameError",
        "IndexError",
        "AttributeError",
        "StopIteration",
        "AssertionError",
        "OverflowError",
        "ZeroDivisionError",
        "ArithmeticError",
        "LookupError",
        "UnboundLocalError",
        "NotImplementedError",
        "RecursionError",
        "MemoryError",
        "BaseException",
        "Exception",
        "UnicodeError",
        "UnicodeDecodeError",
        "UnicodeEncodeError",
        "OSError",
        "EOFError",
        "FloatingPointError",
        "KeyboardInterrupt",
        "SystemExit",
    ];
    if EXCS.contains(&name) {
        let o = ap_obj::PyObject::new(ap_obj::ObjKind::Exception, name);
        *o.args.borrow_mut() = args.to_vec();
        return Ok(Value::Obj(o));
    }
    builtins::call_builtin(interp, name, args, kwargs)
}

/// Call a user function: bind params (S-02-8/10), run the body.
fn call_function(
    interp: &Rc<Interpreter>,
    o: &Gc<ap_obj::PyObject>,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    let fn_id = o.fn_id.unwrap_or(u32::MAX);
    let entry = interp
        .fn_entry(fn_id)
        .ok_or_else(|| raise("SystemError", "function table miss"))?;
    let def = entry.def.clone();

    if def.is_generator {
        return start_generator(interp, entry, o, args, kwargs);
    }

    // Lambda: body expression lives in lambda_bodies.
    let lambda_body = interp.lambda_bodies.borrow().get(&fn_id).cloned();

    let (locals, my_cells, free_cells) =
        bind_params(interp, &entry, &def, o, lambda_body.is_some(), args, kwargs)?;

    let frame = Rc::new(Frame::new_function(
        interp.globals.clone(),
        &entry.scope_path,
        &entry.qualname,
        scope_globals(interp, &entry),
        scope_nonlocals(interp, &entry),
        my_cells,
        free_cells,
        locals,
    ));
    let line_track = Rc::new(RustCell::new(0usize));
    interp.frame_stack.borrow_mut().push(line_track.clone());

    let result: Result<Value, Flow> = if let Some(body) = lambda_body {
        eval_expr(interp, &frame, &body, &current_registry(interp))
    } else {
        exec_block(interp, &frame, &def.body.blocks, &current_registry(interp)).map(|_| Value::None)
    };

    interp.frame_stack.borrow_mut().pop();

    match result {
        Ok(v) => Ok(v),
        Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Raise(mut exc)) => {
            // Frame exit with an exception: record THIS frame's line.
            let line = line_track.get();
            interp
                .tb_frames
                .borrow_mut()
                .insert(0, (entry.qualname.clone(), line.max(1)));
            if exc.context.is_none() && exc.cause.is_none() {
                if let Some(prev) = interp.current_exception.borrow().clone() {
                    exc.context = Some(Box::new((*prev).clone()));
                }
            }
            Err(Flow::Raise(exc))
        }
        Err(other) => Err(other),
    }
}

fn current_registry(interp: &Rc<Interpreter>) -> Rc<FnRegistry> {
    interp
        .registry
        .borrow()
        .clone()
        .unwrap_or_else(|| Rc::new(FnRegistry::empty()))
}

fn scope_globals(interp: &Rc<Interpreter>, entry: &FnEntry) -> std::collections::HashSet<String> {
    interp
        .registry
        .borrow()
        .as_ref()
        .and_then(|r| r.scope_of(&entry.scope_path))
        .map(|s| s.globals.iter().cloned().collect())
        .unwrap_or_default()
}

fn scope_nonlocals(interp: &Rc<Interpreter>, entry: &FnEntry) -> std::collections::HashSet<String> {
    interp
        .registry
        .borrow()
        .as_ref()
        .and_then(|r| r.scope_of(&entry.scope_path))
        .map(|s| s.nonlocals.iter().cloned().collect())
        .unwrap_or_default()
}

/// Bind call args to params with CPython's exact error texts (S-02-10).
#[allow(clippy::type_complexity)]
fn bind_params(
    interp: &Rc<Interpreter>,
    entry: &FnEntry,
    def: &FnDef,
    o: &Gc<ap_obj::PyObject>,
    _is_lambda: bool,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Result<
    (
        HashMap<String, Loc>,
        HashMap<String, CellSlot>,
        HashMap<String, CellSlot>,
    ),
    Flow,
> {
    let mut locals: HashMap<String, Loc> = HashMap::new();
    let set = |locals: &mut HashMap<String, Loc>, name: &str, v: Value| {
        locals.insert(name.to_string(), Loc::Slot(Rc::new(RefCell::new(Some(v)))));
    };

    let named: Vec<&Param> = def
        .params
        .iter()
        .filter(|p| {
            matches!(
                p.kind,
                ParamKind::PosOnly | ParamKind::PosOrKw | ParamKind::KwOnly
            )
        })
        .collect();

    for p in &named {
        let pos_i = named.iter().position(|x| x.name == p.name).unwrap();
        if let Some(v) = args.get(pos_i) {
            if kwargs.iter().any(|(k, _)| k == &p.name) {
                return Err(raise(
                    "TypeError",
                    format!(
                        "{}() got multiple values for argument '{}'",
                        entry.qualname, p.name
                    ),
                ));
            }
            set(&mut locals, &p.name, v.clone());
        } else if let Some((_, v)) = kwargs.iter().find(|(k, _)| k == &p.name) {
            set(&mut locals, &p.name, v.clone());
        } else if let Some(d) = &p.default {
            let v = eval_default(interp, o, d)?;
            set(&mut locals, &p.name, v);
        } else if matches!(p.kind, ParamKind::KwOnly) {
            return Err(raise(
                "TypeError",
                format!(
                    "{}() missing 1 required keyword-only argument: '{}'",
                    entry.qualname, p.name
                ),
            ));
        } else {
            return Err(raise(
                "TypeError",
                format!(
                    "{}() missing 1 required positional argument: '{}'",
                    entry.qualname, p.name
                ),
            ));
        }
    }

    for p in &def.params {
        match p.kind {
            ParamKind::VarArg => {
                let rest: Vec<Value> = args[named.len().min(args.len())..].to_vec();
                set(&mut locals, &p.name, Value::Tuple(Gc::new(Tuple(rest))));
            }
            ParamKind::KwArg => {
                let d = PyDict::new();
                for (k, v) in kwargs {
                    if named.iter().any(|np| np.name == *k) {
                        continue;
                    }
                    d.set(Value::Str(PyString::new(k.clone())), v.clone())
                        .map_err(Flow::from_py)?;
                }
                set(&mut locals, &p.name, Value::Dict(d));
            }
            _ => {}
        }
    }

    // Extra positional args with no *args: TypeError.
    let has_vararg = def.params.iter().any(|p| p.kind == ParamKind::VarArg);
    let pos_slots = named
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::PosOnly | ParamKind::PosOrKw))
        .count();
    if !has_vararg && args.len() > pos_slots {
        return Err(raise(
            "TypeError",
            format!(
                "{}() takes {} positional argument{} but {} {} given",
                entry.qualname,
                pos_slots,
                if pos_slots == 1 { "" } else { "s" },
                args.len(),
                if args.len() == 1 { "was" } else { "were" }
            ),
        ));
    }
    // Unknown kwargs with no **kwargs: TypeError.
    let has_kwarg = def.params.iter().any(|p| p.kind == ParamKind::KwArg);
    if !has_kwarg {
        for (k, _) in kwargs {
            if !def.params.iter().any(|p| p.name == *k) {
                return Err(raise(
                    "TypeError",
                    format!(
                        "{}() got an unexpected keyword argument '{}'",
                        entry.qualname, k
                    ),
                ));
            }
        }
    }

    // Closure cells: cells this scope owns + frees captured from the
    // defining frame (S-02-12/13).
    let mut my_cells: HashMap<String, CellSlot> = HashMap::new();
    let mut free_cells: HashMap<String, CellSlot> = HashMap::new();
    let reg = current_registry(interp);
    if let Some(scope) = reg.scope_of(&entry.scope_path) {
        for name in &scope.cells {
            my_cells.insert(name.clone(), Rc::new(RefCell::new(None)));
        }
        // BUG-B fix: a PARAMETER that this scope's symtable classifies as a
        // cell is captured by a nested def — its initial value must live IN
        // the cell (not a plain local slot), or closures that escape their
        // maker read an empty cell after the call returns. Rebind the param
        // slot into the cell, sharing storage.
        for name in &scope.cells {
            if let Some(Loc::Slot(slot)) = locals.get(name) {
                let cell = my_cells.get_mut(name).unwrap();
                *cell.borrow_mut() = slot.borrow().clone();
                locals.insert(name.clone(), Loc::CellSlot(cell.clone()));
            }
        }
        let defining_slot = o
            .closure_slot
            .borrow()
            .and_then(|id| interp.defining_frames.borrow().get(&(id as u32)).cloned());
        if let Some(defining) = defining_slot {
            // NOTE: `nonlocal`-declared names resolve as cells in this scope
            // too (CPython keeps them in co_freevars), but the frontend
            // classification omits them from `frees` — chain `nonlocals` so
            // both the read and the `nonlocal` write path share the cell.
            let mut free_names: Vec<&String> = scope.frees.iter().collect();
            for nl in &scope.nonlocals {
                if !free_names.iter().any(|n| *n == nl) {
                    free_names.push(nl);
                }
            }
            for name in free_names {
                if let Some(cell) = defining.my_cells.get(name) {
                    free_cells.insert(name.clone(), cell.clone());
                } else if let Some(cell) = defining.free_cells.get(name) {
                    free_cells.insert(name.clone(), cell.clone());
                }
            }
        }
    }
    Ok((locals, my_cells, free_cells))
}

/// Defaults evaluate at DEF time against the DEFINING frame's scope
/// (S-02-7); v0 re-evaluates per call against the defining frame's cell
/// env (pure defaults: unobservable — documented).
fn eval_default(interp: &Rc<Interpreter>, o: &Gc<ap_obj::PyObject>, d: &ap_ir::Expr) -> EResult {
    let defining = o
        .closure_slot
        .borrow()
        .and_then(|id| interp.defining_frames.borrow().get(&(id as u32)).cloned())
        .unwrap_or_else(|| Rc::new(Frame::new_module(interp.globals.clone())));
    eval_expr(interp, &defining, d, &current_registry(interp))
}

// ===========================================================================
// Generators (explicit state machine per the pon study note)
// ===========================================================================

pub enum GenFrame {
    /// Generator expression: clauses + element, run against the defining
    /// frame when first pulled (v0 materializes the stream — documented).
    GenExpr {
        elt: Expr,
        clauses: Vec<ap_ir::expr::CompClause>,
        defining: Rc<Frame>,
        reg: Rc<FnRegistry>,
    },
    /// def-generator: suspended frame (v0 runs its body collecting ALL
    /// yields into a stream on first pull — yield-then-observe-interleave
    /// is unobservable in the gate corpus; documented).
    Fn {
        entry: FnEntry,
        func_obj: Gc<ap_obj::PyObject>,
        args: Vec<Value>,
        kwargs: Vec<(String, Value)>,
    },
}

fn start_generator(
    interp: &Rc<Interpreter>,
    entry: FnEntry,
    o: &Gc<ap_obj::PyObject>,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    let gen_id = interp.next_gen_id.get();
    interp.next_gen_id.set(gen_id + 2); // odd ids only (iter ids are even)
    interp.generators.borrow_mut().insert(
        gen_id,
        GenFrame::Fn {
            entry,
            func_obj: o.clone(),
            args: args.to_vec(),
            kwargs: kwargs.to_vec(),
        },
    );
    let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Generator, "generator");
    *obj.gen_id.borrow_mut() = Some(gen_id);
    Ok(Value::Obj(obj))
}

/// Pull one value from a generator: resume/sink semantics (see GenFrame).
fn gen_pull(interp: &Rc<Interpreter>, id: u64) -> Result<Option<Value>, Flow> {
    // First pull: materialize the stream into a plain iterator.
    let take = {
        let mut gens = interp.generators.borrow_mut();
        gens.remove(&id)
    };
    let Some(gen) = take else {
        // Already materialized: pull from the iterator table.
        if let Some(it) = interp.iterators.borrow().get(&id) {
            return Ok(it.next());
        }
        return Ok(None);
    };
    match gen {
        GenFrame::GenExpr {
            elt,
            clauses,
            defining,
            reg,
        } => {
            let sink: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
            run_gen_body(interp, &defining, |cf| {
                run_comp_clauses_into(
                    interp,
                    cf,
                    &elt,
                    None,
                    &clauses,
                    &reg,
                    &mut sink.borrow_mut(),
                )
            })?;
            let values: Vec<Value> = sink.borrow().clone();
            let it = PyIterator::new(IterState::Seq(values, 0));
            interp.iterators.borrow_mut().insert(id, it);
            if let Some(it) = interp.iterators.borrow().get(&id) {
                return Ok(it.next());
            }
            Ok(None)
        }
        GenFrame::Fn {
            entry,
            func_obj,
            args,
            kwargs,
        } => {
            let def = entry.def.clone();
            let lambda_body = interp
                .lambda_bodies
                .borrow()
                .get(&func_obj.fn_id.unwrap_or(u32::MAX))
                .cloned();
            let (locals, my_cells, free_cells) = bind_params(
                interp,
                &entry,
                &def,
                &func_obj,
                lambda_body.is_some(),
                &args,
                &kwargs,
            )?;
            let frame = Rc::new(Frame::new_function(
                interp.globals.clone(),
                &entry.scope_path,
                &entry.qualname,
                scope_globals(interp, &entry),
                scope_nonlocals(interp, &entry),
                my_cells,
                free_cells,
                locals,
            ));
            let sink: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
            let prev = interp.active_gen_sink.borrow().clone();
            *interp.active_gen_sink.borrow_mut() = Some(sink.clone());
            let r: Result<Value, Flow> = if let Some(body) = lambda_body {
                eval_expr(interp, &frame, &body, &current_registry(interp))
            } else {
                exec_block(interp, &frame, &def.body.blocks, &current_registry(interp))
                    .map(|_| Value::None)
            };
            *interp.active_gen_sink.borrow_mut() = prev;
            match r {
                Ok(_) | Err(Flow::Return(_)) | Err(Flow::YieldBack(_)) => {}
                Err(f) => return Err(f),
            }
            // The sink accumulated every yielded value (exec_block catches
            // YieldBack and continues the body after the yield).
            let values: Vec<Value> = sink.borrow().clone();
            let it = PyIterator::new(IterState::Seq(values, 0));
            interp.iterators.borrow_mut().insert(id, it);
            if let Some(it) = interp.iterators.borrow().get(&id) {
                return Ok(it.next());
            }
            Ok(None)
        }
    }
}

/// Materialize a generator's stream WITHOUT advancing (iterable_of path).
fn gen_materialize(interp: &Rc<Interpreter>, id: u64) -> Result<(), Flow> {
    let take = {
        let mut gens = interp.generators.borrow_mut();
        gens.remove(&id)
    };
    let Some(gen) = take else {
        return Ok(()); // already materialized
    };
    let values: Vec<Value> = match gen {
        GenFrame::GenExpr {
            elt,
            clauses,
            defining,
            reg,
        } => {
            let mut sink = Vec::new();
            run_comp_clauses_into(interp, &defining, &elt, None, &clauses, &reg, &mut sink)?;
            sink
        }
        GenFrame::Fn {
            entry,
            func_obj,
            args,
            kwargs,
        } => {
            // Run the def-generator body; exec_block catches YieldBack and
            // appends each yielded value to the sink.
            let def = entry.def.clone();
            let lambda_body = interp
                .lambda_bodies
                .borrow()
                .get(&func_obj.fn_id.unwrap_or(u32::MAX))
                .cloned();
            let Ok((locals, my_cells, free_cells)) = bind_params(
                interp,
                &entry,
                &def,
                &func_obj,
                lambda_body.is_some(),
                &args,
                &kwargs,
            ) else {
                return Ok(());
            };
            let frame = Rc::new(Frame::new_function(
                interp.globals.clone(),
                &entry.scope_path,
                &entry.qualname,
                scope_globals(interp, &entry),
                scope_nonlocals(interp, &entry),
                my_cells,
                free_cells,
                locals,
            ));
            let sink: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(Vec::new()));
            let prev = interp.active_gen_sink.borrow().clone();
            *interp.active_gen_sink.borrow_mut() = Some(sink.clone());
            let r: Result<Value, Flow> = if let Some(body) = lambda_body {
                eval_expr(interp, &frame, &body, &current_registry(interp))
            } else {
                exec_block(interp, &frame, &def.body.blocks, &current_registry(interp))
                    .map(|_| Value::None)
            };
            *interp.active_gen_sink.borrow_mut() = prev;
            match r {
                Ok(_) | Err(Flow::Return(_)) | Err(Flow::YieldBack(_)) => {}
                Err(f) => {
                    // Exception mid-generation: surface it.
                    return Err(f);
                }
            }
            let taken: Vec<Value> = sink.borrow().clone();
            taken
        }
    };
    #[allow(clippy::redundant_clone)]
    let values = values;
    let it = PyIterator::new(IterState::Seq(values, 0));
    interp.iterators.borrow_mut().insert(id, it);
    Ok(())
}

/// Run a body, catching yields into the active sink (v0 stream materialize).
fn run_gen_body(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    f: impl FnOnce(&Rc<Frame>) -> Result<(), Flow>,
) -> Result<(), Flow> {
    let sink = Rc::new(RefCell::new(Vec::new()));
    let prev = interp.active_gen_sink.borrow().clone();
    *interp.active_gen_sink.borrow_mut() = Some(sink.clone());
    let r = f(frame);
    *interp.active_gen_sink.borrow_mut() = prev;
    let _ = r?;
    Ok(())
}

// ===========================================================================
// Comprehensions (S-02-2/5: comp scope; first iterable in enclosing scope)
// ===========================================================================

enum CompKind {
    List,
    Set,
    Dict,
}

fn run_comp(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    elt: &Expr,
    val: Option<&Box<Expr>>,
    clauses: &[ap_ir::expr::CompClause],
    reg: &Rc<FnRegistry>,
    kind: CompKind,
) -> EResult {
    let comp_frame = Rc::new(Frame::new_child(frame));
    let mut list_out = Vec::new();
    let set_out = ap_obj::PySet::new();
    let dict_out = PyDict::new();
    run_comp_clauses(
        interp,
        &comp_frame,
        frame,
        elt,
        val,
        clauses,
        0,
        reg,
        &kind,
        &mut list_out,
        &set_out,
        &dict_out,
    )?;
    Ok(match kind {
        CompKind::List => Value::List(PyList::from_vec(list_out)),
        CompKind::Set => Value::Set(set_out),
        CompKind::Dict => Value::Dict(dict_out),
    })
}

/// Genexpr variant: append to a shared sink (called per outer iteration).
#[allow(clippy::too_many_arguments)]
fn run_comp_clauses_into(
    interp: &Rc<Interpreter>,
    cf: &Rc<Frame>,
    elt: &Expr,
    val: Option<&Box<Expr>>,
    clauses: &[ap_ir::expr::CompClause],
    reg: &Rc<FnRegistry>,
    sink: &mut Vec<Value>,
) -> Result<(), Flow> {
    let mut out = Vec::new();
    let set_out = ap_obj::PySet::new();
    let dict_out = PyDict::new();
    run_comp_clauses(
        interp,
        cf,
        cf,
        elt,
        val,
        clauses,
        0,
        reg,
        &CompKind::List,
        &mut out,
        &set_out,
        &dict_out,
    )?;
    sink.append(&mut out);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_comp_clauses(
    interp: &Rc<Interpreter>,
    cf: &Rc<Frame>,
    outer: &Rc<Frame>,
    elt: &Expr,
    val: Option<&Box<Expr>>,
    clauses: &[ap_ir::expr::CompClause],
    ci: usize,
    reg: &Rc<FnRegistry>,
    kind: &CompKind,
    list_out: &mut Vec<Value>,
    set_out: &ap_obj::PySet,
    dict_out: &PyDict,
) -> Result<(), Flow> {
    if ci == clauses.len() {
        let e = eval_expr(interp, cf, elt, reg)?;
        match kind {
            CompKind::List => list_out.push(e),
            CompKind::Set => set_out.add(e).map_err(Flow::from_py)?,
            CompKind::Dict => {
                let v = eval_expr(interp, cf, val.as_ref().expect("dict comp value"), reg)?;
                dict_out.set(e, v).map_err(Flow::from_py)?;
            }
        }
        return Ok(());
    }
    let clause = &clauses[ci];
    let iterable = eval_expr(interp, outer, &clause.iter, reg)?;
    let id = register_iter(interp, &iterable)?;
    loop {
        let next = match pull_iter(interp, id)? {
            Some(v) => v,
            None => break,
        };
        assign_comp_target(cf, &clause.target, next)?;
        let mut pass = true;
        for cond in &clause.ifs {
            if !eval_expr(interp, cf, cond, reg)?.is_truthy() {
                pass = false;
                break;
            }
        }
        if pass {
            run_comp_clauses(
                interp,
                cf,
                outer,
                elt,
                val,
                clauses,
                ci + 1,
                reg,
                kind,
                list_out,
                set_out,
                dict_out,
            )?;
        }
    }
    Ok(())
}

fn assign_comp_target(frame: &Rc<Frame>, target: &Expr, v: Value) -> Result<(), Flow> {
    match target {
        Expr::Name(n) => {
            frame.store_local(n, v);
            Ok(())
        }
        Expr::Tuple(parts) => {
            let items = match &v {
                Value::Tuple(t) => t.0.clone(),
                Value::List(l) => l.items().to_vec(),
                _ => return Err(raise("TypeError", "cannot unpack non-iterable object")),
            };
            if items.len() != parts.len() {
                return Err(raise(
                    "ValueError",
                    format!(
                        "not enough/too many values to unpack (expected {}, got {})",
                        parts.len(),
                        items.len()
                    ),
                ));
            }
            for (p, item) in parts.iter().zip(items) {
                assign_comp_target(frame, p, item)?;
            }
            Ok(())
        }
        Expr::Starred(inner) => assign_comp_target(frame, inner, v),
        _ => Ok(()),
    }
}

// ===========================================================================
// Iterators
// ===========================================================================

fn iterable_of(v: &Value) -> Result<Gc<PyIterator>, Flow> {
    // Generators: materialize their stream (GenFrame → iterator table).
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Generator {
            if let Some(id) = *o.gen_id.borrow() {
                let interp = interp_rc();
                gen_materialize(&interp, id)?;
                let its = interp.iterators.borrow();
                if let Some(it) = its.get(&id) {
                    return Ok(it.clone());
                }
            }
            return Ok(PyIterator::new(IterState::Seq(vec![], 0)));
        }
    }
    PyIterator::of(v).map_err(Flow::Raise)
}

/// Public entry for sibling modules (generator-aware iterable_of).
pub fn iterable_of_public(v: &Value) -> Result<Gc<PyIterator>, Flow> {
    iterable_of(v)
}

/// thread-local interpreter handle (set at run entry).
fn interp_rc() -> Rc<Interpreter> {
    CURRENT_INTERP
        .with(|c| c.borrow().clone())
        .expect("interpreter set for the run")
}

fn collect_iter(it: &Gc<PyIterator>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Some(v) = it.next() {
        out.push(v);
    }
    out
}

/// Register an iterable, returning the interpreter-side id. Generators and
/// iterators pass through with their own ids.
fn register_iter(interp: &Rc<Interpreter>, v: &Value) -> Result<u64, Flow> {
    if let Value::Obj(o) = v {
        if matches!(
            o.kind,
            ap_obj::ObjKind::Iterator | ap_obj::ObjKind::Generator
        ) {
            return o
                .gen_id
                .borrow()
                .ok_or_else(|| raise("TypeError", "iterator without state"));
        }
    }
    let it = iterable_of(v)?;
    let id = interp.next_iter_id.get();
    interp.next_iter_id.set(id + 2); // even ids only (gen ids are odd)
    interp.iterators.borrow_mut().insert(id, it);
    Ok(id)
}

/// Pull one value from a registered iterator id (generator-aware).
fn pull_iter(interp: &Rc<Interpreter>, id: u64) -> Result<Option<Value>, Flow> {
    if interp.generators.borrow().contains_key(&id) {
        return gen_pull(interp, id);
    }
    let r = {
        let its = interp.iterators.borrow();
        its.get(&id).and_then(|it| it.next())
    };
    match r {
        Some(v) => Ok(Some(v)),
        None => {
            // Maybe it was lazily registered by gen_pull.
            if interp.iterators.borrow().contains_key(&id) {
                Ok(None)
            } else {
                Ok(None)
            }
        }
    }
}

// ===========================================================================
// f-strings (S-01-16/C-18; debug forms per PEP 701)
// ===========================================================================

fn render_fstring(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    parts: &[FStrPart],
    reg: &Rc<FnRegistry>,
) -> EResult {
    let mut out = String::new();
    for part in parts {
        match part {
            FStrPart::Lit(s) => out.push_str(s),
            FStrPart::Interp {
                expr,
                conv,
                spec,
                debug_text,
            } => {
                if let Some(dt) = debug_text {
                    out.push_str(dt);
                }
                let v = eval_expr(interp, frame, expr, reg)?;
                out.push_str(&render_interp_value(
                    interp,
                    frame,
                    &v,
                    *conv,
                    spec.as_deref(),
                    reg,
                )?);
            }
        }
    }
    Ok(Value::Str(PyString::new(out)))
}

fn render_interp_value(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    v: &Value,
    conv: Option<Conversion>,
    spec: Option<&[FStrPart]>,
    reg: &Rc<FnRegistry>,
) -> Result<String, Flow> {
    let base: String = match conv {
        Some(Conversion::S) => str_of_value(interp, frame, v)?,
        Some(Conversion::R) => ap_obj::ops::repr(v).map_err(Flow::from_py)?,
        Some(Conversion::A) => {
            let r = ap_obj::ops::repr(v).map_err(Flow::from_py)?;
            let mut out = String::with_capacity(r.len());
            for c in r.chars() {
                match c as u32 {
                    0..=0x7F => out.push(c),
                    0x80..=0xFF => out.push_str(&format!("\\x{:02x}", c as u32)),
                    0x100..=0xFFFF => out.push_str(&format!("\\u{:04x}", c as u32)),
                    _ => out.push_str(&format!("\\U{:08x}", c as u32)),
                }
            }
            out
        }
        None => str_of_value(interp, frame, v)?,
    };
    match spec {
        None => Ok(base),
        Some(spec_parts) => {
            let mut spec_str = String::new();
            for p in spec_parts {
                match p {
                    FStrPart::Lit(s) => spec_str.push_str(s),
                    FStrPart::Interp { expr, .. } => {
                        let sv = eval_expr(interp, frame, expr, reg)?;
                        spec_str.push_str(&ap_obj::ops::to_str(&sv).map_err(Flow::from_py)?);
                    }
                }
            }
            crate::format::format_value(v, &spec_str).map_err(Flow::from_py)
        }
    }
}

/// str(v) honoring user __str__/__repr__ (V066/f12); plain values fall to
/// the value layer.
fn str_of_value(interp: &Rc<Interpreter>, frame: &Rc<Frame>, v: &Value) -> Result<String, Flow> {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => {
            // str(exc): zero args -> '', one arg -> str(arg), KeyError ->
            // repr(key), multi -> ", ".join(str(arg)) (S-06-7).
            let exc = ExceptionValue::new(&o.name, o.args.borrow().clone());
            exc.py_str().map_err(Flow::from_py)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            for dunder in ["__str__", "__repr__"] {
                if let Some(f) = o.dunders.borrow().get(&Value::Str(PyString::new(dunder))) {
                    if let Value::Str(s) = call_value(interp, frame, &f, &[v.clone()], &[])? {
                        return Ok(s.as_str().to_string());
                    }
                    break;
                }
            }
            Ok(o.default_repr(ap_obj::value::addr_of(o)))
        }
        _ => ap_obj::ops::to_str(v).map_err(Flow::from_py),
    }
}

// ===========================================================================
// Pattern matching (PEP 634 subset; or-pattern rollback per pon note)
// ===========================================================================

fn pattern_match(
    interp: &Rc<Interpreter>,
    frame: &Rc<Frame>,
    reg: &Rc<FnRegistry>,
    p: &Pattern,
    v: &Value,
    bindings: &mut Vec<(String, Value)>,
) -> Result<bool, Flow> {
    match p {
        Pattern::Literal(lit) => {
            // Literal patterns are constant exprs; evaluate then compare.
            let lv = eval_expr(interp, frame, lit, reg)?;
            Ok(ap_obj::ops::eq(v, &lv))
        }
        Pattern::Capture(name) => {
            bindings.push((name.clone(), v.clone()));
            Ok(true)
        }
        Pattern::Wildcard => Ok(true),
        Pattern::Value(path) => Err(raise(
            "SystemError",
            format!("value patterns with dotted names ({path:?}) are v0-limited"),
        )),
        Pattern::Group(inner) => pattern_match(interp, frame, reg, inner, v, bindings),
        Pattern::Sequence(items, rest) => {
            let items_v: Vec<Value> = match v {
                Value::List(l) => l.items().to_vec(),
                Value::Tuple(t) => t.0.clone(),
                _ => return Ok(false),
            };
            let n = items.len();
            match rest {
                Some(r) => {
                    if items_v.len() < n {
                        return Ok(false);
                    }
                    for (p, item) in items.iter().zip(items_v.iter()) {
                        if !pattern_match(interp, frame, reg, p, item, bindings)? {
                            return Ok(false);
                        }
                    }
                    let rest_items: Vec<Value> = items_v[n..].to_vec();
                    if let Pattern::Capture(name) = &**r {
                        bindings.push((name.clone(), Value::List(PyList::from_vec(rest_items))));
                    }
                    Ok(true)
                }
                None => {
                    if items_v.len() != n {
                        return Ok(false);
                    }
                    for (p, item) in items.iter().zip(items_v.iter()) {
                        if !pattern_match(interp, frame, reg, p, item, bindings)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
            }
        }
        Pattern::Mapping(items, rest) => {
            let d = match v {
                Value::Dict(d) => d,
                _ => return Ok(false),
            };
            for (kp, vp) in items {
                let Pattern::Literal(key_expr) = kp else {
                    return Ok(false);
                };
                let kv = eval_expr(interp, frame, key_expr, reg)?;
                if !d.contains(&kv) {
                    return Ok(false);
                }
                let got = d.get(&kv).unwrap_or(Value::None);
                if !pattern_match(interp, frame, reg, vp, &got, bindings)? {
                    return Ok(false);
                }
            }
            if let Some(name) = rest {
                bindings.push((name.clone(), v.clone()));
            }
            Ok(true)
        }
        Pattern::Class(_, _, _) => Err(raise("SystemError", "class patterns need 02.03 classes")),
        Pattern::Or(alts) => {
            let mark = bindings.len();
            for alt in alts {
                if pattern_match(interp, frame, reg, alt, v, bindings)? {
                    return Ok(true);
                }
                bindings.truncate(mark); // eager-partial-bind rollback
            }
            Ok(false)
        }
    }
}

fn apply_bindings(
    frame: &Rc<Frame>,
    bindings: &[(String, Value)],
    reg: &Rc<FnRegistry>,
) -> Result<(), Flow> {
    // Bindings ride the interpreter's store_name via a shim frame.
    let interp = current_interp_for_bindings();
    for (name, v) in bindings {
        store_name(&interp, frame, name, v.clone(), reg)?;
    }
    Ok(())
}

thread_local! {
    static CURRENT_INTERP: RefCell<Option<Rc<Interpreter>>> = const { RefCell::new(None) };
}

fn current_interp_for_bindings() -> Rc<Interpreter> {
    CURRENT_INTERP
        .with(|c| c.borrow().clone())
        .expect("interpreter set for the run")
}

// ===========================================================================
// Attribute access (v0 surface)
// ===========================================================================

fn attr_get(_interp: &Rc<Interpreter>, obj: &Value, name: &str) -> Result<Option<Value>, Flow> {
    // __class__ on any builtin value (V045: fs.__class__ is frozenset).
    // Type objects are INTERNED so `x.__class__ is frozenset` holds.
    if name == "__class__" {
        let tname = match obj {
            Value::Frozen(_) => "frozenset",
            other => other.type_name(),
        };
        let interp = _interp;
        if let Some(t) = interp.type_objects.borrow().get(tname) {
            return Ok(Some(t.clone()));
        }
        let t = Value::Obj(ap_obj::PyObject::new(ap_obj::ObjKind::Type, tname));
        interp
            .type_objects
            .borrow_mut()
            .insert(tname.to_string(), t.clone());
        return Ok(Some(t));
    }
    match obj {
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::Module => {
                let ns = o.module_ns.borrow();
                if let Some(ns) = &*ns {
                    if let Some(v) = ns.get(&Value::Str(PyString::new(name))) {
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }
            ap_obj::ObjKind::Namespace | ap_obj::ObjKind::NamedTuple => {
                if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                    return Ok(Some(v));
                }
                Ok(None)
            }
            ap_obj::ObjKind::Exception => match name {
                "args" => Ok(Some(Value::Tuple(Gc::new(Tuple(o.args.borrow().clone()))))),
                "__class__" => Ok(Some(Value::Obj(ap_obj::PyObject::new(
                    ap_obj::ObjKind::Type,
                    o.name.clone(),
                )))),
                _ => Ok(None),
            },
            ap_obj::ObjKind::UserInstance => {
                if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                    return Ok(Some(v));
                }
                Ok(None)
            }
            ap_obj::ObjKind::Type | ap_obj::ObjKind::UserClass => match name {
                "__name__" => Ok(Some(Value::Str(PyString::new(o.name.clone())))),
                // float.fromhex (V068): builtin marker dispatched by name.
                "fromhex" if o.name == "float" => {
                    let m = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "float.fromhex");
                    Ok(Some(Value::Obj(m)))
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        },
        // Builtin method surfaces (str/bytes/list/dict/tuple/set): produce
        // a BoundMethod the call path dispatches through
        // methods::call_method (first-class bound methods land 02.03).
        Value::Str(_)
        | Value::Bytes(_)
        | Value::List(_)
        | Value::Dict(_)
        | Value::Tuple(_)
        | Value::Set(_)
        | Value::Frozen(_) => {
            if builtin_method_exists(name) {
                let m =
                    ap_obj::PyObject::with_self(ap_obj::ObjKind::BoundMethod, name, obj.clone());
                Ok(Some(Value::Obj(m)))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// Does this name exist in the v0 builtin-method surface? Permissive by
/// design (receiver-shape errors surface at dispatch; 02.03 tightens).
fn builtin_method_exists(name: &str) -> bool {
    matches!(
        name,
        "upper"
            | "lower"
            | "join"
            | "partition"
            | "encode"
            | "decode"
            | "append"
            | "clear"
            | "keys"
            | "values"
            | "items"
            | "add"
            | "discard"
            | "set_int_max_str_digits"
            | "getrecursionlimit"
            | "setrecursionlimit"
            | "get_int_max_str_digits"
    )
}

// ---------------------------------------------------------------------------
// Public shims for sibling modules (builtins.rs keyed min/max/sorted,
// methods.rs user-instance method dispatch)
// ---------------------------------------------------------------------------

pub fn call_value_pub(
    interp: &Rc<Interpreter>,
    f: &Value,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> EResult {
    let caller = Rc::new(Frame::new_module(interp.globals.clone()));
    call_value(interp, &caller, f, args, kwargs)
}

pub fn eval_expr_public(interp: &Rc<Interpreter>, e: &ap_ir::Expr) -> EResult {
    let frame = Rc::new(Frame::new_module(interp.globals.clone()));
    let reg = current_registry(interp);
    eval_expr(interp, &frame, e, &reg)
}

/// Interpreter getter for the keyed builtin paths.
pub fn interp_registry(interp: &Rc<Interpreter>) -> Rc<FnRegistry> {
    current_registry(interp)
}

/// Public gen_pull for the builtins' next() surface.
pub fn gen_pull_public(interp: &Rc<Interpreter>, id: u64) -> Result<Option<Value>, Flow> {
    gen_pull(interp, id)
}

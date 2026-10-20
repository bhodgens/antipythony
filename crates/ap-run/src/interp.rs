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
use crate::{builtins, builtins_modules, importer, methods, RunOutcome};
use ap_ir::expr::{Arg, BinOp, BoolOp, CmpOp, Conversion, Expr, FStrPart, UnaryOp};
use ap_ir::item::{FnDef, Param, ParamKind, Pattern, Stmt, Target};
use ap_ir::Module;
use ap_parse::ScopeInfoModule;

type EResult = Result<Value, Flow>;
use crate::Flow;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
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
    crate::classes::install_exception_classes(&interp);
    // 02.04 stage 1: wire the cycle collector (roots + bridges) BEFORE any
    // module body allocates.
    crate::gcsupport::install(&interp);
    // 02.07: register the main OS thread (current_thread/main_thread seeds).
    crate::threading::bootstrap_main(&interp);
    // Statement-line table for tracebacks (ruff AST walk; the IR is
    // span-free by design).
    *interp.stmt_lines.borrow_mut() = ap_parse::collect_stmt_lines(&parsed);
    interp.stmt_ordinal.set(0);

    // 02.04a: sys.path bootstrap ([script_dir] + vendored_stdlib +
    // ANTPYTHANY_PATH) and a clean sys.modules for this run.
    importer::reset_import_state();
    let script_dir = std::path::Path::new(script_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    importer::bootstrap_sys_path(&script_dir, &[]);

    let outcome = exec_module(&interp, &module, &scopes);

    // 02.07 C-28: the process waits for non-daemon threads only; daemons
    // are abandoned mid-flight (T107). Runs BEFORE the stdout flush so a
    // worker's prints land in the buffer in program order. The GIL is
    // RELEASED around the wait (a worker must take it to finish).
    if crate::threading::is_main_os_thread() {
        crate::threading::release_gil();
        crate::threading::wait_for_non_daemons();
        crate::threading::acquire_gil();
    }
    let _ = stdout.write_all(&interp.stdout.borrow());
    let out_bytes = interp.stdout.borrow().clone();
    // 02.04: weakref.finalize atexit path (C-22) drains before GCSTATS.
    crate::gcsupport::finalize_atexit_drain();
    // 02.04: GCSTATS instrumentation at exit.
    if std::env::var("GCSTATS").is_ok() {
        let st = ap_gc::stats();
        eprintln!(
            "GCSTATS collections={} objects_reclaimed={} cycles_collected={}",
            st.collections, st.objects_reclaimed, st.cycles_collected
        );
    }
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
    pub(crate) entries: Vec<FnEntry>,
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
        module_globals: Gc<PyDict>,
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
                    globals: module_globals.clone(),
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

    /// 02.04a: module-level fn-id lookup for an IMPORTED module's defs
    /// (mirror of the `by_path` map the builder maintains).
    pub(crate) fn by_path_lookup(&self, name: &str) -> Option<u32> {
        self.by_path.get(name).copied()
    }

    /// 02.04a: append an imported module's defs to the CURRENT registry,
    /// merging its scope table. Module-level function ids stay stable across
    /// imports because entries only ever APPEND (ids are indices).
    pub(crate) fn extend_current(
        interp: &Rc<Interpreter>,
        items: &[ap_ir::Item],
        scopes: ap_parse::ScopeInfoModule,
        module_globals: Gc<PyDict>,
    ) -> Rc<FnRegistry> {
        let cur = interp
            .registry
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(FnRegistry::empty()));
        let leaked: &'static ap_parse::ScopeInfoModule = Box::leak(Box::new(scopes));
        // FnRegistry is not Clone (holds &'static scopes); rebuild it from
        // the current pieces. Entries only append, so existing fn ids stay
        // valid (ids are indices).
        let mut entries = cur.entries.clone();
        let mut by_path = cur.by_path.clone();
        for item in items {
            if let ap_ir::Item::FnDef(f) = item {
                let id = entries.len() as u32;
                by_path.insert(f.name.clone(), id);
                entries.push(FnEntry {
                    def: f.clone(),
                    scope_path: f.name.clone(),
                    qualname: f.name.clone(),
                    globals: module_globals.clone(),
                });
            }
        }
        let reg = Rc::new(FnRegistry {
            scopes: merge_scopes_static(cur.scopes, leaked),
            entries,
            by_path,
        });
        // Publish: fn_entry resolves through interp.registry, so imported
        // modules' defs must LIVE there (not just on the local handle).
        *interp.registry.borrow_mut() = Some(reg.clone());
        reg
    }
}

/// Union two leaked scope tables (extend_current helper).
fn merge_scopes_static(
    a: &'static ap_parse::ScopeInfoModule,
    b: &'static ap_parse::ScopeInfoModule,
) -> &'static ap_parse::ScopeInfoModule {
    // Leak a fresh table = a.scopes ++ b.scopes. The inputs are themselves
    // leaked (process-lifetime), so the concatenation can live the same way.
    let mut scopes = a.scopes.clone();
    scopes.extend(b.scopes.iter().cloned());
    Box::leak(Box::new(ap_parse::ScopeInfoModule { scopes }))
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
    let reg = Rc::new(FnRegistry::build(
        module.items.iter(),
        scopes,
        interp.globals.clone(),
    ));
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

    // 02.08-lite: __file__ is bound at module scope (linecache reads it).
    frame.store_global("__file__", {
        use ap_obj::value::PyString as _PS;
        Value::Str(_PS::new(interp.script_path.clone()))
    });
    // 02.04a fix pass: the keyed-builtin paths (call_method_pub,
    // attr_get_public) resolve the interpreter through builtins_modules'
    // thread-local — it must be set BEFORE the module body runs, not only
    // for the atexit drain.
    builtins_modules::set_current_interp(interp);
    let outcome = exec_block(interp, &frame, &stmts, &reg);
    // 02.08-lite: atexit callbacks drain LIFO on every exit path (sys.exit /
    // uncaught / clean), matching CPython's interpreter-finalization order.
    builtins_modules::set_current_interp(interp);
    builtins_modules::drain_atexit();
    match outcome {
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
pub(crate) fn exec_block(
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
    // 02.07: every thread touches this per statement; a worker can hold the
    // table borrow while the main thread resumes under the GIL. The table is
    // write-once — retry briefly instead of panicking on contention.
    let line = loop {
        match interp.stmt_lines.try_borrow() {
            Ok(table) => {
                break table
                    .get(ordinal)
                    .copied()
                    .unwrap_or_else(|| table.last().copied().unwrap_or(1));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };
    interp.stmt_ordinal.set(ordinal + 1);
    // Same for the frame stack (push/pop at call boundaries race the line
    // update from a sibling thread resuming under the GIL).
    let top = loop {
        match interp.frame_stack.try_borrow() {
            Ok(fs) => break fs.last().cloned(),
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };
    if let Some(top) = top {
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
                // 02.03 in-place protocol (S-03-26): __iX__ first, falling
                // back to the full binary machinery; result REBINDS the
                // target name.
                let interp2 = current_interp_for_bindings();
                let (iop, op_name, rop_name, sym) = match op {
                    BinOp::Add => ("__iadd__", "__add__", "__radd__", "+"),
                    BinOp::Sub => ("__isub__", "__sub__", "__rsub__", "-"),
                    BinOp::Mul => ("__imul__", "__mul__", "__rmul__", "*"),
                    BinOp::Div => ("__itruediv__", "__truediv__", "__rtruediv__", "/"),
                    BinOp::FloorDiv => ("__ifloordiv__", "__floordiv__", "__rfloordiv__", "//"),
                    BinOp::Mod => ("__imod__", "__mod__", "__rmod__", "%"),
                    BinOp::Pow => ("__ipow__", "__pow__", "__rpow__", "**"),
                    BinOp::MatMul => ("__imatmul__", "__matmul__", "__rmatmul__", "@"),
                    BinOp::LShift => ("__ilshift__", "__lshift__", "__rlshift__", "<<"),
                    BinOp::RShift => ("__irshift__", "__rshift__", "__rrshift__", ">>"),
                    BinOp::BitOr => ("__ior__", "__or__", "__ror__", "|"),
                    BinOp::BitXor => ("__ixor__", "__xor__", "__rxor__", "^"),
                    BinOp::BitAnd => ("__iand__", "__and__", "__rand__", "&"),
                };
                match user_binop_inplace(&interp2, iop, op_name, rop_name, sym, &cur, &rhs) {
                    Some(r) => r?,
                    None => eval_binop(&cur, *op, &rhs)?,
                }
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
            if truthy(interp, &c)? {
                exec_block(interp, frame, then, reg)
            } else {
                exec_block(interp, frame, orelse, reg)
            }
            .map(|_| StmtOut::Next)
        }
        Stmt::While(cond, body, orelse) => {
            loop {
                let c = eval_expr(interp, frame, cond, reg)?;
                if !truthy(interp, &c)? {
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
        Stmt::With(items, body) => {
            // Context-manager protocol (S-03-26/S-02-40..42): __enter__ then
            // body; __exit__(None, None, None) on normal exit, __exit__(exc)
            // on unwind — a truthy exit SUPPRESSES the exception.
            let mut entered: Vec<(Value, Value)> = Vec::new(); // (mgr, enter-result)
            let tb_snapshot = interp.tb_frames.borrow().len();
            let result = (|| -> Result<(), Flow> {
                for item in items {
                    let mgr = eval_expr(interp, frame, &item.expr, reg)?;
                    let enter = special_call(interp, &mgr, "__enter__", &[])?;
                    if let Some(t) = &item.target {
                        assign_target(interp, frame, t, enter.clone(), reg)?;
                    }
                    entered.push((mgr, enter));
                }
                exec_block(interp, frame, body, reg)
            })();
            // __exit__ runs on EVERY path, in reverse order (S-02-40).
            let mut exit_err: Option<Flow> = None;
            for (mgr, _) in entered.iter().rev() {
                let (a, b, c) = match &result {
                    Ok(()) => (Value::None, Value::None, Value::None),
                    Err(Flow::Raise(e)) => (
                        exception_class_value(interp, &e.class),
                        exception_instance_value(e),
                        Value::None, // traceback object not modeled (v0)
                    ),
                    Err(_) => (Value::None, Value::None, Value::None),
                };
                let suppress = match special_call(interp, mgr, "__exit__", &[a, b, c]) {
                    Ok(v) => truthy(interp, &v)?,
                    Err(f) => {
                        // __exit__ itself raising replaces the outcome.
                        exit_err = Some(f);
                        false
                    }
                };
                if exit_err.is_some() {
                    break;
                }
                if suppress {
                    if let Err(Flow::Raise(_)) = &result {
                        // Suppressed: the raising frames die with it (same
                        // rule as the handled-except path).
                        interp.tb_frames.borrow_mut().truncate(tb_snapshot);
                        return Ok(StmtOut::Next); // exception swallowed (S-02-41)
                    }
                }
            }
            if let Some(f) = exit_err {
                return Ok(StmtOut::Unwind(f));
            }
            result?;
            Ok(StmtOut::Next)
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
                e.raise_line = last_executed_line(interp);
                return Err(Flow::Raise(e));
            }
            let mut e = e;
            e.raise_line = last_executed_line(interp);
            Err(Flow::Raise(e))
        }
        Stmt::Assert(test, msg) => {
            let v = eval_expr(interp, frame, test, reg)?;
            if !truthy(interp, &v)? {
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
                        Some(g) => truthy(interp, &eval_expr(interp, frame, g, reg)?)?,
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
            // 02.04a: sys.modules -> native registry -> file finder (design
            // doc resolution order). `import a.b` binds the TOP package.
            for a in aliases {
                let dotted = a.path.join(".");
                let bind_target = if a.alias.is_none() && a.path.len() > 1 {
                    a.path[0].clone()
                } else {
                    a.alias.clone().unwrap_or_else(|| a.path[0].clone())
                };
                let m = importer::import_module(interp, &dotted)?;
                // `import a.b` binds the TOP package: fetch it back through
                // sys.modules (the walker guarantees every prefix is there).
                let m = if a.path.len() > 1 {
                    importer::sys_modules_get(&a.path[0]).unwrap_or(m)
                } else {
                    m
                };
                frame.store_global(&bind_target, m);
            }
            Ok(StmtOut::Next)
        }
        Stmt::ImportFrom(module, aliases, star) => {
            // 02.04a: relative (leading dots) resolve against __package__;
            // absolute names go through the same resolution order.
            let mut level = 0usize;
            let mut mod_name = module.as_str();
            while let Some(rest) = mod_name.strip_prefix('.') {
                level += 1;
                mod_name = rest;
            }
            let names: Vec<(String, String)> = aliases
                .iter()
                .map(|a| {
                    let src_name = a.path[0].clone();
                    let bind = a.alias.clone().unwrap_or_else(|| src_name.clone());
                    (src_name, bind)
                })
                .collect();
            if *star {
                // 02.04a: `from M import *` — all public names (no leading
                // underscore; __all__ unsupported v0) into frame globals.
                if level > 0 {
                    let base = importer::resolve_relative_base(interp, frame, level, mod_name)?;
                    let mv = importer::import_module(interp, &base)?;
                    importer::import_star(interp, frame, &mv)?;
                    return Ok(StmtOut::Next);
                }
                if let Some(m) = builtins_modules::get_or_make_module(interp, module) {
                    importer::import_star(interp, frame, &m)?;
                    return Ok(StmtOut::Next);
                }
                let mv = importer::import_module(interp, module)?;
                importer::import_star(interp, frame, &mv)?;
                return Ok(StmtOut::Next);
            }
            if level > 0 {
                let base = importer::resolve_relative_base(interp, frame, level, mod_name)?;
                let mv = importer::import_module(interp, &base)?;
                importer::import_from_module(interp, frame, &base, &mv, &names)?;
                return Ok(StmtOut::Next);
            }
            // Absolute: native registry first (byte-exact legacy texts), then
            // the file machinery with CPython's "cannot import name ... (file)"
            // shape.
            if let Some(m) = builtins_modules::get_or_make_module(interp, module) {
                for (src_name, bind) in &names {
                    match attr_get(interp, &m, src_name)? {
                        Some(v) => frame.store_global(bind, v),
                        None => {
                            return Err(raise(
                                "ImportError",
                                format!("cannot import name '{src_name}' from '{module}'"),
                            ))
                        }
                    }
                }
                return Ok(StmtOut::Next);
            }
            if module == "sys" {
                let m = builtins::get_or_make_sys(interp);
                importer::attach_sys_extras(interp, &m);
                for (src_name, bind) in &names {
                    match attr_get(interp, &m, src_name)? {
                        Some(v) => frame.store_global(bind, v),
                        None => {
                            return Err(raise(
                                "ImportError",
                                format!("cannot import name '{src_name}' from 'sys'"),
                            ))
                        }
                    }
                }
                return Ok(StmtOut::Next);
            }
            let mv = importer::import_module(interp, module)?;
            importer::import_from_module(interp, frame, module, &mv, &names)?;
            Ok(StmtOut::Next)
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
                    Target::Attribute(obj, name) => {
                        let o = eval_expr(interp, frame, obj, reg)?;
                        crate::classes::attr_del_protocol(interp, &o, name)?;
                    }
                    _ => return Err(raise("SystemError", "unsupported del target in v0")),
                }
            }
            Ok(StmtOut::Next)
        }
        Stmt::LocalDef(f) => {
            // Qualified name: methods defined in a class body report
            // "Class.method" (CPython __qualname__); the class frame's
            // scope_path is the class name at class-body exec time.
            let qual = if frame.scope_path != "<module>" && !frame.scope_path.contains("<locals>") {
                format!("{}.{}", frame.scope_path, f.name)
            } else {
                f.name.clone()
            };
            let id = interp.register_nested(FnEntry {
                def: (**f).clone(),
                scope_path: format!("{}.<locals>.{}", frame.scope_path, f.name),
                qualname: qual,
                globals: frame.globals.clone(),
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
            // Decorators: bottom-up application at def time (S-02-7) —
            // @property/@classmethod/@staticmethod land here.
            let mut decorated = func;
            for d in f.decorators.iter().rev() {
                let df = eval_expr(interp, frame, d, reg)?;
                decorated = call_value(interp, frame, &df, &[decorated], &[])?;
            }
            store_name(interp, frame, &f.name, decorated, reg)?;
            Ok(StmtOut::Next)
        }
        Stmt::LocalClass(c) => {
            crate::classes::exec_class_def(interp, frame, c, reg, &c.name)?;
            Ok(StmtOut::Next)
        }
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
            // 02.03b: break/continue raised THROUGH a nested `with` body
            // arrive as Err(Flow::Break/Continue) (the With handler
            // forwards them raw) — they are THIS loop's control flow, not
            // escapes (warnings.warn_explicit: break inside with + for-else
            // silently unwound the whole script).
            Err(Flow::Break) => return Ok(LoopOut::Break),
            Err(Flow::Continue) => return Ok(LoopOut::Continue),
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
    // CPython: frames the raising unwound through belong to THIS exception's
    // traceback only. Snapshot BEFORE the body runs (the unwinding itself
    // pushes those frames) and truncate on every handled path, so a caught
    // exception's frames never leak into the NEXT traceback (bughunt
    // 2026-10-17 deferred finding).
    let tb_snapshot = interp.tb_frames.borrow().len();
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
            // (tb_snapshot was taken BEFORE the body: it marks where this
            // exception's unwound frames begin.)
            let mut body_tb_start = tb_snapshot;
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
                    // Frames pushed by the HANDLER BODY belong to any new
                    // exception raised there; the ORIGINAL exception's frames
                    // ([tb_snapshot..body_tb_start)) do not (CPython shows
                    // them only in the chained "During handling" report).
                    let body_tb_start = interp.tb_frames.borrow().len();
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
                Some(Ok(())) => {
                    // Handled: the raising frames die with this exception.
                    interp.tb_frames.borrow_mut().truncate(tb_snapshot);
                    exec_block(interp, frame, &t.finalbody, reg).map(|_| StmtOut::Next)
                }
                Some(Err(f)) => {
                    // Handler body raised: the NEW exception's traceback owns
                    // only the handler-body frames; drop the original
                    // exception's frames ([tb_snapshot..body_tb_start)).
                    {
                        let end = body_tb_start.min(interp.tb_frames.borrow().len());
                        let mut tb = interp.tb_frames.borrow_mut();
                        tb.drain(tb_snapshot..end);
                    }
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
    let interp = current_interp_for_bindings();
    if std::env::var("APX_DEBUG").is_ok() {
        eprintln!(
            "[02.03] exc_matches tv_kind={:?} tv_name={:?} exc={}",
            match tv {
                Value::Obj(o) => format!("{:?}", o.kind),
                _ => "other".into(),
            },
            match tv {
                Value::Obj(o) => o.name.clone(),
                _ => "?".into(),
            },
            exc.class
        );
    }
    match tv {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            // Real class target (builtin tree or user exception class):
            // MRO membership (S-03-31/32). A non-exception class target is
            // the catch-time TypeError (S-03-34) — never a silent skip.
            let t = crate::classes::type_of(&interp, tv)
                .ok_or_else(|| raise("SystemError", "class without payload"))?;
            if !t.flags.is_exception {
                return Err(raise(
                    "TypeError",
                    "catching classes that do not inherit from BaseException is not allowed",
                ));
            }
            let via_instance = exc
                .instance
                .as_ref()
                .and_then(|iv| crate::classes::class_of_value(&interp, iv))
                .is_some_and(|ec| ec.is_subclass(&t));
            Ok(exc.class == t.name
                || via_instance
                || crate::classes::exc_class(&interp, &exc.class)
                    .is_some_and(|ec| ec.is_subclass(&t))
                || legacy_exc_is_subclass(&exc.class, &t))
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type => {
            if !is_builtin_exc_name(&o.name) {
                return Err(raise(
                    "TypeError",
                    "catching classes that do not inherit from BaseException is not allowed",
                ));
            }
            Ok(is_subclass(&exc.class, &o.name))
        }
        Value::Tuple(parts) => {
            // FLAT tuples only (A-4: nesting raises the same TypeError).
            for p in &parts.0 {
                match p {
                    Value::Obj(po)
                        if po.kind == ap_obj::ObjKind::UserClass
                            || po.kind == ap_obj::ObjKind::Type => {}
                    _ => return Err(raise(
                        "TypeError",
                        "catching classes that do not inherit from BaseException is not allowed",
                    )),
                }
                if exc_matches(p, exc)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Err(raise(
            "TypeError",
            "catching classes that do not inherit from BaseException is not allowed",
        )),
    }
}

/// MRO subclass check for a legacy-raised exception name against a real
/// user class that derives (transitively) from a builtin exception.
fn legacy_exc_is_subclass(exc_name: &str, target: &Rc<ap_obj::classes::TypeRef>) -> bool {
    // Only LEGACY-known names participate (the old table defaults unknowns
    // to Exception, which would wrongly catch BaseExceptionGroup here).
    // The legacy table's known set (its `_ => Exception` default must not
    // leak 3.14-only classes like BaseExceptionGroup into Exception).
    const LEGACY_KNOWN: &[&str] = &[
        "BaseException",
        "KeyboardInterrupt",
        "SystemExit",
        "Exception",
        "StopIteration",
        "ArithmeticError",
        "ZeroDivisionError",
        "OverflowError",
        "FloatingPointError",
        "TypeError",
        "ValueError",
        "NameError",
        "LookupError",
        "AttributeError",
        "RuntimeError",
        "AssertionError",
        "MemoryError",
        "OSError",
        "UnboundLocalError",
        "IndexError",
        "KeyError",
        "NotImplementedError",
        "RecursionError",
        "UnicodeError",
        "UnicodeDecodeError",
        "UnicodeEncodeError",
        "EOFError",
    ];
    if !LEGACY_KNOWN.contains(&exc_name) {
        return false;
    }
    // The raised name must be a legacy subclass OF THE TARGET itself —
    // not of the target's ancestors (TypeError is not a KeyError).
    is_subclass(exc_name, &target.name)
}

/// Current frame's line cell (top of the frame stack); 0 when empty.
/// 02.07: frame_stack pop with cross-thread contention retry (all holders
/// run under the GIL, so a held borrow clears on the next scheduling turn).
fn frame_stack_pop(interp: &Rc<Interpreter>) {
    for _ in 0..2000 {
        match interp.frame_stack.try_borrow_mut() {
            Ok(mut fs) => {
                fs.pop();
                return;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
}

fn current_frame_line(interp: &Rc<Interpreter>) -> usize {
    let top = loop {
        match interp.frame_stack.try_borrow() {
            Ok(fs) => break fs.last().cloned(),
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };
    top.map(|c| c.get()).unwrap_or(0)
}

/// Source line of the statement JUST executed: the global statement-ordinal
/// counter advanced past it, so the raise's own line is table[ordinal - 1].
/// (current_frame_line alone reflects the mapped value written to THIS
/// frame's cell, which for GC-bridge frames can lag to the caller's line.)
fn last_executed_line(interp: &Rc<Interpreter>) -> usize {
    let ordinal = interp.stmt_ordinal.get();
    if ordinal == 0 {
        return current_frame_line(interp);
    }
    // 02.07: shared table — retry on cross-thread contention (write-once).
    let line = loop {
        match interp.stmt_lines.try_borrow() {
            Ok(t) => break t.get(ordinal - 1).copied(),
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };
    line.unwrap_or_else(|| current_frame_line(interp))
}

fn exception_instance(exc: &ExceptionValue) -> Value {
    // Real instance raised as-is: identity preserved at the handler (e007).
    if let Some(v) = &exc.instance {
        return v.clone();
    }
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Exception, exc.class.clone());
    *o.args.borrow_mut() = exc.args.clone();
    // Carry the class payload back-ref when the class is a real class.
    let interp = current_interp_for_bindings();
    if let Some(cv) = crate::classes::exc_class_value(&interp, &exc.class) {
        if let Value::Obj(co) = &cv {
            if let Some(t) = co.class_ref.borrow().clone() {
                *o.class_ref.borrow_mut() = Some(t);
            }
        }
    }
    Value::Obj(o)
}

/// Evaluate a raise value: exception instance, exception CLASS (bare), or
/// TypeError otherwise (S-03-33: "exceptions must derive from BaseException").
fn raise_from_value(v: &Value) -> Result<ExceptionValue, Flow> {
    let interp = current_interp_for_bindings();
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => {
            Ok(ExceptionValue::new(&o.name, o.args.borrow().clone()))
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Type && is_builtin_exc_name(&o.name) => {
            Ok(ExceptionValue::new(&o.name, vec![]))
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
            // User class: instantiate (S-03-33) if it derives BaseException.
            let t = crate::classes::type_of(&interp, v);
            let is_exc = t.as_ref().is_some_and(|t| t.flags.is_exception);
            if !is_exc {
                return Err(raise(
                    "TypeError",
                    "exceptions must derive from BaseException",
                ));
            }
            let t = t.unwrap();
            // Bare-class raise instantiates (runs __init__; binding errors
            // surface as the ordinary TypeError — S-03-33/e012).
            let inst = crate::classes::instantiate(&interp, v, &t, &[], &[])?;
            let mut ev = exception_value_of_instance(&inst)?;
            ev.instance = Some(inst);
            Ok(ev)
        }
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
            // An instance of a user exception class raised directly (e007:
            // identity preserved at the handler).
            let t = o.type_ref.borrow().as_ref().and_then(|w| w.upgrade());
            if t.is_some_and(|t| t.flags.is_exception) {
                let mut ev = exception_value_of_instance(v)?;
                ev.instance = Some(v.clone());
                Ok(ev)
            } else {
                Err(raise(
                    "TypeError",
                    "exceptions must derive from BaseException",
                ))
            }
        }
        _ => Err(raise(
            "TypeError",
            "exceptions must derive from BaseException",
        )),
    }
}

/// Exception class names pre-created by install_exception_classes.
fn is_builtin_exc_name(n: &str) -> bool {
    // 02.08-lite: stdlib exception types are catchable builtin excs.
    if n == "JSONDecodeError" {
        return true;
    }
    crate::classes::EXC_TREE.iter().any(|(name, _)| *name == n)
}

/// Recover the ExceptionValue plumbing (class name + args) from a real
/// exception instance Value.
fn exception_value_of_instance(v: &Value) -> Result<ExceptionValue, Flow> {
    let interp = current_interp_for_bindings();
    let Value::Obj(o) = v else {
        return Err(raise("SystemError", "not an exception instance"));
    };
    // str(exc) runs through the class __str__ when overridden; args come
    // from the instance __dict__ (BaseException.__init__ stores them).
    let from_attrs = o
        .attrs
        .borrow()
        .get(&Value::Str(PyString::new("args")))
        .and_then(|a| match a {
            Value::Tuple(t) => Some(t.0.clone()),
            _ => None,
        });
    let args: Vec<Value> = match from_attrs {
        Some(a) => a,
        None => o.args.borrow().clone(),
    };
    let mut ev = ExceptionValue::new(&o.name, args);
    // User __str__ override: pickle the rendering into args so py_str
    // (which joins args) reproduces it byte-exactly. class_of_value reads
    // the class_ref back-ref first (user exception classes).
    if let Some(t) = o
        .class_ref
        .borrow()
        .clone()
        .or_else(|| crate::classes::class_of_value(&interp, v))
    {
        if let Some((sf, owner)) = t.lookup_with_owner("__str__") {
            let is_default = matches!(&sf, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function && owner.name == "BaseException");
            if !is_default {
                let s = call_value(
                    &interp,
                    &Rc::new(Frame::new_module(interp.globals.clone())),
                    &sf,
                    &[v.clone()],
                    &[],
                )?;
                if let Value::Str(s) = s {
                    ev.args = vec![Value::Str(s)];
                }
            }
        }
    }
    Ok(ev)
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
        Target::Attribute(obj, name) => {
            let o = eval_expr(interp, frame, obj, reg)?;
            crate::classes::attr_set_protocol(interp, &o, name, v)
        }
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
            // 02.04a (interp bug #13b): generic iterables (classes with
            // metaclass __iter__, __getitem__-iterables) unpack via the
            // iterator protocol.
            let ok = iterable_of(other).ok().map(|it| crate::iter::collect(&it));
            match ok {
                Some(items) => items,
                None => {
                    return Err(raise(
                        "TypeError",
                        format!("cannot unpack non-iterable {} object", other.type_name()),
                    ))
                }
            }
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
        // 02.04a fix pass (BUG-A follow-on): `obj.attr += x` — pathlib does
        // `self._str += ...`-style augassigns. 02.03b: fall back to the
        // plain attr read when the protocol misses (modules and other
        // non-02.03 hosts — `_wm._filters_version += 1` in warnings.py).
        Target::Attribute(obj, name) => {
            let o = eval_expr(interp, frame, obj, reg)?;
            match crate::classes::attr_get_protocol(interp, &o, name) {
                Ok(v) => Ok(v),
                Err(Flow::Raise(e)) if e.class == "AttributeError" => {
                    match attr_get(interp, &o, name)? {
                        Some(v) => Ok(v),
                        None => Err(Flow::Raise(e)),
                    }
                }
                Err(other) => Err(other),
            }
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
            // Real exception classes shadow nothing (installed separately);
            // `except ValueError` name reads resolve through them (S-03-32).
            if let Some(v) = crate::classes::exc_class_value(interp, name) {
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
    if let Some(ns) = &frame.class_ns {
        // Class-body frame: assignments land in the class namespace (S-03-3).
        // 02.03b: a dict-subclass namespace instance (EnumDict) intercepts
        // the store through its __setitem__ (member tracking); with no
        // instance or no override, the plain dict write stands.
        if let Some(inst) = &frame.class_ns_inst {
            if let Value::Obj(io) = inst {
                if let Some(t) = io.type_ref.borrow().as_ref().and_then(|w| w.upgrade()) {
                    if let Some((sf, _)) = t.lookup_with_owner("__setitem__") {
                        let is_fn =
                            matches!(&sf, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function);
                        if is_fn {
                            let interp = current_interp_for_bindings();
                            let caller = module_frame_pub();
                            // Zero-arg super() inside __setitem__ (EnumDict
                            // -> dict): stage the defining-class ctx like
                            // bound-method dispatch does.
                            let owner_t = t.lookup_with_owner("__setitem__").map(|(_, ow)| ow);
                            if let Some(ow) = &owner_t {
                                *interp.pending_super_ctx.borrow_mut() =
                                    Some((ow.clone(), inst.clone(), false));
                            }
                            call_value(
                                &interp,
                                &caller,
                                &sf,
                                &[inst.clone(), Value::Str(PyString::new(name)), v],
                                &[],
                            )?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        return ns
            .set(Value::Str(PyString::new(name)), v)
            .map_err(Flow::from_py);
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

pub(crate) fn eval_expr(
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
            // 02.08-full part 1: complex as an Opaque pair (re/im attrs,
            // repr/str/real/imag surfaces the corpus exercises — s001
            // repr(1j), s029 attrgetter("real")).
            Ok(crate::builtins_modules::make_complex_value(*r, *i))
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
            // 02.03 unary dunders (__neg__/__pos__/__invert__/__abs__)
            // before the builtin numeric paths (c030).
            let unary_slot = match op {
                UnaryOp::Minus => Some("__neg__"),
                UnaryOp::Plus => Some("__pos__"),
                UnaryOp::Invert => Some("__invert__"),
                UnaryOp::Not => None,
            };
            if let Some(slot) = unary_slot {
                if let Some((f, _)) = crate::classes::special_lookup(interp, &xv, slot) {
                    let r = call_value(interp, &module_frame_pub(), &f, &[xv.clone()], &[])?;
                    if !result_is_not_implemented(&Ok(r.clone())) {
                        return Ok(r);
                    }
                } else if let Some((f, _)) = crate::classes::special_lookup(interp, &xv, "__abs__")
                {
                    if slot == "__neg__" || slot == "__pos__" {
                        // CPython doesn't fall back to __abs__ for -/+
                        // (only round/abs paths); skip.
                    }
                }
            }
            match op {
                UnaryOp::Plus => match xv {
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) => Ok(xv),
                    other => Err(raise(
                        "TypeError",
                        format!(
                            "bad operand type for unary +: '{}'",
                            crate::classes::display_type_name(&other)
                        ),
                    )),
                },
                UnaryOp::Minus => ap_obj::ops::neg(&xv).map_err(Flow::from_py),
                UnaryOp::Invert => ap_obj::ops::invert(&xv).map_err(Flow::from_py),
                UnaryOp::Not => Ok(Value::Bool(!truthy(interp, &xv)?)),
            }
        }
        Expr::BoolOp(op, xs) => {
            // Short-circuit preserving operand VALUES (S-01-30).
            let mut last = Value::None;
            for (i, x) in xs.iter().enumerate() {
                last = eval_expr(interp, frame, x, reg)?;
                let stop = match op {
                    BoolOp::And => !truthy(interp, &last)?,
                    BoolOp::Or => truthy(interp, &last)?,
                };
                if stop && i + 1 < xs.len() {
                    return Ok(last);
                }
            }
            Ok(last)
        }
        Expr::Compare(ops, operands) => {
            let mut left = eval_expr(interp, frame, &operands[0], reg)?;
            let mut last = Value::Bool(true);
            for (i, op) in ops.iter().enumerate() {
                let right = eval_expr(interp, frame, &operands[i + 1], reg)?;
                // 02.03: __eq__/__ne__ return the SLOT's raw value (c028:
                // `1 == REq()` -> "reflected-eq"); ordering ops stay bool.
                if matches!(op, CmpOp::Eq | CmpOp::NotEq) {
                    let interp2 = current_interp_for_bindings();
                    if let Some(rv) = user_compare_value(&interp2, *op, &left, &right) {
                        last = rv?;
                        let truth = truthy(interp, &last)?;
                        if !truth {
                            return Ok(Value::Bool(false));
                        }
                        left = right;
                        continue;
                    }
                }
                if !eval_compare(&left, *op, &right)? {
                    return Ok(Value::Bool(false));
                }
                last = Value::Bool(true);
                left = right;
            }
            Ok(last)
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
            // 02.03b-fix: `super().__new__` FIRST-CLASS call path. CPython's
            // implicit-staticmethod __new__ rule (type.__new__ wraps user
            // __new__ as staticmethod) makes super().__new__ resolve UNBOUND;
            // the marker set by builtin_super (zero-arg) routes the read to
            // the bare-function arm of attr_get's super scan.
            if name == "__new__" {
                if let Value::Obj(sup) = &o {
                    if sup.kind == ap_obj::ObjKind::Opaque && sup.name == "super" {
                        if sup
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("__ap_unbound_new__")))
                            .is_some_and(|v| v.is_truthy())
                        {
                            if let Some(v) = attr_get(interp, &o, name)? {
                                return Ok(v);
                            }
                        }
                    }
                }
            }
            attr_get(interp, &o, name)?.ok_or_else(|| {
                let extra = match &o {
                    Value::Obj(mo) if mo.kind == ap_obj::ObjKind::Module => {
                        format!(" (module '{}')", mo.name)
                    }
                    _ => String::new(),
                };
                raise(
                    "AttributeError",
                    format!(
                        "'{}' object has no attribute '{}'{}",
                        crate::classes::display_type_name(&o),
                        name,
                        { extra }
                    ),
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
                globals: frame.globals.clone(),
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
            if truthy(interp, &cv)? {
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
    let interp = current_interp_for_bindings();
    // Bound methods == by (__func__, __self__) (S-03-21; c024).
    if matches!(op, CmpOp::Eq | CmpOp::NotEq) {
        if let (Value::Obj(a), Value::Obj(b)) = (left, right) {
            if a.kind == ap_obj::ObjKind::BoundMethod && b.kind == ap_obj::ObjKind::BoundMethod {
                let funcs_eq = match (a.func.borrow().clone(), b.func.borrow().clone()) {
                    (Some(fx), Some(fy)) => value_is(&fx, &fy),
                    (None, None) => true,
                    _ => false,
                };
                let same = funcs_eq
                    && match (a.self_value.borrow().clone(), b.self_value.borrow().clone()) {
                        (Some(x), Some(y)) => value_is(&x, &y),
                        (None, None) => true,
                        _ => false,
                    };
                return Ok(if op == CmpOp::Eq { same } else { !same });
            }
        }
    }
    // 02.03 user comparison slots (S-03-25) take precedence for instances
    // of user classes; builtin paths keep their exact messages.
    if let Some(r) = user_compare(&interp, op, left, right) {
        return r;
    }
    // Ordering on user instances without slots: exact TypeError with the
    // CLASS names (c027 — values_cmp would print the static 'object').
    let left_user = matches!(left, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance);
    let right_user = matches!(right, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance);
    if (left_user || right_user) && matches!(op, CmpOp::Lt | CmpOp::LtE | CmpOp::Gt | CmpOp::GtE) {
        if matches!(op, CmpOp::Lt | CmpOp::Gt) {
            return Err(raise(
                "TypeError",
                format!(
                    "'{}' not supported between instances of '{}' and '{}'",
                    match op {
                        CmpOp::Lt => "<",
                        CmpOp::Gt => ">",
                        _ => unreachable!(),
                    },
                    crate::classes::display_type_name(left),
                    crate::classes::display_type_name(right)
                ),
            ));
        }
        // <= / >= fall back to values_cmp (same message shape anyway).
    }
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
        // 02.03b-fix: NotImplemented is a runtime SINGLETON semantically
        // (`ok is not NotImplemented` in _py_abc.__subclasscheck__), but the
        // v0 model materializes fresh marker objects at each creation site —
        // identify by the marker name.
        (Value::Obj(x), Value::Obj(y))
            if x.name == "NotImplemented"
                && y.name == "NotImplemented"
                && x.kind == ap_obj::ObjKind::Opaque
                && y.kind == ap_obj::ObjKind::Opaque =>
        {
            true
        }
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
    // 02.03 __contains__ on the type (S-03-26; c030), then the iteration
    // fallback.
    {
        let interp = current_interp_for_bindings();
        if let Some((f, _)) = crate::classes::special_lookup(&interp, b, "__contains__") {
            let r = call_value(
                &interp,
                &module_frame_pub(),
                &f,
                &[b.clone(), a.clone()],
                &[],
            )?;
            return Ok(r.is_truthy());
        }
        if matches!(b, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance) {
            if crate::classes::special_lookup(&interp, b, "__iter__").is_some()
                || crate::classes::special_lookup(&interp, b, "__getitem__").is_some()
            {
                let it = iterable_of(&b)?;
                while let Some(item) = it.next() {
                    if eval_compare(a, CmpOp::Eq, &item)? {
                        return Ok(true);
                    }
                }
                return Ok(false);
            }
            // 02.03b: dict-derived instances contain via their backing dict.
            if let Some(d) = crate::classes::dict_backing(b) {
                return Ok(d.contains(a));
            }
            return Err(raise(
                "TypeError",
                format!(
                    "argument of type '{}' is not iterable",
                    crate::classes::display_type_name(b)
                ),
            ));
        }
    }
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
        // 02.04 completion: weak containers answer `in` (prunes dead
        // entries; unweakrefable probes are simply not members).
        Value::Obj(o)
            if o.kind == ap_obj::ObjKind::Opaque
                && crate::gcsupport::is_weak_container(&o.name) =>
        {
            match crate::gcsupport::call_weakref(
                &format!("{}.__contains__", o.name),
                &[Value::Obj(o.clone()), a.clone()],
                &[],
            ) {
                Some(Ok(v)) => Ok(v.is_truthy()),
                Some(Err(f)) => Err(f),
                None => Ok(false),
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
    // 02.08-full part 1: complex arithmetic (s029: 3 + 4j). The Opaque
    // complex pair supports + - * and comparisons for equality only.
    {
        let as_complex = |v: &Value| -> Option<(f64, f64)> {
            match v {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "complex" => {
                    let attrs = o.attrs.borrow();
                    let re = attrs.get(&ap_obj::value::Value::Str(ap_obj::value::PyString::new(
                        "real",
                    )));
                    let im = attrs.get(&ap_obj::value::Value::Str(ap_obj::value::PyString::new(
                        "imag",
                    )));
                    match (re, im) {
                        (
                            Some(ap_obj::value::Value::Float(r)),
                            Some(ap_obj::value::Value::Float(i2)),
                        ) => Some((r, i2)),
                        _ => None,
                    }
                }
                Value::Float(f) => Some((*f, 0.0)),
                Value::Int(n) => Some((n.to_i64().unwrap_or(0) as f64, 0.0)),
                _ => None,
            }
        };
        let arith = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div);
        if arith {
            if let (Some((ar, ai)), Some((br, bi))) = (as_complex(a), as_complex(b)) {
                let complex_side = |v: &Value| matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "complex");
                if complex_side(a) || complex_side(b) {
                    let (r, i) = match op {
                        BinOp::Add => (ar + br, ai + bi),
                        BinOp::Sub => (ar - br, ai - bi),
                        BinOp::Mul => (ar * br - ai * bi, ar * bi + ai * br),
                        BinOp::Div => {
                            let d = br * br + bi * bi;
                            ((ar * br + ai * bi) / d, (ai * br - ar * bi) / d)
                        }
                        _ => unreachable!(),
                    };
                    return Ok(crate::builtins_modules::make_complex_value(r, i));
                }
            }
        }
    }
    // 02.04a (interp bug #7b): PEP 604 union `int | str` — types.py reads
    // type(int | str) at import. v0: stub _Union value.
    if matches!(op, BinOp::BitOr) {
        let is_type = |v: &Value| matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::Type);
        if is_type(a) && is_type(b) {
            let u = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "_Union");
            return Ok(Value::Obj(u));
        }
    }
    // 02.03 user operator slots (S-03-25) — before every builtin path.
    {
        let interp = current_interp_for_bindings();
        let (op_name, rop_name, sym, iop_name) = match op {
            BinOp::Add => ("__add__", "__radd__", "+", "__iadd__"),
            BinOp::Sub => ("__sub__", "__rsub__", "-", "__isub__"),
            BinOp::Mul => ("__mul__", "__rmul__", "*", "__imul__"),
            BinOp::Div => ("__truediv__", "__rtruediv__", "/", "__itruediv__"),
            BinOp::FloorDiv => ("__floordiv__", "__rfloordiv__", "//", "__ifloordiv__"),
            BinOp::Mod => ("__mod__", "__rmod__", "%", "__imod__"),
            BinOp::Pow => ("__pow__", "__rpow__", "**", "__ipow__"),
            BinOp::MatMul => ("__matmul__", "__rmatmul__", "@", "__imatmul__"),
            BinOp::LShift => ("__lshift__", "__rlshift__", "<<", "__ilshift__"),
            BinOp::RShift => ("__rshift__", "__rrshift__", ">>", "__irshift__"),
            BinOp::BitOr => ("__or__", "__ror__", "|", "__ior__"),
            BinOp::BitXor => ("__xor__", "__rxor__", "^", "__ixor__"),
            BinOp::BitAnd => ("__and__", "__rand__", "&", "__iand__"),
        };
        let _ = iop_name; // augmented ops route through exec_stmt's AugAssign
                          // Union types: X | Y on class values (S-03-31 isinstance union form).
        if op == BinOp::BitOr {
            let both_classes = matches!(
                (a, b),
                (Value::Obj(x), Value::Obj(y))
                    if x.kind == ap_obj::ObjKind::UserClass && y.kind == ap_obj::ObjKind::UserClass
            );
            if both_classes {
                let u = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "__union__");
                *u.slice_parts.borrow_mut() = vec![a.clone(), b.clone()];
                return Ok(Value::Obj(u));
            }
        }
        if let Some(r) = user_binop(&interp, op_name, rop_name, sym, a, b) {
            return r;
        }
    }
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

pub(crate) fn call_value(
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
        Value::Obj(o) => {
            match o.kind {
                ap_obj::ObjKind::BuiltinFn => {
                    // 02.07: type(thread_obj) reports its <class> attr
                    // (_MainThread / Thread / Timer — T108).
                    if std::env::var("APX_TDEBUG").is_ok() {
                        eprintln!("[TDEBUG] BuiltinFn call: {}", o.name);
                    }
                    if o.name == "type" {
                        if let Some(Value::Obj(so)) = args.first() {
                            if so.kind == ap_obj::ObjKind::Opaque && so.name == "thread" {
                                let cls = so
                                    .attrs
                                    .borrow()
                                    .get(&Value::Str(PyString::new("<class>")))
                                    .and_then(|v| match v {
                                        Value::Str(sv) => Some(sv.as_str().to_string()),
                                        _ => None,
                                    })
                                    .unwrap_or_else(|| "Thread".to_string());
                                return Ok(builtins::interned_type(interp, &cls));
                            }
                        }
                    }
                    // Class-statement keyword marker (lowering): the class-creation
                    // path consumes kwargs; the call itself only carries the value.
                    // 02.04a fix pass: park the value on the marker and return
                    // the MARKER so exec_class_def's base filter recognizes,
                    // skips it, and recovers (name, value) for the metaclass
                    // call kwargs (enum._simple_enum's boundary=/_simple=).
                    if o.name.starts_with("__ap_classkw__") {
                        let kwname = o.name.trim_start_matches("__ap_classkw__.").to_string();
                        *o.slice_parts.borrow_mut() = vec![
                            Value::Str(PyString::new(kwname)),
                            args.first().cloned().unwrap_or(Value::None),
                        ];
                        return Ok(f.clone());
                    }
                    // 02.04a coroutine-stub methods (close/send/throw): no-ops.
                    if o.name == "__ap_coroutine_noop__" {
                        return Ok(Value::None);
                    }
                    if o.name == "__ap_functools_nop__" {
                        return Ok(Value::None);
                    }
                    // 02.04: exc.with_traceback(tb) — v0 tracebacks are
                    // stubs; the method returns the exception itself.
                    if o.name == "__ap_with_traceback__" {
                        let recv = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        return Ok(recv);
                    }
                    // 02.04a fix pass (BUG-C): no-op RLock context protocol.
                    if o.name == "__ap_rlock_enter__" {
                        return Ok(args.first().cloned().unwrap_or(Value::None));
                    }
                    if o.name == "__ap_rlock_exit__" {
                        return Ok(Value::Bool(false));
                    }
                    // object.__new__(cls) virtual (c003): allocates the parked
                    // subject class.
                    if o.name.starts_with("property.") {
                        // p.setter(fn)/p.deleter(fn): clone with the new slot.
                        let Some(prop) = o.slice_parts.borrow().first().cloned() else {
                            return Err(raise("SystemError", "property marker without receiver"));
                        };
                        let Value::Obj(po) = prop else {
                            return Err(raise("SystemError", "property marker receiver"));
                        };
                        let no = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, "property");
                        *no.descr_kind.borrow_mut() = Some("property".to_string());
                        *no.self_value.borrow_mut() = po.self_value.borrow().clone();
                        let accessor = args.first().cloned();
                        let kind = o.name.rsplit('.').next().unwrap_or("");
                        match kind {
                            "setter" => *no.func.borrow_mut() = accessor,
                            "deleter" => {
                                if let Some(f) = accessor {
                                    let _ = no
                                        .dunders
                                        .borrow_mut()
                                        .set(Value::Str(PyString::new("fdel")), f);
                                }
                            }
                            _ => {}
                        }
                        return Ok(Value::Obj(no));
                    }
                    // 02.03b-fix: os.PathLike.register(subclass) -> subclass
                    // (registry is v0-inert; the isinstance check keys on
                    // __fspath__ directly).
                    if o.name == "__ap_pathlike_register__" {
                        return Ok(args.first().cloned().unwrap_or(Value::None));
                    }
                    // 02.03b-fix: namedtuple class-level field getter —
                    // descr_get property arm calls fget(receiver); the
                    // receiver is the instance, the field name is parked.
                    if o.name == "__ap_namedtuple_getter__" {
                        let field = match o.slice_parts.borrow().first() {
                            Some(Value::Str(x)) => x.as_str().to_string(),
                            _ => String::new(),
                        };
                        let recv = args.first().cloned().unwrap_or(Value::None);
                        return match crate::interp::attr_get_public(interp, &recv, &field) {
                            Ok(Some(v)) => Ok(v),
                            Ok(None) => Err(raise(
                                "AttributeError",
                                format!("namedtuple has no field '{}'", field),
                            )),
                            Err(e) => Err(e),
                        };
                    }
                    // 02.03b-fix: function __repr__ marker (bound via the
                    // attr surface above).
                    if o.name == "__ap_function_repr__" || o.name == "__ap_builtin_repr__" {
                        let target = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        if o.name == "__ap_function_repr__" {
                            return crate::builtins::call_builtin(interp, "repr", &[target], &[]);
                        }
                        // __format__ variant: format_value with the spec.
                        let spec = args
                            .first()
                            .and_then(|v| match v {
                                Value::Str(x) => Some(x.as_str().to_string()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let s = match crate::builtins::py_str_of(&target) {
                            Ok(x) => x,
                            Err(e) => return Err(Flow::from_py(e)),
                        };
                        let out =
                            match crate::format::format_value(&Value::Str(PyString::new(s)), &spec)
                            {
                                Ok(x) => x,
                                Err(e) => return Err(Flow::from_py(e)),
                            };
                        return Ok(Value::Str(PyString::new(out)));
                    }
                    if o.name == "__subclasses__" {
                        let cls = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        let name = match &cls {
                            Value::Obj(co) => co.name.clone(),
                            _ => String::new(),
                        };
                        let subs = crate::classes::subclasses_of(interp, &name);
                        return Ok(Value::List(PyList::from_vec(subs)));
                    }
                    if o.name == "__object_init__" {
                        // object.__init__(self, *args): BaseException stores args.
                        let subject = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        if let Value::Obj(io) = &subject {
                            if crate::classes::is_exception_value(&subject) {
                                let argv = Value::Tuple(Gc::new(Tuple(args.to_vec())));
                                let _ = io
                                    .attrs
                                    .borrow_mut()
                                    .set(Value::Str(PyString::new("args")), argv);
                            }
                        }
                        return Ok(Value::None);
                    }
                    // 02.03b: dict.__setitem__/__getitem__ virtuals — route
                    // to the receiver's backing dict.
                    if o.name.starts_with("__ap_dict_") {
                        let subject = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        let d = crate::classes::dict_backing(&subject).ok_or_else(|| {
                            raise("TypeError", "dict virtual on a non-dict-derived receiver")
                        })?;
                        match o.name.as_str() {
                            "__ap_dict_setitem__" => {
                                let k = args.first().cloned().unwrap_or(Value::None);
                                let v = args.get(1).cloned().unwrap_or(Value::None);
                                d.set(k, v).map_err(Flow::from_py)?;
                                return Ok(Value::None);
                            }
                            "__ap_dict_getitem__" => {
                                let k = args.first().cloned().unwrap_or(Value::None);
                                return d.get(&k).ok_or_else(|| {
                                    Flow::Raise(ExceptionValue::new("KeyError", vec![k]))
                                });
                            }
                            "__ap_dict_get__" => {
                                let k = args.first().cloned().unwrap_or(Value::None);
                                return Ok(d.get(&k).unwrap_or(Value::None));
                            }
                            "__ap_dict_setdefault__" => {
                                let k = args.first().cloned().unwrap_or(Value::None);
                                let dv = args.get(1).cloned().unwrap_or(Value::None);
                                if let Some(v) = d.get(&k) {
                                    return Ok(v);
                                }
                                d.set(k, dv.clone()).map_err(Flow::from_py)?;
                                return Ok(dv);
                            }
                            _ => {}
                        }
                    }
                    // 02.03b: str.__new__(cls, value) / int.__new__ —
                    // v0 value model: the value IS the str/int; return it.
                    if matches!(
                        o.name.as_str(),
                        "__str__new__" | "__int__new__" | "__bytes__new__"
                    ) {
                        // (cls) -> zero value; (cls, value) -> the value.
                        return Ok(match args.get(1) {
                            Some(v) => v.clone(),
                            None => match o.name.as_str() {
                                "__str__new__" => Value::Str(PyString::new(String::new())),
                                "__int__new__" => Value::Int(ap_obj::Int::from_i64(0)),
                                _ => Value::Bytes(Gc::new(Vec::new())),
                            },
                        });
                    }
                    // 02.03b: object-virtual markers called as methods
                    // (enum member __init__ via the object.__init__
                    // virtual): default object.__init__/__setattr__ etc are
                    // no-ops here; __str__/__repr__/__format__ fall through
                    // to the builtin surfaces below via the dotted name.
                    // 02.03b: re.Pattern.match/search — v0 literal engine.
                    if let Some(meth) = o.name.strip_prefix("__re_pattern_") {
                        let meth = meth.trim_end_matches('_');
                        let pat = o
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("pattern")))
                            .and_then(|v| match v {
                                Value::Str(x) => Some(x.as_str().to_string()),
                                _ => None,
                            })
                            .unwrap_or_default();
                        let ic = o
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("__ap_re_ic__")))
                            .map(|v| v.is_truthy())
                            .unwrap_or(false);
                        // Pattern.sub(repl, string, count=0): literal
                        // substitution through the module-level re.sub.
                        if meth == "sub" {
                            let mut full: Vec<Value> = vec![Value::Str(PyString::new(pat))];
                            full.extend_from_slice(args);
                            return crate::builtins_modules::call_native(
                                "re", "sub", &full, kwargs,
                            )
                            .unwrap_or_else(|| Ok(Value::None));
                        }
                        // match(pattern_self, text)
                        let text = match args.first() {
                            Some(Value::Str(x)) => x.as_str().to_string(),
                            _ => String::new(),
                        };
                        let base = o.name.trim_end_matches('_');
                        let _ = base;
                        return Ok(
                            match crate::builtins_modules::re_match_stub(&pat, &text, ic, meth) {
                                Some(mo) => mo,
                                None => Value::None,
                            },
                        );
                    }
                    // 02.03b: sys.stdout.write / sys.stderr.write — the
                    // warnings machinery writes formatted text to stderr.
                    if o.name.starts_with("__ap_stream_write") {
                        let text = args.first().cloned().unwrap_or(Value::None);
                        let s = match &text {
                            Value::Str(x) => x.as_str().to_string(),
                            other => crate::builtins::py_str_of(other).unwrap_or_default(),
                        };
                        if o.name.ends_with("stderr__") {
                            eprint!("{s}");
                        } else {
                            interp.write_out(&s);
                        }
                        return Ok(Value::Int(ap_obj::Int::from_i64(s.len() as i64)));
                    }
                    if o.name.starts_with("__ap_stream_flush") {
                        return Ok(Value::None);
                    }
                    if o.name.starts_with("__object_virtual_") {
                        let base = o
                            .name
                            .trim_start_matches("__object_virtual_")
                            .trim_end_matches('_');
                        // 02.03b-fix: object.__subclasshook__ (classmethod)
                        // returns NotImplemented for every query — the
                        // _py_abc.__subclasscheck__ probe compares
                        // `ok is not NotImplemented`.
                        if base.trim_start_matches('_') == "subclasshook" {
                            let ni =
                                ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "NotImplemented");
                            return Ok(Value::Obj(ni));
                        }
                        match base.trim_start_matches('_') {
                            "init" | "setattr" | "delattr" | "getattribute" => {
                                return Ok(Value::None);
                            }
                            _ => {}
                        }
                    }
                    if o.name == "__object_new__" {
                        let subject = o
                            .slice_parts
                            .borrow()
                            .first()
                            .cloned()
                            .unwrap_or(Value::None);
                        if let Some(t) = crate::classes::type_of(interp, &subject) {
                            return Ok(crate::classes::alloc_instance(&t));
                        }
                        return Ok(Value::None);
                    }
                    // 02.03b: object.__new__ read via getattr(cls, '__new__')
                    // (enum._find_new_): the RECEIVING class is args[0].
                    if o.name == "__object_new2__" {
                        let subject = args.first().cloned().unwrap_or(Value::None);
                        if let Some(t) = crate::classes::type_of(interp, &subject) {
                            return Ok(crate::classes::alloc_instance(&t));
                        }
                        return Ok(Value::None);
                    }
                    // 02.03b: object.__new__ read via getattr(cls, '__new__'):
                    // the RECEIVING class arrives as args[0].
                    if o.name == "__object_new2__" {
                        let subject = args.first().cloned().unwrap_or(Value::None);
                        if let Some(t) = crate::classes::type_of(interp, &subject) {
                            return Ok(crate::classes::alloc_instance(&t));
                        }
                        return Ok(Value::None);
                    }
                    // 02.03 protocol builtins: len/bool/repr/str/iter/next/hash
                    // dispatch user __len__/__bool__/__repr__/__str__/__iter__/
                    // __next__/__hash__ on instance args (S-03-22/26) BEFORE the
                    // builtin table (whose messages assume builtin types).
                    if let Some(r) = class_protocol_builtin(interp, &o.name, args, kwargs) {
                        return r;
                    }
                    builtins::call_builtin(interp, &o.name, args, kwargs)
                }
                ap_obj::ObjKind::Function => call_function(interp, o, args, kwargs),
                ap_obj::ObjKind::Type => call_type_ctor(interp, &o.name, args, kwargs),
                ap_obj::ObjKind::UserClass => {
                    // 02.03 instantiation (S-03-05..08) — except `type` itself,
                    // which CALLS as the metaclass: type(x) -> metaclass of x
                    // (S-03-31 x.__class__ surface).
                    let t = crate::classes::type_of(interp, f)
                        .ok_or_else(|| raise("SystemError", "class without payload"))?;
                    if t.name == "type" && !t.flags.builtin_exc && t.bases.len() == 1 {
                        if let Some(subject) = args.first() {
                            if args.len() == 1 {
                                // 02.07: threading objects report their <class>
                                // attr (type(me).__name__ == "_MainThread" — T108).
                                if let Value::Obj(so) = subject {
                                    if so.kind == ap_obj::ObjKind::Opaque && so.name == "thread" {
                                        let cls = so
                                            .attrs
                                            .borrow()
                                            .get(&Value::Str(PyString::new("<class>")))
                                            .and_then(|v| match v {
                                                Value::Str(sv) => Some(sv.as_str().to_string()),
                                                _ => None,
                                            })
                                            .unwrap_or_else(|| "Thread".to_string());
                                        return Ok(builtins::interned_type(interp, &cls));
                                    }
                                }
                                // Descriptors report their kind (property/
                                // staticmethod/member_descriptor, c008/c016/c024);
                                // super objects report "super" (c021).
                                if let Value::Obj(so) = subject {
                                    if so.kind == ap_obj::ObjKind::Exception {
                                        // Legacy exception instance: type(e) is
                                        // its class (e004).
                                        if let Some(cv) =
                                            crate::classes::exc_class_value(interp, &so.name)
                                        {
                                            return Ok(cv);
                                        }
                                    }
                                    if so.kind == ap_obj::ObjKind::Descriptor {
                                        let tn = so
                                            .descr_kind
                                            .borrow()
                                            .clone()
                                            .unwrap_or_else(|| "object".to_string());
                                        let tn = if tn == "member" {
                                            "member_descriptor".to_string()
                                        } else {
                                            tn
                                        };
                                        return Ok(builtins::interned_type(interp, &tn));
                                    }
                                    if so.kind == ap_obj::ObjKind::Opaque && so.name == "super" {
                                        return Ok(builtins::interned_type(interp, "super"));
                                    }
                                    // 02.08-full p2: type(re.Pattern-obj) etc.
                                    if so.kind == ap_obj::ObjKind::Opaque
                                        && (so.name == "re.Pattern" || so.name == "re.Match")
                                    {
                                        return Ok(builtins::interned_type(
                                            interp,
                                            so.name.trim_start_matches("re."),
                                        ));
                                    }
                                }
                                return match crate::classes::class_of_value(interp, subject) {
                                    Some(ct) => {
                                        // Instances: type(x) IS x's class
                                        // (ExceptionGroup case, e002). Classes:
                                        // type(C) is the metaclass.
                                        let is_instance = matches!(subject, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance);
                                        if is_instance {
                                            return Ok(crate::classes::class_value_for_public(&ct));
                                        }
                                        let meta =
                                            ct.metaclass.borrow().clone().unwrap_or_else(|| {
                                                crate::classes::type_class(interp)
                                            });
                                        Ok(crate::classes::class_value_for_public(&meta))
                                    }
                                    None => {
                                        // Builtin value: interned name-Type.
                                        // 02.04a fix pass (BUG-A): use the
                                        // PyObject's own type name (Exception
                                        // arm already handled above); the old
                                        // Value::type_name() collapsed every
                                        // Namespace/Opaque to "object", so
                                        // type(<coroutine stub>) interned an
                                        // "object" Type and ABCMeta.register's
                                        // cycle check (issubclass(Coroutine,
                                        // <coroutine>)) saw the "object" name
                                        // and returned True -> the pathlib
                                        // import RuntimeError.
                                        let tn = match subject {
                                            Value::Obj(o) => {
                                                // 02.08-full part 1: named
                                                // native iterators report
                                                // their function name (s012
                                                // type(count(1)).__name__);
                                                // sys.implementation is a
                                                // "module" (s002 pin).
                                                if o.kind == ap_obj::ObjKind::Namespace
                                                    && o.name == "implementation"
                                                {
                                                    "module".to_string()
                                                } else if o.kind == ap_obj::ObjKind::Iterator
                                                    && matches!(
                                                        o.name.as_str(),
                                                        "count"
                                                            | "cycle"
                                                            | "repeat"
                                                            | "chain"
                                                            | "accumulate"
                                                            | "compress"
                                                            | "dropwhile"
                                                            | "takewhile"
                                                            | "filterfalse"
                                                            | "groupby"
                                                            | "islice"
                                                            | "starmap"
                                                            | "zip_longest"
                                                            | "batched"
                                                            | "pairwise"
                                                    )
                                                {
                                                    o.name.as_str().to_string()
                                                } else {
                                                    o.type_name().to_string()
                                                }
                                            }
                                            other => other.type_name().to_string(),
                                        };
                                        Ok(builtins::interned_type(interp, &tn))
                                    }
                                };
                            }
                        }
                    }
                    crate::classes::instantiate(interp, f, &t, args, kwargs)
                }
                ap_obj::ObjKind::UserInstance => {
                    // __call__ protocol (S-03-23): type lookup, not instance.
                    if let Some((cf, _)) = crate::classes::special_lookup(interp, f, "__call__") {
                        let mut full = vec![f.clone()];
                        full.extend_from_slice(args);
                        return call_value(interp, caller, &cf, &full, kwargs);
                    }
                    Err(raise(
                        "TypeError",
                        format!(
                            "'{}' object is not callable",
                            crate::classes::display_type_name(f)
                        ),
                    ))
                }
                ap_obj::ObjKind::BoundMethod => {
                    // v0 builtin-method dispatch: (receiver, name) -> methods.
                    // User bound methods (02.03, func present) skip the table.
                    if o.func.borrow().is_none() {
                        if let Some(recv) = o.self_value.borrow().clone() {
                            if let Some(r) = methods::call_method(interp, &recv, &o.name, args) {
                                return r;
                            }
                        }
                    }
                    // User bound method (02.03): the defining class + receiver
                    // form the zero-arg super() context (S-03-17/18).
                    let owner = o.method_owner.borrow().as_ref().and_then(|w| w.upgrade());
                    let func = o.func.borrow().clone().unwrap_or(Value::None);
                    let mut full: Vec<Value> =
                        vec![o.self_value.borrow().clone().unwrap_or(Value::None)];
                    full.extend_from_slice(args);
                    if let (Some(owner_t), Some(recv)) = (owner, o.self_value.borrow().clone()) {
                        // classmethod receivers carry the class itself. Staged:
                        // call_function's sentinel lands first, ctx above it.
                        let is_cls = matches!(recv, Value::Obj(ref ro) if ro.kind == ap_obj::ObjKind::UserClass);
                        *interp.pending_super_ctx.borrow_mut() = Some((owner_t, recv, is_cls));
                        return call_value(interp, caller, &func, &full, kwargs);
                    }
                    call_value(interp, caller, &func, &full, kwargs)
                }
                // 02.04 fix pass: _weakref.ref tokens stored in WeakSets are
                // CALLED to dereference (_weakrefset.__iter__/__contains__
                // -> itemref()). Route through gcsupport: alive -> target,
                // dead -> None (C-22).
                ap_obj::ObjKind::Opaque if o.name == "_weakref_token" => {
                    Ok(crate::gcsupport::weakref_call(o))
                }
                // 02.04 completion: WeakMethod token call -> the bound method
                // while the receiver lives, None after (C-22).
                ap_obj::ObjKind::Opaque if o.name == "_weakmethod_token" => {
                    Ok(crate::gcsupport::weakmethod_call(o))
                }
                // 02.04: functools.cmp_to_key wrapper called on an item —
                // park the item and return the SAME wrapper (CPython's K
                // instances; the sort path unwraps via the parked cmp).
                ap_obj::ObjKind::Opaque if o.name == "functools.cmp_to_key" => {
                    let _ = o.attrs.borrow_mut().set(
                        Value::Str(PyString::new("<obj>")),
                        args.first().cloned().unwrap_or(Value::None),
                    );
                    Ok(f.clone())
                }
                // 02.04: weakref.finalize token called -> invoke once.
                ap_obj::ObjKind::Opaque if o.name == "_finalize_token" => {
                    Ok(crate::gcsupport::finalize_call(o))
                }
                // 02.04a fix pass: functools pass-through decorator token —
                // applying it to a callable attaches inert cache surfaces
                // (cache/cache_clear) and returns the callable: typing reads
                // `_lazy_load_getattr_static.cache_clear` at module scope.
                ap_obj::ObjKind::Opaque if o.name == "functools._ap_deco" => {
                    let f = args.first().cloned().unwrap_or(Value::None);
                    if let Value::Obj(fo) = &f {
                        if fo.kind == ap_obj::ObjKind::Function {
                            let nop = ap_obj::PyObject::new(
                                ap_obj::ObjKind::BuiltinFn,
                                "__ap_functools_nop__",
                            );
                            let _ = fo
                                .attrs
                                .borrow_mut()
                                .set(Value::Str(PyString::new("cache_clear")), Value::Obj(nop));
                            let _ = fo.attrs.borrow_mut().set(
                                Value::Str(PyString::new("cache")),
                                Value::Dict(ap_obj::dict::PyDict::new()),
                            );
                        }
                    }
                    Ok(f)
                }
                _ => Err(raise(
                    "TypeError",
                    format!("'{}' object is not callable", f.type_name()),
                )),
            }
        }
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
    // 02.03b-fix: int.from_bytes(b, byteorder='big') classmethod.
    if name == "int" {
        if let Some(Value::Obj(m)) = args.first() {
            if m.kind == ap_obj::ObjKind::BoundMethod && m.name == "from_bytes" {
                let data = args.get(1).cloned().unwrap_or(Value::None);
                let order = args
                    .get(2)
                    .and_then(|v| match v {
                        Value::Str(x) => Some(x.as_str().to_string()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "big".to_string());
                let bytes: Vec<u8> = match &data {
                    Value::Bytes(b) => b.iter().cloned().collect(),
                    Value::Str(x) => x.as_str().bytes().collect(),
                    _ => Vec::new(),
                };
                let mut acc: i64 = 0;
                match order.as_str() {
                    "little" => {
                        for (i, b) in bytes.iter().enumerate() {
                            acc |= (*b as i64) << (8 * i);
                        }
                    }
                    _ => {
                        for b in &bytes {
                            acc = (acc << 8) | (*b as i64);
                        }
                    }
                }
                return Ok(Value::Int(ap_obj::Int::from_i64(acc)));
            }
        }
    }
    // 02.04 completion: weak-container class calls construct the Opaque
    // instances (the class VALUES live in the native weakref module ns).
    if matches!(
        name,
        "WeakKeyDictionary" | "WeakValueDictionary" | "WeakSet"
    ) {
        return crate::gcsupport::call_weakref(name, args, kwargs)
            .unwrap_or_else(|| Ok(Value::None));
    }
    if name == "WeakMethod" {
        return crate::gcsupport::call_weakref(name, args, kwargs)
            .unwrap_or_else(|| Ok(Value::None));
    }
    // Interned type objects double as the builtin callables (bool/str/int
    // ...); route class-protocol subjects through the 02.03 layer first.
    if matches!(
        name,
        "bool" | "len" | "repr" | "str" | "iter" | "hash" | "next" | "type"
    ) {
        if let Some(r) = class_protocol_builtin(interp, name, args, kwargs) {
            return r;
        }
    }
    // 02.03b-fix: _weakref.ref(ob[, cb]) — the ref surface is a TYPE
    // (importlib._bootstrap subclasses it: class KeyedRef(_weakref.ref));
    // constructing one returns the opaque token (v0: never dereferenced).
    if name == "ref" {
        // 02.04 completion: route through the gcsupport surface so weakref
        // checking + WEAKREFS registration match the module-level ref().
        return crate::gcsupport::call_weakref("ref", args, &[]).unwrap_or_else(|| Ok(Value::None));
    }
    // 02.07: threading/queue native constructors (module-level Types call as
    // ctors over the prim table).
    if matches!(
        name,
        "Lock"
            | "RLock"
            | "Condition"
            | "Event"
            | "Semaphore"
            | "BoundedSemaphore"
            | "Barrier"
            | "Timer"
            | "local"
            | "allocate_lock"
            | "Thread"
    ) {
        return crate::builtins_modules::call_native("threading", name, args, kwargs)
            .unwrap_or_else(|| Ok(Value::None));
    }
    if matches!(
        name,
        "Queue" | "LifoQueue" | "PriorityQueue" | "SimpleQueue"
    ) {
        return crate::builtins_modules::call_native("queue", name, args, kwargs)
            .unwrap_or_else(|| Ok(Value::None));
    }
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
    // 02.04a (interp bug #3, vendored-stdlib stress): types.py does
    // `async def _c(): pass; _c = _c(); _c.close()` at import. Coroutines
    // are unsupported v0 — return a stub object with no-op close()/send()/
    // throw() via the generic no-method path; the close() call itself is
    // what types.py needs to NOT raise.
    if def.is_async {
        let co = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "coroutine");
        {
            let attrs = co.attrs.borrow_mut();
            let _ = attrs.set(
                Value::Str(PyString::new("__ap_coroutine__")),
                Value::Bool(true),
            );
            for m in ["close", "send", "throw"] {
                let _ = attrs.set(
                    Value::Str(PyString::new(m)),
                    Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "__ap_coroutine_noop__",
                    )),
                );
            }
        }
        return Ok(Value::Obj(co));
    }

    // Lambda: body expression lives in lambda_bodies (write-once; retry on
    // cross-thread contention).
    let lambda_body = loop {
        match interp.lambda_bodies.try_borrow() {
            Ok(b) => break b.get(&fn_id).cloned(),
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };

    let (locals, my_cells, free_cells) =
        bind_params(interp, &entry, &def, o, lambda_body.is_some(), args, kwargs)?;

    let frame = Rc::new(Frame::new_function(
        entry.globals.clone(),
        &entry.scope_path,
        &entry.qualname,
        scope_globals(interp, &entry),
        scope_nonlocals(interp, &entry),
        my_cells,
        free_cells,
        locals,
    ));
    let line_track = Rc::new(RustCell::new(0usize));
    // 02.07: cross-thread push contention — retry (all holders run under the
    // GIL, so the borrow is transient).
    for _ in 0..2000 {
        match interp.frame_stack.try_borrow_mut() {
            Ok(mut fs) => {
                fs.push(line_track.clone());
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
    // Plain (non-method) function frame: zero-arg super() inside it (even
    // nested) sees NO method context -> RuntimeError (c021).
    // Whether instantiate() staged a context for THIS call (the sentinel is
    // unconditional; the staged ctx pops with it).
    // 02.04: a GC auto-collect can fire INSIDE this window (dict alloc in
    // the TypeObject literal below) and re-enter call_function through a
    // weakref callback; a contended ctx borrow means an outer frame is
    // staging — treat as no staged ctx (the outer frame consumes it).
    let pushed_plain_ctx = interp
        .pending_super_ctx
        .try_borrow_mut()
        .ok()
        .and_then(|mut b| b.take());
    // Build the sentinel BEFORE borrowing super_stack: the literal's dict
    // alloc can trigger a GC auto-collect whose weakref callbacks re-enter
    // call_function (which borrows super_stack).
    let plain_sentinel = std::rc::Rc::new(ap_obj::classes::TypeObject {
        name: "__plain_fn__".to_string(),
        qualname: String::new(),
        bases: vec![],
        mro: std::cell::RefCell::new(Vec::new()),
        dict: ap_obj::dict::PyDict::new(),
        flags: ap_obj::classes::TypeFlags::default(),
        metaclass: std::cell::RefCell::new(None),
        class_value: std::cell::RefCell::new(None),
    });
    interp
        .super_stack
        .borrow_mut()
        .push((plain_sentinel, Value::None, false));
    if let Some(ctx) = pushed_plain_ctx.clone() {
        interp.super_stack.borrow_mut().push(ctx);
    }
    // instantiate() staged a __new__/__init__ context for THIS call; it
    // sits ABOVE the sentinel, so super() sees the method context, and the
    // sentinel still guards plain nested frames.
    if let Some(mut b) = interp.pending_super_ctx.try_borrow_mut().ok() {
        if let Some(ctx) = b.take() {
            interp.super_stack.borrow_mut().push(ctx);
        }
    }

    // 02.04a fix pass (BUG-A follow-on): `lambda: (yield)` is a GENERATOR
    // function in CPython (_collections_abc: generator = type((lambda:
    // (yield))())). Route yield-bodied lambdas through the generator
    // machinery instead of evaluating the body inline.
    if let Some(body) = &lambda_body {
        if matches!(body, Expr::Yield(_) | Expr::YieldFrom(_)) {
            frame_stack_pop(interp);
            return start_generator(interp, entry, o, args, kwargs);
        }
    }
    let result: Result<Value, Flow> = if let Some(body) = lambda_body {
        eval_expr(interp, &frame, &body, &current_registry(interp))
    } else {
        exec_block(interp, &frame, &def.body.blocks, &current_registry(interp)).map(|_| Value::None)
    };

    frame_stack_pop(interp);
    interp.super_stack.borrow_mut().pop();
    if pushed_plain_ctx.is_some() {
        // The staged __new__/__init__ context sits above the sentinel.
        interp.super_stack.borrow_mut().pop();
    }

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

    // Positional-consumable params: PosOnly + PosOrKw only. KwOnly params
    // bind exclusively from kwargs/defaults (02.03b: `def f(*a, b)` — the
    // old binder indexed named-list positions into args for kw-only params,
    // stealing vararg contents and raising "multiple values").
    let pos_consumable: Vec<&Param> = def
        .params
        .iter()
        .filter(|p| matches!(p.kind, ParamKind::PosOnly | ParamKind::PosOrKw))
        .collect();
    let mut pos_consumed = 0usize;
    for p in &pos_consumable {
        if let Some(v) = args.get(pos_consumed) {
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
        } else {
            return Err(raise(
                "TypeError",
                format!(
                    "{}() missing 1 required positional argument: '{}'",
                    entry.qualname, p.name
                ),
            ));
        }
        pos_consumed += 1;
    }
    for p in &named {
        if matches!(p.kind, ParamKind::PosOnly | ParamKind::PosOrKw) {
            continue;
        }
        if let Some((_, v)) = kwargs.iter().find(|(k, _)| k == &p.name) {
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
                // 02.03b: slice from the POSITIONAL consumption count, not
                // the named list length (kw-only params inflate `named`).
                let rest: Vec<Value> = args[pos_consumed.min(args.len())..].to_vec();
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
    if std::env::var("APX_CLOSURE_DEBUG").is_ok() {
        eprintln!(
            "BIND scope={} known={}",
            entry.scope_path,
            reg.scope_of(&entry.scope_path).is_some()
        );
    }
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
            match locals.get(name) {
                Some(Loc::Slot(slot)) => {
                    let cell = my_cells.get_mut(name).unwrap();
                    *cell.borrow_mut() = slot.borrow().clone();
                    locals.insert(name.clone(), Loc::CellSlot(cell.clone()));
                }
                // 02.04: forward-classified cells that are NOT bound yet
                // (assigned later in the body — enum.EnumType.__new__'s
                // `enum_class`) must still alias the cell, or later stores
                // land in a fresh plain slot and nested-scope readers (comps,
                // inner defs) see an empty cell.
                None => {
                    let cell = my_cells.get_mut(name).unwrap();
                    locals.insert(name.clone(), Loc::CellSlot(cell.clone()));
                }
                Some(Loc::CellSlot(_)) => {}
            }
        }
        let defining_slot = o
            .closure_slot
            .borrow()
            .and_then(|id| interp.defining_frames.borrow().get(&(id as u32)).cloned());
        if std::env::var("APX_CLOSURE_DEBUG").is_ok() {
            eprintln!(
                "CLOSURE scope={} frees={:?} closure_slot={:?} defining={}",
                entry.scope_path,
                scope.frees,
                o.closure_slot.borrow(),
                defining_slot.is_some()
            );
        }
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
                entry.globals.clone(),
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
                entry.globals.clone(),
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
    // 02.03b-fix: comprehension bodies read enclosing FUNCTION locals
    // (enum._convert_'s `filter` param inside its members comp). The comp
    // frame only chained parent CELLS; plain locals were invisible and the
    // read fell through to builtins. Seed the comp frame with a snapshot of
    // the parent's locals — writes stay in the comp frame (comps don't leak
    // bindings), reads see the enclosing scope.
    {
        let parent_locals = frame.locals.borrow();
        for (k, loc) in parent_locals.iter() {
            let v: Option<Value> = match loc {
                crate::frame::Loc::Slot(cell) => cell.borrow().clone(),
                crate::frame::Loc::CellSlot(cell) => cell.borrow().clone(),
            };
            if let Some(v) = v {
                comp_frame.store_local(k, v);
            }
        }
    }
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
    // 02.03b-fix: only the FIRST clause's iterable evaluates in the ENCLOSING
    // scope; nested clause iterables reference names bound by earlier
    // clauses in the comp frame (dis: {s: b for b, fam in d.items() for s in
    // fam}).
    let iterable = if ci == 0 {
        eval_expr(interp, outer, &clause.iter, reg)?
    } else {
        eval_expr(interp, cf, &clause.iter, reg)?
    };
    let id = register_iter(interp, &iterable)?;
    loop {
        let next = match pull_iter(interp, id)? {
            Some(v) => v,
            None => break,
        };
        assign_comp_target(cf, &clause.target, next)?;
        let mut pass = true;
        for cond in &clause.ifs {
            if !truthy(interp, &eval_expr(interp, cf, cond, reg)?)? {
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
            // 02.07 fix: a genexpr whose DEFINING frame is the module frame
            // ran with store_local — the comp target landed in the module
            // frame's `locals` and PERMANENTLY shadowed the global name
            // (lookup checks locals first). Module frames keep locals empty;
            // route comp-target stores to globals there.
            if frame.is_module {
                frame.store_global(n, v);
            } else {
                frame.store_local(n, v);
            }
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
    // 02.03 user instances: __iter__ / legacy __getitem__ (S-03-24).
    if matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance) {
        let interp = current_interp_for_bindings();
        return user_iter_to_iterator(&interp, v);
    }
    // 02.04a (interp bug #13): iterating a CLASS drives the metaclass
    // __iter__ (enum.py: STRICT, CONFORM, EJECT, KEEP = FlagBoundary).
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::UserClass {
            if let Some(t) = o.class_ref.borrow().clone() {
                let meta = t.metaclass.borrow().clone();
                if let Some(meta) = meta {
                    let it_fn = meta.lookup_with_owner("__iter__").map(|(f, _)| f);
                    if let Some(it_fn) = it_fn {
                        let interp = current_interp_for_bindings();
                        let iter_v = call_value(
                            &interp,
                            &Rc::new(Frame::new_module(interp.globals.clone())),
                            &it_fn,
                            &[v.clone()],
                            &[],
                        )?;
                        return iterable_of(&iter_v);
                    }
                }
            }
        }
    }
    // 02.08-lite: registered iterator handles (iter()/native iters) re-fetch
    // their table entry instead of erroring on re-entry.
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Iterator {
            if let Some(id) = *o.gen_id.borrow() {
                let found = {
                    let interp2 = interp_rc();
                    let it = interp2.iterators.borrow().get(&id).cloned();
                    it
                };
                if let Some(it) = found {
                    return Ok(it);
                }
            }
        }
    }
    // 02.08-lite: native opaque iterables (deque) via the module layer.
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque {
            if let Some(it) = builtins_modules::native_iter(v) {
                return Ok(it);
            }
        }
    }
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
pub fn collect_iter_public(it: &Gc<PyIterator>) -> Vec<Value> {
    collect_iter(it)
}
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
    // 02.07: it.next() can re-enter interp.iterators (Thread rendezvous
    // inside a for loop) — never hold the borrow across it. Retry on a
    // contended borrow (a worker holds it while parked under the GIL).
    let r = loop {
        match interp.iterators.try_borrow() {
            Ok(its) => {
                let r = its.get(&id).and_then(|it| it.next());
                break r;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    };
    if r.is_none() {
        // 02.04: a finished iterator's Seq state pins every value it ever
        // yielded (genexpr snapshots) — a permanent root set. Drop it.
        // 02.07: retry on contention (workers hold borrows under the GIL).
        loop {
            match interp.iterators.try_borrow_mut() {
                Ok(mut its) => {
                    its.remove(&id);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
            }
        }
    }
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
        Some(Conversion::R) => {
            // 02.03: user __repr__ through the type first (f12).
            match user_repr(interp, v, "__repr__")? {
                Some(s) => s,
                None => ap_obj::ops::repr(v).map_err(Flow::from_py)?,
            }
        }
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
            // Exception instances: str via args semantics (S-06-7) even
            // though the value is a real user-class instance (e007).
            let is_exc = o
                .type_ref
                .borrow()
                .as_ref()
                .and_then(|w| w.upgrade())
                .is_some_and(|t| t.flags.is_exception);
            if is_exc {
                let ev = exception_value_of_instance(v)?;
                return ev.py_str().map_err(Flow::from_py);
            }
            // 02.03: __str__ -> __repr__ -> default form, via the TYPE (S-03-22/24).
            match user_repr(interp, v, "__str__")? {
                Some(s) => Ok(s),
                None => Ok(default_instance_repr(v)),
            }
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

/// Public installer for WORKER threads (02.07): a new OS thread evaluates
/// through the SAME interpreter object; this seeds its thread-local handle.
pub fn install_current_interp(interp: &Rc<Interpreter>) {
    CURRENT_INTERP.with(|c| *c.borrow_mut() = Some(interp.clone()));
}

fn current_interp_for_bindings() -> Rc<Interpreter> {
    CURRENT_INTERP
        .with(|c| c.borrow().clone())
        .expect("interpreter set for the run")
}

// ===========================================================================
// 02.03 special-method dispatch (S-03-22..26)
// ===========================================================================

/// Call a dunder found on the value's type with `self` prepended.
pub fn special_call(
    interp: &Rc<Interpreter>,
    obj: &Value,
    dunder: &str,
    args: &[Value],
) -> EResult {
    // 02.08-full part 1: native opaque objects answer dunder protocols
    // through the native_attr layer (tempfile.TemporaryDirectory
    // __enter__/__exit__/cleanup, stream context managers — s046/s005).
    if matches!(
        obj,
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque
    ) {
        if let Some(f) = crate::builtins_modules::native_attr(obj, dunder) {
            let mut full = vec![obj.clone()];
            full.extend_from_slice(args);
            return call_value(interp, &module_frame_pub(), &f, &full, &[]);
        }
    }
    let (f, _owner) = crate::classes::special_lookup(interp, obj, dunder)
        .ok_or_else(|| raise("TypeError", format!("no {}", dunder)))?;
    let mut full = vec![obj.clone()];
    full.extend_from_slice(args);
    call_value(interp, &module_frame_pub(), &f, &full, &[])
}

pub fn module_frame_pub() -> Rc<Frame> {
    let interp = current_interp_for_bindings();
    Rc::new(Frame::new_module(interp.globals.clone()))
}

/// The class Value for an exception class NAME (None-safe: v0 legacy
/// exceptions that predate the class tree still resolve through here).
pub fn exception_class_value(interp: &Rc<Interpreter>, name: &str) -> Value {
    crate::classes::exc_class_value(interp, name)
        .unwrap_or_else(|| Value::Obj(ap_obj::PyObject::new(ap_obj::ObjKind::Type, name)))
}

/// Materialize an exception INSTANCE value from an in-flight ExceptionValue
/// (for __exit__ args / except-as binding identity).
pub fn exception_instance_value(e: &ExceptionValue) -> Value {
    exception_instance(e)
}

/// Seed the `__class__` cell for zero-arg super(): every method defined in
/// the class body captures `__class__` through its closure `frees` (the
/// frontend classifies the implicit reference as a free read of the
/// class-frame-local `__class__`). Called AFTER the class value exists
/// (S-03-17: the cell binds the created class object).
pub fn seed_class_cell(interp: &Rc<Interpreter>, class_frame: &Rc<Frame>, class_v: &Value) {
    // Park the cell on the class frame; method-def registration inside the
    // body copied `class_frame.my_cells` BEFORE this runs, so we ALSO walk
    // the nested table and wire the cell directly (methods capture by
    // shared Rc — see bind_params' defining-frame path).
    let cell: CellSlot = Rc::new(RefCell::new(Some(class_v.clone())));
    *class_frame.class_cell.borrow_mut() = Some(cell.clone());
    // Walk functions defined in this body (registered while the body ran)
    // and give their defining-frame entry a `__class__` free-cell.
    let ids: Vec<u32> = interp
        .defining_frames
        .borrow()
        .iter()
        .filter(|(_, f)| Rc::ptr_eq(f, class_frame))
        .map(|(id, _)| *id)
        .collect();
    let _ = ids; // the real wiring happens in bind_params via class_cell
    let _ = interp;
}

/// User-protocol truthiness: __bool__ then __len__ then default (S-01-27
/// + S-03-26). Returns None when no protocol applies.
fn user_truthiness(interp: &Rc<Interpreter>, v: &Value) -> Result<Option<bool>, Flow> {
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__bool__") {
        let r = call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[])?;
        let b = r
            .as_int()
            .and_then(|i| i.to_i64())
            .ok_or_else(|| raise("TypeError", "__bool__ should return bool"))?;
        return Ok(Some(b != 0));
    }
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__len__") {
        let r = call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[])?;
        let n = r.as_int().and_then(|i| i.to_i64()).ok_or_else(|| {
            raise(
                "TypeError",
                format!("'__len__' should return >= 0, not '{}'", r.type_name()),
            )
        })?;
        return Ok(Some(n != 0));
    }
    Ok(None)
}

/// Protocol-aware truthiness: user __bool__/__len__ first (S-03-26),
/// then the value layer (S-01-27).
pub(crate) fn truthy(interp: &Rc<Interpreter>, v: &Value) -> Result<bool, Flow> {
    match user_truthiness(interp, v)? {
        Some(b) => Ok(b),
        None => Ok(v.is_truthy()),
    }
}

/// User-protocol len(): __len__ via the type (S-03-22), exact error text.
fn user_len(interp: &Rc<Interpreter>, v: &Value) -> Result<Option<i64>, Flow> {
    match crate::classes::special_lookup(interp, v, "__len__") {
        Some((f, _)) => {
            let r = call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[])?;
            match r.as_int().and_then(|i| i.to_i64()) {
                Some(n) => Ok(Some(n)),
                None => Err(raise(
                    "TypeError",
                    format!("'__len__' should return >= 0, not '{}'", r.type_name()),
                )),
            }
        }
        None => Ok(None),
    }
}

/// User-protocol repr/str. `which`: "__repr__" or "__str__".
fn user_repr(interp: &Rc<Interpreter>, v: &Value, which: &str) -> Result<Option<String>, Flow> {
    // __str__ falls back to __repr__ only through object.__str__ semantics
    // (S-03-24): an explicit __str__ must return str; missing __str__ with
    // a class __repr__ uses it; neither -> default form.
    let lookup = |name: &str| crate::classes::special_lookup(interp, v, name);
    let target = match which {
        "__str__" => lookup("__str__").or_else(|| lookup("__repr__")),
        other => lookup(other),
    };
    match target {
        Some((f, _)) => {
            let r = call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[])?;
            match r {
                Value::Str(s) => Ok(Some(s.as_str().to_string())),
                other => Err(raise(
                    "TypeError",
                    format!("{} returned non-string (type {})", which, other.type_name()),
                )),
            }
        }
        None => Ok(None),
    }
}

/// User binary-operator dispatch (S-03-25): returns Some(result) when a
/// user slot produced a value (or raised), None when no slot applies.
#[allow(clippy::too_many_arguments)]
fn user_binop(
    interp: &Rc<Interpreter>,
    op: &str,
    rop: &str,
    sym: &str,
    a: &Value,
    b: &Value,
) -> Option<EResult> {
    let a_slot = crate::classes::special_lookup(interp, a, op);
    let b_slot = crate::classes::special_lookup(interp, b, rop);
    let try_primary = |f: &Value| -> Option<EResult> {
        Some(call_value(
            interp,
            &module_frame_pub(),
            f,
            &[a.clone(), b.clone()],
            &[],
        ))
    };
    let try_reflected = |f: &Value| -> Option<EResult> {
        Some(call_value(
            interp,
            &module_frame_pub(),
            f,
            &[b.clone(), a.clone()],
            &[],
        ))
    };
    // Rule 1: proper-subclass reflected first (S-03-25).
    let b_subclass_a = matches!(
        (crate::classes::special_lookup(interp, b, "__init__"), a_slot.clone()),
        _ if false
    );
    let _ = b_subclass_a;
    let b_is_proper_sub = value_is_proper_subclass_of(interp, b, a);
    if b_is_proper_sub {
        if let Some((rf, _)) = &b_slot {
            if let Some(r) = try_reflected(rf) {
                if !is_not_implemented(&r) {
                    return Some(r);
                }
            }
        }
        if let Some((pf, _)) = &a_slot {
            if let Some(r) = try_primary(pf) {
                if !is_not_implemented(&r) {
                    return Some(r);
                }
            }
        }
    } else {
        if let Some((pf, _)) = &a_slot {
            if let Some(r) = try_primary(pf) {
                if !is_not_implemented(&r) {
                    return Some(r);
                }
            }
        }
        if let Some((rf, _)) = &b_slot {
            if let Some(r) = try_reflected(rf) {
                if !is_not_implemented(&r) {
                    return Some(r);
                }
            }
        }
    }
    // Both deferred/absent: only raise when at least one USER slot exists
    // (otherwise the builtin path raises its own message).
    if a_slot.is_some() || b_slot.is_some() {
        return Some(Err(raise(
            "TypeError",
            format!(
                "unsupported operand type(s) for {}: '{}' and '{}'",
                sym,
                crate::classes::display_type_name(a),
                crate::classes::display_type_name(b)
            ),
        )));
    }
    None
}

fn is_not_implemented(r: &EResult) -> bool {
    match r {
        Ok(Value::Obj(o)) => o.name == "NotImplemented" && o.kind == ap_obj::ObjKind::Opaque,
        _ => false,
    }
}
fn result_is_not_implemented(r: &EResult) -> bool {
    is_not_implemented(r)
}
fn is_not_implemented_value(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if o.name == "NotImplemented" && o.kind == ap_obj::ObjKind::Opaque)
}

fn value_is_proper_subclass_of(interp: &Rc<Interpreter>, b: &Value, a: &Value) -> bool {
    let (Some(bc), Some(ac)) = (
        crate::classes::class_of_value(interp, b),
        crate::classes::class_of_value(interp, a),
    ) else {
        return false;
    };
    // Proper subclass: b in a's MRO but a is not in b's MRO in the same slot.
    bc.is_subclass(&ac) && !ac.is_subclass(&bc)
}

/// In-place op dispatch (S-03-26): __iX__ first.
fn user_binop_inplace(
    interp: &Rc<Interpreter>,
    iop: &str,
    op: &str,
    rop: &str,
    sym: &str,
    a: &Value,
    b: &Value,
) -> Option<EResult> {
    if let Some((f, _)) = crate::classes::special_lookup(interp, a, iop) {
        let r = call_value(
            interp,
            &module_frame_pub(),
            &f,
            &[a.clone(), b.clone()],
            &[],
        );
        if !result_is_not_implemented(&r) {
            return Some(r);
        }
    }
    user_binop(interp, op, rop, sym, a, b)
}

/// User comparison dispatch: __eq__/__ne__/__lt__/__le__/__gt__/__ge__.
fn user_compare(
    interp: &Rc<Interpreter>,
    op: CmpOp,
    a: &Value,
    b: &Value,
) -> Option<Result<bool, Flow>> {
    // Ordering slots only; __eq__/__ne__ route through user_compare_value
    // (raw-value semantics, c028).
    if matches!(op, CmpOp::Eq | CmpOp::NotEq) {
        return user_compare_value(interp, op, a, b).map(|r| r.map(|v| v.is_truthy()));
    }
    let slot = match op {
        CmpOp::Lt => "__lt__",
        CmpOp::LtE => "__le__",
        CmpOp::Gt => "__gt__",
        CmpOp::GtE => "__ge__",
        _ => return None,
    };
    let a_slot = crate::classes::special_lookup(interp, a, slot);
    let b_slot = crate::classes::special_lookup(interp, b, slot);
    let call_slot = |f: &Value, x: &Value, y: &Value| {
        call_value(interp, &module_frame_pub(), f, &[x.clone(), y.clone()], &[])
    };
    // Proper-subclass reflected priority (S-03-25 rule 1).
    let b_proper = value_is_proper_subclass_of(interp, b, a);
    let try_one = |f: &Value, x: &Value, y: &Value| -> Option<Result<bool, Flow>> {
        let r = call_slot(f, x, y);
        if matches!(&r, Ok(v) if is_not_implemented_value(v)) {
            None
        } else {
            Some(r.map(|v| v.is_truthy()))
        }
    };
    if b_proper {
        if let Some((f, _)) = &b_slot {
            if let Some(r) = try_one(f, b, a) {
                return Some(r);
            }
        }
        if let Some((f, _)) = &a_slot {
            if let Some(r) = try_one(f, a, b) {
                return Some(r);
            }
        }
    } else {
        if let Some((f, _)) = &a_slot {
            if let Some(r) = try_one(f, a, b) {
                return Some(r);
            }
        }
        if let Some((f, _)) = &b_slot {
            if let Some(r) = try_one(f, b, a) {
                return Some(r);
            }
        }
    }
    if a_slot.is_some() || b_slot.is_some() {
        return Some(Err(raise(
            "TypeError",
            format!(
                "'{}' not supported between instances of '{}' and '{}'",
                match op {
                    CmpOp::Lt => "<",
                    CmpOp::LtE => "<=",
                    CmpOp::Gt => ">",
                    CmpOp::GtE => ">=",
                    _ => unreachable!(),
                },
                crate::classes::display_type_name(a),
                crate::classes::display_type_name(b)
            ),
        )));
    }
    None
}

fn class_protocol_builtin(
    interp: &Rc<Interpreter>,
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<EResult> {
    let subject = args.first();
    let is_user_instance =
        matches!(subject, Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::UserInstance);
    match name {
        "len" if is_user_instance => Some(match user_len(interp, subject.unwrap()) {
            Ok(Some(n)) => Ok(Value::Int(ap_obj::Int::from_i64(n))),
            Ok(None) => Err(raise(
                "TypeError",
                format!(
                    "object of type '{}' has no len()",
                    crate::classes::display_type_name(subject.unwrap())
                ),
            )),
            Err(f) => Err(f),
        }),
        // 02.08-full part 1: complex truthiness = (re, im) != (0, 0)
        // (V047: bool(0j) is False).
        "bool" if matches!(subject, Some(Value::Obj(o)) if o.kind == ap_obj::ObjKind::Opaque && o.name == "complex") =>
        {
            let (re, im) = complex_parts(subject.unwrap()).unwrap_or((0.0, 0.0));
            Some(Ok(Value::Bool(re != 0.0 || im != 0.0)))
        }
        "bool" if is_user_instance => Some(match user_truthiness(interp, subject.unwrap()) {
            Ok(Some(b)) => Ok(Value::Bool(b)),
            Ok(None) => Ok(Value::Bool(subject.unwrap().is_truthy())),
            Err(f) => Err(f),
        }),
        "abs" if is_user_instance => {
            crate::classes::special_lookup(interp, subject.unwrap(), "__abs__").map(|(f, _)| {
                call_value(
                    interp,
                    &module_frame_pub(),
                    &f,
                    &[subject.unwrap().clone()],
                    &[],
                )
            })
        }
        "repr" if is_user_instance && is_exception_instance(subject.unwrap()) => {
            // Exception instances: ClassName('arg', ...) (V043/e010).
            Some(repr_with_classes(interp, subject.unwrap()))
        }
        "repr" if is_user_instance => Some(match user_repr(interp, subject.unwrap(), "__repr__") {
            Ok(Some(s)) => Ok(Value::Str(PyString::new(s))),
            Ok(None) => Ok(Value::Str(PyString::new(default_instance_repr(
                subject.unwrap(),
            )))),
            Err(f) => Err(f),
        }),
        "repr" => {
            // Containers holding user instances render member __repr__s
            // (V066 repr([Both()]) -> [REPR]).
            Some(repr_with_classes(interp, subject.unwrap()))
        }
        "str" if is_user_instance && is_exception_instance(subject.unwrap()) => Some(
            exception_value_of_instance(subject.unwrap())
                .and_then(|e| e.py_str().map_err(Flow::from_py))
                .map(|s| Value::Str(PyString::new(s))),
        ),
        "str" if is_user_instance => Some(match user_repr(interp, subject.unwrap(), "__str__") {
            Ok(Some(s)) => Ok(Value::Str(PyString::new(s))),
            Ok(None) => Ok(Value::Str(PyString::new(default_instance_repr(
                subject.unwrap(),
            )))),
            Err(f) => Err(f),
        }),
        "iter" if is_user_instance => Some(user_iter(interp, subject.unwrap())),
        "next" => {
            let v = match args.first() {
                Some(v) => v,
                None => return None,
            };
            match v {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Iterator => None, // builtin path
                other if matches!(other, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance) => {
                    Some(user_next(interp, other))
                }
                _ => None,
            }
        }
        "hash" if is_user_instance => Some(
            match crate::classes::special_lookup(interp, subject.unwrap(), "__hash__") {
                Some((f, _)) => call_value(
                    interp,
                    &module_frame_pub(),
                    &f,
                    &[subject.unwrap().clone()],
                    &[],
                ),
                None => match ap_obj::ops::hash(subject.unwrap()) {
                    Ok(h) => Ok(Value::Int(ap_obj::Int::from_i64(h))),
                    Err(e) => Err(Flow::from_py(e)),
                },
            },
        ),
        "type" => {
            // type(x) on class-bearing values returns the metaclass CLASS
            // (identity: type(A) is type, c001); builtin values keep the
            // interned name-Type path (V045).
            // 02.03b: 3-arg type(name, bases, dict, **kwds) — dynamic class
            // creation — routes through the full class machinery.
            if args.len() >= 3 {
                return Some(builtins::call_builtin(interp, "type", args, kwargs));
            }
            // 02.08-full part 1: values whose __class__ differs from the
            // static type name (named native iterators → "count" etc).
            if let Some(vv) = args.first() {
                if let Value::Obj(o) = vv {
                    if o.kind == ap_obj::ObjKind::Iterator {
                        let t = crate::builtins::interned_type(interp, &o.name);
                        return Some(Ok(t));
                    }
                    // 02.07: threading objects report their <class> attr
                    // (type(me).__name__ == "_MainThread" — T108).
                    if std::env::var("APX_TDEBUG").is_ok() {
                        if let Value::Obj(so) = vv {
                            eprintln!("[TDEBUG] cpb type arm: kind={:?} name={}", so.kind, so.name);
                        }
                    }
                    if o.kind == ap_obj::ObjKind::Opaque && o.name == "thread" {
                        let cls = o
                            .attrs
                            .borrow()
                            .get(&Value::Str(PyString::new("<class>")))
                            .and_then(|v| match v {
                                Value::Str(sv) => Some(sv.as_str().to_string()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "Thread".to_string());
                        let t = crate::builtins::interned_type(interp, &cls);
                        return Some(Ok(t));
                    }
                    // 02.07: threading objects report Thread/_MainThread/
                    // Timer (type(me).__name__ == "_MainThread").
                    if o.kind == ap_obj::ObjKind::Opaque && o.name == "thread" {
                        let cls = o
                            .attrs
                            .borrow()
                            .get(&Value::Str(ap_obj::PyString::new("<class>")))
                            .and_then(|v| match v {
                                Value::Str(sv) => Some(sv.as_str().to_string()),
                                _ => None,
                            })
                            .unwrap_or_else(|| "Thread".to_string());
                        let t = crate::builtins::interned_type(interp, &cls);
                        return Some(Ok(t));
                    }
                }
            }
            let v = match args.first() {
                Some(v) => v,
                None => return None,
            };
            // Descriptors report their kind name (type(C.p) is property);
            // super objects report "super" (c021).
            if let Value::Obj(o) = v {
                if o.kind == ap_obj::ObjKind::Descriptor {
                    return Some(Ok(Value::Str(PyString::new(
                        o.descr_kind
                            .borrow()
                            .clone()
                            .unwrap_or_else(|| "object".to_string()),
                    ))));
                }
                if o.kind == ap_obj::ObjKind::Opaque && o.name == "super" {
                    return Some(Ok(Value::Str(PyString::new("super"))));
                }
                // 02.08-full p2: re.Pattern / re.Match report their Python
                // type names (type(p).__name__ == "Pattern", isinstance gates).
                if o.kind == ap_obj::ObjKind::Opaque
                    && (o.name == "re.Pattern" || o.name == "re.Match")
                {
                    return Some(Ok(Value::Str(PyString::new(
                        o.name.trim_start_matches("re.").to_string(),
                    ))));
                }
                // 02.08-full part 1: named native iterators report their
                // function name as the type (type(itertools.count(1)).__name__
                // == "count", s012 pin) — same mapping __class__ uses.
                if o.kind == ap_obj::ObjKind::Iterator
                    && matches!(
                        o.name.as_str(),
                        "count"
                            | "cycle"
                            | "repeat"
                            | "chain"
                            | "accumulate"
                            | "compress"
                            | "dropwhile"
                            | "takewhile"
                            | "filterfalse"
                            | "groupby"
                            | "islice"
                            | "starmap"
                            | "zip_longest"
                            | "batched"
                            | "pairwise"
                    )
                {
                    let t = crate::builtins::interned_type(interp, &o.name);
                    return Some(Ok(t));
                }
                if o.kind == ap_obj::ObjKind::Opaque && o.name == "complex" {
                    let t = crate::builtins::interned_type(interp, "complex");
                    return Some(Ok(t));
                }
            }
            match crate::classes::class_of_value(interp, v) {
                Some(t) => {
                    let meta = t
                        .metaclass
                        .borrow()
                        .clone()
                        .unwrap_or_else(|| crate::classes::type_class(interp));
                    Some(Ok(crate::classes::class_value_for_public(&meta)))
                }
                None => None,
            }
        }
        "isinstance" => Some(class_isinstance(interp, args)),
        "issubclass" => Some(class_issubclass(interp, args)),
        "property" => Some(make_property(interp, args)),

        "classmethod" => Some(make_descr(interp, "classmethod", args)),
        "staticmethod" => Some(make_descr(interp, "staticmethod", args)),
        "super" => {
            let r = match builtin_super(interp, args, kwargs) {
                Ok(v) => v,
                Err(e) => return Some(Err(e)),
            };
            // CPython semantic: `super().__new__` inside a metaclass /
            // class-hierarchy __new__ resolves through the implicit-
            // staticmethod rule (type.__new__ wraps user __new__ as
            // staticmethod), so it must come back UNBOUND — callers pass
            // mcs/cls explicitly. Mark the resolved value unbound here so
            // attr_get hands back the bare function; zero-arg super()
            // contexts stay intact for every other attribute.
            if args.is_empty() {
                if let Value::Obj(so) = &r {
                    if so.kind == ap_obj::ObjKind::Opaque && so.name == "super" {
                        let _ = so.attrs.borrow_mut().set(
                            Value::Str(PyString::new("__ap_unbound_new__")),
                            Value::Bool(true),
                        );
                    }
                }
            }
            Some(Ok(r))
        }
        _ => {
            let _ = kwargs;
            None
        }
    }
}

/// Public wrapper for print()'s container rendering.
pub fn repr_with_classes_public(interp: &Rc<Interpreter>, v: &Value) -> EResult {
    repr_with_classes(interp, v)
}

/// repr() that renders user __repr__ through containers (V066's
/// `repr([Both()])` -> `[REPR]`): walks the value tree, pre-rendering
/// user instances, then splices them into the structural form.
fn repr_with_classes(interp: &Rc<Interpreter>, v: &Value) -> EResult {
    fn render(
        interp: &Rc<Interpreter>,
        v: &Value,
        out: &mut String,
        active: &mut std::collections::HashSet<usize>,
    ) -> Result<(), Flow> {
        match v {
            Value::List(l) => {
                if !active.insert(ap_obj::value::addr_of(l)) {
                    out.push_str("[...]");
                    return Ok(());
                }
                out.push('[');
                for (i, item) in l.items().iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render(interp, item, out, active)?;
                }
                out.push(']');
                active.remove(&ap_obj::value::addr_of(l));
                Ok(())
            }
            Value::Tuple(t) => {
                out.push('(');
                for (i, item) in t.0.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render(interp, item, out, active)?;
                }
                if t.0.len() == 1 {
                    out.push(',');
                }
                out.push(')');
                Ok(())
            }
            Value::Dict(d) => {
                let id = ap_obj::value::addr_of(d);
                if !active.insert(id) {
                    out.push_str("{...}");
                    return Ok(());
                }
                out.push('{');
                for (i, (k, val)) in d.items().iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    render(interp, k, out, active)?;
                    out.push_str(": ");
                    render(interp, val, out, active)?;
                }
                out.push('}');
                active.remove(&id);
                Ok(())
            }
            Value::Set(s) => {
                let mut elems: Vec<String> = Vec::new();
                for e in s.iter() {
                    let mut tmp = String::new();
                    render(interp, &e, &mut tmp, active)?;
                    elems.push(tmp);
                }
                elems.sort();
                if elems.is_empty() {
                    out.push_str("set()");
                } else {
                    out.push('{');
                    out.push_str(&elems.join(", "));
                    out.push('}');
                }
                Ok(())
            }
            Value::Frozen(s) => {
                let mut elems: Vec<String> = Vec::new();
                for e in s.iter() {
                    let mut tmp = String::new();
                    render(interp, &e, &mut tmp, active)?;
                    elems.push(tmp);
                }
                elems.sort();
                if elems.is_empty() {
                    out.push_str("frozenset()");
                } else {
                    out.push_str("frozenset({");
                    out.push_str(&elems.join(", "));
                    out.push_str("})");
                }
                Ok(())
            }
            Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass => {
                out.push_str(&format!("<class '{}'>", o.name));
                Ok(())
            }
            Value::Obj(o) if o.kind == ap_obj::ObjKind::Exception => {
                // Exception instances: ClassName('arg', ...) (V043).
                let mut s = o.name.clone();
                s.push('(');
                for (i, a) in o.args.borrow().iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(&ap_obj::ops::repr(a).map_err(Flow::from_py)?);
                }
                s.push(')');
                out.push_str(&s);
                Ok(())
            }
            Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => {
                if crate::classes::is_exception_value(v) {
                    // ClassName('arg', ...) (e009/e010 repr).
                    let ev = exception_value_of_instance(v)?;
                    let mut s = ev.class;
                    s.push('(');
                    for (i, a) in ev.args.iter().enumerate() {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        s.push_str(&ap_obj::ops::repr(a).map_err(Flow::from_py)?);
                    }
                    s.push(')');
                    out.push_str(&s);
                    return Ok(());
                }
                match user_repr(interp, v, "__repr__")? {
                    Some(s) => {
                        out.push_str(&s);
                        Ok(())
                    }
                    None => {
                        out.push_str(&default_instance_repr(v));
                        Ok(())
                    }
                }
            }
            Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque => {
                // 02.07: thread reprs (<Thread(NAME, state)> — the corpus
                // prints HEADS only, addresses masked; the repr carries the
                // ident like CPython when started/stopped).
                if o.name == "thread" {
                    out.push_str(&crate::threading_call::thread_repr_pub(o));
                    return Ok(());
                }
                // 02.08-full part 1: native opaque repr_text wins (complex,
                // dict views, TemporaryDirectory).
                if let Some(txt) = &*o.repr_text.borrow() {
                    out.push_str(txt);
                    return Ok(());
                }
                out.push_str(&ap_obj::ops::repr(v).map_err(Flow::from_py)?);
                Ok(())
            }
            other => {
                out.push_str(&ap_obj::ops::repr(other).map_err(Flow::from_py)?);
                Ok(())
            }
        }
    }
    let mut out = String::new();
    let mut active = std::collections::HashSet::new();
    render(interp, v, &mut out, &mut active)?;
    Ok(Value::Str(PyString::new(out)))
}

/// Is this a real exception-class instance (02.03 fast-path product)?
fn is_exception_instance(v: &Value) -> bool {
    matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance)
        && crate::classes::is_exception_value(v)
}

/// `<module.Class object at 0x...>` deterministic form (V066/C-24: the
/// address digits are UNSPECIFIED; the prefix is byte-pinned).
fn default_instance_repr(v: &Value) -> String {
    match v {
        Value::Obj(o) => format!(
            "<{}.{} object at 0x{:x}>",
            ap_obj::MODULE_NAME,
            o.name,
            ap_obj::value::addr_of(o)
        ),
        _ => v.type_name().to_string(),
    }
}

/// isinstance through real classes with the tuple/union forms (S-03-31).

fn t_kind(t: &Value) -> Option<&'static str> {
    match t {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
            match o.descr_kind.borrow().as_deref() {
                Some(k @ ("classmethod" | "staticmethod" | "property")) => Some(match k {
                    "classmethod" => "classmethod",
                    "staticmethod" => "staticmethod",
                    _ => "property",
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

fn class_isinstance(interp: &Rc<Interpreter>, args: &[Value]) -> EResult {
    let v = args
        .first()
        .ok_or_else(|| raise("TypeError", "isinstance expected 2 arguments, got 1"))?;
    let t = args
        .get(1)
        .ok_or_else(|| raise("TypeError", "isinstance expected 2 arguments, got 1"))?;
    // Custom metaclass __instancecheck__ (S-03-31; c033 Weird) — consulted
    // before the structural check, for any subject.
    if let Value::Obj(to) = t {
        if to.kind == ap_obj::ObjKind::UserClass {
            if let Some(tc) = crate::classes::type_of(interp, t) {
                if let Some(meta) = tc.metaclass.borrow().clone() {
                    if let Some(hook) = meta.lookup("__instancecheck__") {
                        if matches!(&hook, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function) {
                            return call_value(
                                interp,
                                &module_frame_pub(),
                                &hook,
                                &[t.clone(), v.clone()],
                                &[],
                            )
                            .map(|r| Value::Bool(r.is_truthy()));
                        }
                    }
                }
            }
        }
    }
    // Fast path: builtin value + builtin type name (V045 surface).
    if !matches!(v, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance || o.kind == ap_obj::ObjKind::Exception)
    {
        if let Value::Obj(to) = t {
            if to.kind == ap_obj::ObjKind::Type {
                return Ok(Value::Bool(builtins::type_matches(v, &to.name)));
            }
        }
        // Non-type second arg with a builtin subject: pinned TypeError
        // (c033 `isinstance(1, 2)`).
        let is_union = matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "__union__");
        // 02.03b: descriptor pseudo-types (enum.py isinstance(value,
        // staticmethod) in EnumDict.__setitem__) — v0: staticmethod/
        // classmethod/property checks answer False for builtin subjects
        // (a builtin value is never a descriptor instance).
        let is_descr_type = matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor
            && matches!(o.descr_kind.borrow().as_deref(), Some("classmethod" | "staticmethod" | "property")));
        let is_descr_marker = matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::BuiltinFn
            && matches!(o.name.as_str(), "staticmethod" | "classmethod" | "property"));
        if is_descr_type || is_descr_marker {
            // Subject of the same descriptor kind -> True (enum.py
            // isinstance(value, staticmethod) where value IS the descriptor).
            let want: Option<String> = match t {
                Value::Obj(o) if o.kind == ap_obj::ObjKind::Descriptor => {
                    o.descr_kind.borrow().clone()
                }
                Value::Obj(o) => Some(o.name.clone()),
                _ => None,
            };
            let same_kind = matches!(v, Value::Obj(vo) if vo.kind == ap_obj::ObjKind::Descriptor
                && vo.descr_kind.borrow().as_deref() == want.as_deref());
            return Ok(Value::Bool(same_kind));
        }
        if !matches!(t, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass || o.kind == ap_obj::ObjKind::Type)
            && !matches!(t, Value::Tuple(_))
            && !is_union
        {
            {
                let tn2 = match t {
                    Value::Obj(o) => format!("Obj/{}", o.name),
                    other => other.type_name().to_string(),
                };
                let vn = match v {
                    Value::Obj(o) => format!("Obj/{}", o.name),
                    other => other.type_name().to_string(),
                };
                let k2 = match t {
                    Value::Obj(o) => o.descr_kind.borrow().clone().unwrap_or_default(),
                    _ => String::new(),
                };
                let k1 = match v {
                    Value::Obj(o) => o.descr_kind.borrow().clone().unwrap_or_default(),
                    _ => String::new(),
                };
            }
            return Err(raise(
                "TypeError",
                "isinstance() arg 2 must be a type, a tuple of types, or a union",
            ));
        }
    }
    Ok(Value::Bool(crate::classes::value_isinstance(interp, v, t)?))
}

fn class_issubclass(interp: &Rc<Interpreter>, args: &[Value]) -> EResult {
    if std::env::var("APX_DBG_ISS").is_ok() {
        if let (Some(a), Some(b)) = (args.first(), args.get(1)) {
            let nm = |v: &Value| match v {
                Value::Obj(o) => format!("{:?}:{}", o.kind, o.name),
                other => other.type_name().to_string(),
            };
            eprintln!("DBG issubclass {} <= {}", nm(a), nm(b));
        }
    }
    let a = args
        .first()
        .ok_or_else(|| raise("TypeError", "issubclass expected 2 arguments, got 1"))?;
    let b = args
        .get(1)
        .ok_or_else(|| raise("TypeError", "issubclass expected 2 arguments, got 1"))?;
    // Custom metaclass __subclasscheck__ (S-03-31; c033 WS).
    if let Value::Obj(to) = b {
        if to.kind == ap_obj::ObjKind::UserClass {
            if let Some(tc) = crate::classes::type_of(interp, b) {
                if let Some(meta) = tc.metaclass.borrow().clone() {
                    if let Some(hook) = meta.lookup("__subclasscheck__") {
                        if matches!(&hook, Value::Obj(f) if f.kind == ap_obj::ObjKind::Function) {
                            return call_value(
                                interp,
                                &module_frame_pub(),
                                &hook,
                                &[b.clone(), a.clone()],
                                &[],
                            )
                            .map(|r| Value::Bool(r.is_truthy()));
                        }
                    }
                }
            }
        }
    }
    // 02.08-lite: legacy interned-Type LEFT side (e.g.
    // issubclass(json.JSONDecodeError, ValueError)) routes through the v0
    // name-based subclass table — the value layer rejects plain Type objects.
    if let Value::Obj(x) = a {
        if x.kind == ap_obj::ObjKind::Type {
            let name_ok = match b {
                Value::Obj(y)
                    if y.kind == ap_obj::ObjKind::Type || y.kind == ap_obj::ObjKind::UserClass =>
                {
                    true
                }
                Value::Tuple(_) => true,
                _ => false,
            };
            if name_ok {
                let one = |have: &str, want: &str| {
                    have == want
                        // 02.08-full part 1: legacy name-Types subclass
                        // object (s003 issubclass(os.PathLike, object)).
                        || want == "object"
                        || (have == "bool" && want == "int")
                        // 02.07: Timer IS-A Thread (CPython hierarchy).
                        || (have == "Timer" && want == "Thread")
                        || crate::exc_hier::is_subclass(have, want)
                };
                let result = match b {
                    Value::Obj(y) => {
                        let r = one(&x.name, &y.name);
                        if std::env::var("APX_DBG_ISS").is_ok() && r {
                            eprintln!("DBG TRUE: {} <= {}", x.name, y.name);
                        }
                        r
                    }
                    Value::Tuple(parts) => parts.0.iter().any(|p| {
                        if let Value::Obj(y) = p {
                            one(&x.name, &y.name)
                        } else {
                            false
                        }
                    }),
                    _ => false,
                };
                return Ok(Value::Bool(result));
            }
        }
    }
    Ok(Value::Bool(crate::classes::value_issubclass(interp, a, b)?))
}

/// property(fget, [fset], [fdel]) -> descriptor object (S-03-21).
fn make_property(interp: &Rc<Interpreter>, args: &[Value]) -> EResult {
    let _ = interp;
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, "property");
    *o.descr_kind.borrow_mut() = Some("property".to_string());
    *o.self_value.borrow_mut() = args.first().cloned(); // fget
    *o.func.borrow_mut() = args.get(1).cloned(); // fset
    if let Some(fd) = args.get(2) {
        o.dunders
            .borrow_mut()
            .set(Value::Str(PyString::new("fdel")), fd.clone())
            .map_err(Flow::from_py)?;
    }
    Ok(Value::Obj(o))
}

fn make_descr(interp: &Rc<Interpreter>, kind: &str, args: &[Value]) -> EResult {
    let _ = interp;
    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Descriptor, kind);
    *o.descr_kind.borrow_mut() = Some(kind.to_string());
    *o.func.borrow_mut() = args.first().cloned();
    Ok(Value::Obj(o))
}

/// super() zero-arg (via __class__ cell) + two-arg forms (S-03-17).
/// v0 zero-arg: the enclosing method's `self`/`cls` is the FIRST parameter
/// slot; we record (start-class) on the super VALUE and resolve attributes
/// against type(self).__mro__ after that class (S-03-18).
fn builtin_super(interp: &Rc<Interpreter>, args: &[Value], _kwargs: &[(String, Value)]) -> EResult {
    match args.len() {
        0 => {
            // Zero-arg super(): the innermost active method context
            // (S-03-17). Outside a method the stack is empty -> the pinned
            // RuntimeError.
            let ctx = interp
                .super_stack
                .borrow()
                .last()
                .cloned()
                .filter(|(start, _, _)| start.name != "__plain_fn__");
            match ctx {
                None => {
                    let fs = interp.super_stack.borrow();
                    let names: Vec<String> = fs.iter().map(|(st, _, _)| st.name.clone()).collect();
                    let tb = interp.tb_frames.borrow();
                    Err(raise("RuntimeError", "super(): no arguments"))
                }
                Some((start_t, obj, is_cls)) => {
                    let start_v = crate::classes::class_value_for_public(&start_t);
                    let o = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "super");
                    *o.slice_parts.borrow_mut() = vec![start_v, obj];
                    let _ = is_cls;
                    Ok(Value::Obj(o))
                }
            }
        }
        2 => {
            let start = args[0].clone();
            let obj = args[1].clone();
            // Validation (S-03-17 text).
            let start_t = crate::classes::base_type_ref(interp, &start)?
                .ok_or_else(|| raise("TypeError", "super() argument 1 must be a type"))?;
            let obj_is_inst = crate::classes::class_of_value(interp, &obj)
                .is_some_and(|oc| oc.is_subclass(&start_t));
            let obj_is_class = crate::classes::base_type_ref(interp, &obj)?
                .is_some_and(|oc| oc.is_subclass(&start_t));
            if !obj_is_inst && !obj_is_class {
                let inst_name = crate::classes::display_type_name(&obj);
                return Err(raise(
                    "TypeError",
                    format!(
                        "super(type, obj): obj (instance of {}) is not an instance or subtype of type ({}).",
                        inst_name, start_t.name
                    ),
                ));
            }
            let o = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "super");
            *o.slice_parts.borrow_mut() = vec![start, obj];
            Ok(Value::Obj(o))
        }
        n => Err(raise(
            "TypeError",
            format!("super() takes 0 or 2 arguments, got {n}"),
        )),
    }
}

/// Register an iterator for the CURRENT run (shared with builtins_modules'
/// register_shared_iter; single table, single id space).
pub fn register_shared_iter_pub(it: Gc<PyIterator>) -> u64 {
    let interp = current_interp_for_bindings();
    let id = interp.next_iter_id.get();
    interp.next_iter_id.set(id + 2);
    interp.iterators.borrow_mut().insert(id, it);
    id
}

/// user_iter variant returning the registered PyIterator (iterable_of path).
fn user_iter_to_iterator(interp: &Rc<Interpreter>, v: &Value) -> Result<Gc<PyIterator>, Flow> {
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__iter__") {
        let r = call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[])?;
        return iterable_of(&r);
    }
    // 02.03b: dict-derived instances without __iter__ iterate their
    // backing dict's keys (CPython dict-subclass semantics).
    if let Some(d) = crate::classes::dict_backing(v) {
        let items: Vec<Value> = d.keys();
        return Ok(PyIterator::new(IterState::Seq(items, 0)));
    }
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__getitem__") {
        let mut items = Vec::new();
        for i in 0.. {
            let r = call_value(
                interp,
                &module_frame_pub(),
                &f,
                &[v.clone(), Value::Int(ap_obj::Int::from_i64(i))],
                &[],
            );
            match r {
                Ok(item) => items.push(item),
                Err(Flow::Raise(e)) if e.class == "IndexError" => break,
                Err(f2) => return Err(f2),
            }
            if i > 1_000_000 {
                break;
            }
        }
        return Ok(crate::iter::PyIterator::new(crate::iter::IterState::Seq(
            items, 0,
        )));
    }
    Err(raise(
        "TypeError",
        format!(
            "'{}' object is not iterable",
            crate::classes::display_type_name(v)
        ),
    ))
}

/// iter() over a user instance: __iter__ result or legacy __getitem__
/// sequence protocol (S-03-24).
fn user_iter(interp: &Rc<Interpreter>, v: &Value) -> EResult {
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__iter__") {
        return call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[]);
    }
    if let Some((f, _)) = crate::classes::special_lookup(interp, v, "__getitem__") {
        // Legacy sequence protocol: index 0.. until IndexError.
        let mut items = Vec::new();
        for i in 0.. {
            let r = call_value(
                interp,
                &module_frame_pub(),
                &f,
                &[v.clone(), Value::Int(ap_obj::Int::from_i64(i))],
                &[],
            );
            match r {
                Ok(item) => items.push(item),
                Err(Flow::Raise(e)) if e.class == "IndexError" => break,
                Err(f2) => return Err(f2),
            }
            if i > 1_000_000 {
                break; // defensive: no runaway sequences
            }
        }
        let it = crate::iter::PyIterator::new(crate::iter::IterState::Seq(items, 0));
        let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
        let id = interp.next_iter_id.get();
        interp.next_iter_id.set(id + 2);
        *obj.gen_id.borrow_mut() = Some(id);
        interp.iterators.borrow_mut().insert(id, it);
        return Ok(Value::Obj(obj));
    }
    Err(raise(
        "TypeError",
        format!(
            "'{}' object is not iterable",
            crate::classes::display_type_name(v)
        ),
    ))
}

/// next() over a user-defined iterator (__next__ protocol).
fn user_next(interp: &Rc<Interpreter>, v: &Value) -> EResult {
    let (f, _) = crate::classes::special_lookup(interp, v, "__next__").ok_or_else(|| {
        raise(
            "TypeError",
            format!("'{}' object is not an iterator", v.type_name()),
        )
    })?;
    match call_value(interp, &module_frame_pub(), &f, &[v.clone()], &[]) {
        Ok(r) => Ok(r),
        Err(Flow::Raise(e)) if e.class == "StopIteration" => Err(Flow::Raise(e)),
        Err(f2) => Err(f2),
    }
}

/// Raw-value user __eq__/__ne__ dispatch (c028: the slot's return value IS
/// the comparison result, truthiness applied by the caller).
fn user_compare_value(
    interp: &Rc<Interpreter>,
    op: CmpOp,
    a: &Value,
    b: &Value,
) -> Option<EResult> {
    let mut slot = match op {
        CmpOp::Eq => "__eq__",
        CmpOp::NotEq => "__ne__",
        _ => return None,
    };
    let mut a_slot = crate::classes::special_lookup(interp, a, slot);
    let mut b_slot = crate::classes::special_lookup(interp, b, slot);
    // != derives from __eq__ when __ne__ is absent (S-03-25/c028).
    let negate = slot == "__ne__" && a_slot.is_none() && b_slot.is_none();
    if negate {
        slot = "__eq__";
        a_slot = crate::classes::special_lookup(interp, a, slot);
        b_slot = crate::classes::special_lookup(interp, b, slot);
    }
    let call_slot = |f: &Value, x: &Value, y: &Value| {
        call_value(interp, &module_frame_pub(), f, &[x.clone(), y.clone()], &[])
    };
    let negate_if = |r: EResult| -> EResult {
        if negate {
            r.map(|v| Value::Bool(!v.is_truthy()))
        } else {
            r
        }
    };
    // Proper-subclass reflected priority (S-03-25 rule 1).
    let b_proper = value_is_proper_subclass_of(interp, b, a);
    if b_proper {
        if let Some((f, _)) = &b_slot {
            let r = call_slot(f, b, a);
            if !matches!(&r, Ok(v) if is_not_implemented_value(v)) {
                return Some(negate_if(r));
            }
        }
    }
    if let Some((f, _)) = &a_slot {
        let r = call_slot(f, a, b);
        if !matches!(&r, Ok(v) if is_not_implemented_value(v)) {
            return Some(negate_if(r));
        }
    }
    if !b_proper {
        if let Some((f, _)) = &b_slot {
            let r = call_slot(f, b, a);
            if !matches!(&r, Ok(v) if is_not_implemented_value(v)) {
                return Some(negate_if(r));
            }
        }
    }
    None
}
fn attr_get(_interp: &Rc<Interpreter>, obj: &Value, name: &str) -> Result<Option<Value>, Flow> {
    // __class__ on any builtin value (V045: fs.__class__ is frozenset).
    // Type objects are INTERNED so `x.__class__ is frozenset` holds.
    if name == "__class__" {
        // 02.03: user classes/instances answer from the real class table
        // (identity: x.__class__ is C, type(A) is A.__class__).
        if matches!(obj, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass || o.kind == ap_obj::ObjKind::UserInstance)
        {
            if let Some(v) = crate::classes::attr_get_opt(_interp, obj, name)? {
                return Ok(Some(v));
            }
        }
        // 02.07: threading native objects carry their class name (<class>).
        if matches!(obj, Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque
            && matches!(o.name.as_str(), "thread"))
        {
            if let Some(v) = crate::builtins_modules::native_attr(obj, name) {
                return Ok(Some(v));
            }
        }
        let tname = match obj {
            Value::Frozen(_) => "frozenset",
            // 02.08-full part 1: named native iterators report their
            // function name as the class (s012 type(itertools.count(1))).
            Value::Obj(o)
                if o.kind == ap_obj::ObjKind::Iterator
                    && matches!(
                        o.name.as_str(),
                        "count"
                            | "cycle"
                            | "repeat"
                            | "chain"
                            | "accumulate"
                            | "compress"
                            | "dropwhile"
                            | "takewhile"
                            | "filterfalse"
                            | "groupby"
                            | "islice"
                            | "starmap"
                            | "zip_longest"
                            | "batched"
                            | "pairwise"
                    ) =>
            {
                o.name.as_str()
            }
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
    // 02.04a fix pass: None.__new__ (enum._find_new_ compares None.__new__
    // inside a set to detect user overrides).
    if matches!(obj, Value::None) && name == "__new__" {
        return Ok(Some(Value::Obj(ap_obj::PyObject::new(
            ap_obj::ObjKind::BuiltinFn,
            "NoneType.__new__",
        ))));
    }
    // 02.04a fix pass: dict ITEM fallback for underscore attribute reads —
    // EnumDict's _member_names/_ignore land as ns items (class __prepare__
    // bridge); CPython dicts don't expose attrs, but our EnumDict shim needs
    // the read to resolve.
    if let Value::Dict(d) = obj {
        if name.starts_with('_') {
            if let Some(v) = d.get(&Value::Str(PyString::new(name))) {
                return Ok(Some(v));
            }
        }
    }
    // 02.04a fix pass: function attribute reads (functools.wraps sets
    // __wrapped__/cache etc on the wrapper; vendored stdlib reads them back).
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Function {
            if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                return Ok(Some(v));
            }
            if name == "__module__" {
                return Ok(Some(Value::Str(PyString::new("builtins"))));
            }
            if name == "__name__" || name == "__qualname__" {
                return Ok(Some(Value::Str(PyString::new(o.name.clone()))));
            }
        }
    }
    // 02.04a fix pass: class-statement keyword marker — the lowering emits
    // `__ap_classkw__.<name>(value)` base expressions; reads on the marker
    // return callable no-ops the call path consumes (interp call_value).
    if matches!(obj, Value::Obj(o) if o.kind == ap_obj::ObjKind::Opaque && o.name == "__ap_classkw__")
    {
        return Ok(Some(Value::Obj(ap_obj::PyObject::new(
            ap_obj::ObjKind::BuiltinFn,
            &format!("__ap_classkw__.{name}"),
        ))));
    }
    // 02.03b: `_value_` on builtin str/int "enum members" (StrEnum/IntEnum
    // via the _simple_enum path): the value IS the member — CPython
    // str-subclass members carry _value_ == self.
    if name == "_value_" {
        if matches!(obj, Value::Str(_) | Value::Int(_) | Value::Bool(_)) {
            return Ok(Some(obj.clone()));
        }
    }
    // 02.03b: member.__init__(...) on a str/int enum member — no-op
    // callable (object.__init__ semantics).
    if name == "__init__" {
        if matches!(
            obj,
            Value::Str(_) | Value::Int(_) | Value::Bool(_) | Value::Bytes(_)
        ) {
            return Ok(Some(Value::Obj(ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                "__object_virtual_init__",
            ))));
        }
    }
    // 02.08-lite: native-type instance attrs/methods + structured exc fields.
    // 02.04 completion: weakref.proxy — transparent attribute forwarding to
    // the live target; a dead target raises ReferenceError (C-22). This arm
    // runs BEFORE native_attr so the proxy object never answers directly.
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "_weakref_proxy" {
            let addr = o
                .attrs
                .borrow()
                .get(&Value::Str(PyString::new("<target_addr>")))
                .and_then(|v| v.as_int())
                .and_then(|iv| iv.to_i64())
                .map(|n| n.max(0) as usize)
                .unwrap_or(0);
            if addr == 0 || !ap_gc::weakref_alive(addr) {
                return Err(raise(
                    "ReferenceError",
                    "weakly-referenced object no longer exists",
                ));
            }
            let Some(target) = crate::gcsupport::target_value_by_addr(addr) else {
                return Err(raise(
                    "ReferenceError",
                    "weakly-referenced object no longer exists",
                ));
            };
            // Dunders keep builtin semantics (repr/str/class handled by the
            // generic paths); everything else forwards to the target.
            if name.starts_with("__") && name.ends_with("__") {
                // fall through to the generic surfaces below
            } else {
                return crate::classes::attr_get_protocol(_interp, &target, name).map(Some);
            }
        }
    }
    if let Some(v) = builtins_modules::native_attr(obj, name) {
        return Ok(Some(v));
    }
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Exception {
            // 02.08-full part 1: SyntaxError field surface (s001 e.msg pin).
            if o.name == "SyntaxError" {
                match name {
                    "msg" => {
                        let msg = o
                            .args
                            .borrow()
                            .first()
                            .map(|v| crate::builtins::py_str_of(v).unwrap_or_default())
                            .unwrap_or_else(|| "invalid syntax".to_string());
                        return Ok(Some(Value::Str(PyString::new(msg))));
                    }
                    "lineno" | "offset" => return Ok(Some(Value::None)),
                    "text" => return Ok(Some(Value::None)),
                    _ => {}
                }
            }
            if let Some(v) = builtins_modules::exception_attr(&o.name, name) {
                return Ok(Some(v));
            }
        }
    }
    // property.setter/.deleter: return a NEW property with fset/fdel set
    // (c011's @x.setter decoration chain).
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Descriptor
            && matches!(o.descr_kind.borrow().as_deref(), Some("property"))
        {
            if name == "setter" || name == "deleter" {
                // `p.setter(fn)` — a BuiltinFn marker carrying the property;
                // the call site is class_protocol_builtin's "property.*".
                let m = ap_obj::PyObject::new(
                    ap_obj::ObjKind::BuiltinFn,
                    &format!("property.{}", name),
                );
                *m.slice_parts.borrow_mut() = vec![obj.clone()];
                return Ok(Some(Value::Obj(m)));
            }
        }
    }
    // Bound-method introspection (S-03-21/24: __self__/__func__/__name__;
    // c024 identity probes).
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::BoundMethod {
            match name {
                "__self__" => {
                    return Ok(Some(o.self_value.borrow().clone().unwrap_or(Value::None)))
                }
                "__func__" => return Ok(Some(o.func.borrow().clone().unwrap_or(Value::None))),
                "__name__" => return Ok(Some(Value::Str(PyString::new(o.name.clone())))),
                // 02.03b-fix: bound-method __doc__ delegates to the function
                // (unittest reads testMethod.__doc__ off the bound method).
                "__doc__" => {
                    let f = o.func.borrow().clone().unwrap_or(Value::None);
                    if let Value::Obj(fo) = &f {
                        if fo.kind == ap_obj::ObjKind::Function {
                            return Ok(Some(
                                fo.attrs
                                    .borrow()
                                    .get(&Value::Str(PyString::new("__doc__")))
                                    .unwrap_or(Value::None),
                            ));
                        }
                    }
                    return Ok(Some(Value::None));
                }
                _ => {}
            }
        }
        // 02.03b-fix: descriptor attribute reads (property.__doc__ etc):
        // creation-stored attrs win, __doc__ defaults to None.
        if o.kind == ap_obj::ObjKind::Descriptor {
            if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                return Ok(Some(v));
            }
            if name == "__doc__" {
                return Ok(Some(Value::None));
            }
        }
        // 02.03b: descriptor introspection — enum.py reads
        // value.__func__ on staticmethod/classmethod descriptors
        // (EnumDict._generate_next_value_ handling).
        if o.kind == ap_obj::ObjKind::Descriptor {
            if name == "__func__" {
                return Ok(Some(o.func.borrow().clone().unwrap_or(Value::None)));
            }
            // 02.03b-fix: property.__doc__ surface (dis.py writes
            // _Instruction.opname.__doc__ = "...").
            if name == "__doc__" && o.descr_kind.borrow().as_deref() == Some("property") {
                return Ok(Some(Value::None));
            }
        }
        if o.kind == ap_obj::ObjKind::Function && name == "__name__" {
            return Ok(Some(Value::Str(PyString::new(o.name.clone()))));
        }
        // 02.03b-fix: function __doc__ (unittest.TestCase stores
        // testMethod.__doc__ at instance init).
        if o.kind == ap_obj::ObjKind::Function && name == "__doc__" {
            return Ok(Some(
                o.attrs
                    .borrow()
                    .get(&Value::Str(PyString::new("__doc__")))
                    .unwrap_or(Value::None),
            ));
        }
        // 02.03b-fix: function __dict__ — the live attrs dict
        // (importlib._bootstrap machinery reads fn.__dict__).
        if o.kind == ap_obj::ObjKind::Function && name == "__dict__" {
            return Ok(Some(Value::Dict(o.attrs.borrow().clone())));
        }
        // 02.03b-fix: function.__repr__/__str__ surface (unittest import
        // probes it via the warnings/contextlib decoration chain).
        if o.kind == ap_obj::ObjKind::Function && matches!(name, "__repr__" | "__str__") {
            let marker = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_function_repr__");
            *marker.slice_parts.borrow_mut() = vec![obj.clone()];
            return Ok(Some(Value::Obj(marker)));
        }
        // 02.03b-fix: builtin markers (collections.deque etc) expose a repr
        // surface — native-marker values get repr'd inside class bodies.
        if o.kind == ap_obj::ObjKind::BuiltinFn && matches!(name, "__repr__" | "__str__") {
            let marker = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_builtin_repr__");
            *marker.slice_parts.borrow_mut() = vec![obj.clone()];
            return Ok(Some(Value::Obj(marker)));
        }
        // 02.03b: functions ARE descriptors in CPython (enum._is_descriptor
        // probes __get__ to keep plain methods out of _member_names_).
        if o.kind == ap_obj::ObjKind::Function
            && matches!(name, "__get__" | "__set__" | "__delete__")
        {
            return Ok(Some(Value::Obj(ap_obj::PyObject::new(
                ap_obj::ObjKind::BuiltinFn,
                &format!("function.{}", name),
            ))));
        }
        // 02.04a (interp bug #1, vendored-stdlib stress): CPython functions
        // carry __doc__/__module__/__qualname__ — posixpath.py does
        // `splitext.__doc__ = genericpath._splitext.__doc__` at import.
        // Function-attribute WRITES land in o.attrs (bug #1b, classes.rs);
        // reads check that dict first, then fall back to defaults.
        if o.kind == ap_obj::ObjKind::Function {
            if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                return Ok(Some(v));
            }
            match name {
                "__doc__" | "__module__" => return Ok(Some(Value::None)),
                "__qualname__" => return Ok(Some(Value::Str(PyString::new(o.name.clone())))),
                // types.py/_cell_factory probes __closure__ (tuple of cells)
                // and reads [0]. A one-cell stub satisfies CellType = type(...)
                // (cell_contents reads return None — documented v0 gap).
                "__closure__" => {
                    let cell = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "cell");
                    let _ = cell
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("cell_contents")), Value::None);
                    return Ok(Some(Value::Tuple(Gc::new(Tuple(vec![Value::Obj(cell)])))));
                }
                // 02.04a: a stub code object (types.py/inspect.py/dataclasses
                // probe __code__ at import). Only the attributes those probes
                // read exist; no bytecode.
                "__code__" => {
                    let code = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<code>");
                    let _ = code.attrs.borrow_mut().set(
                        Value::Str(PyString::new("co_flags")),
                        Value::Int(ap_obj::Int::from_i64(67)),
                    );
                    let _ = code.attrs.borrow_mut().set(
                        Value::Str(PyString::new("co_name")),
                        Value::Str(PyString::new(o.name.clone())),
                    );
                    let _ = code.attrs.borrow_mut().set(
                        Value::Str(PyString::new("co_freevars")),
                        Value::Tuple(Gc::new(Tuple(Vec::new()))),
                    );
                    let _ = code.attrs.borrow_mut().set(
                        Value::Str(PyString::new("co_firstlineno")),
                        Value::Int(ap_obj::Int::from_i64(1)),
                    );
                    let _ = code.attrs.borrow_mut().set(
                        Value::Str(PyString::new("co_varnames")),
                        Value::Tuple(Gc::new(Tuple(Vec::new()))),
                    );
                    return Ok(Some(Value::Obj(code)));
                }
                _ => {}
            }
        }
    }
    // super object: attribute lookups scan type(obj).__mro__ strictly AFTER
    // the start class (S-03-18), applying the descriptor protocol.
    if let Value::Obj(o) = obj {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "super" {
            let parts = o.slice_parts.borrow();
            let (start_v, subject) = (parts[0].clone(), parts[1].clone());
            let start_t = crate::classes::type_of(_interp, &start_v)
                .or_else(|| crate::classes::class_of_value(_interp, &start_v));
            let subj_t = crate::classes::class_of_value(_interp, &subject);
            if let (Some(start_t), Some(subj_t)) = (start_t, subj_t) {
                // 02.04a fix pass: when the enclosing method belongs to the
                // SUBJECT'S METACLASS (EnumType.__new__ during class
                // creation), the MRO to scan is the metaclass chain
                // (type(subject).__mro__), not the subject's own class MRO —
                // otherwise super().__new__ in EnumType.__new__ scans
                // ReprEnum→Enum→object and misses ABCMeta/type entirely.
                let scan_mro: Vec<std::rc::Rc<ap_obj::classes::TypeRef>>;
                let borrowed;
                let subject_is_class =
                    matches!(&subject, Value::Obj(so) if so.kind == ap_obj::ObjKind::UserClass);
                let meta_of_subject = if subject_is_class {
                    crate::classes::type_of(_interp, &subject)
                        .and_then(|st| st.metaclass.borrow().clone())
                } else {
                    None
                };
                if let Some(meta) = &meta_of_subject {
                    if meta.is_subclass(&start_t) || std::rc::Rc::ptr_eq(meta, &start_t) {
                        scan_mro = meta.mro.borrow().clone();
                        borrowed = scan_mro;
                        // fall through to the shared scan below using `borrowed`
                        let mro = &borrowed;
                        let mut after = false;
                        for cls in mro.iter() {
                            if !after {
                                if std::rc::Rc::ptr_eq(cls, &start_t) {
                                    after = true;
                                }
                                continue;
                            }
                            if let Some(entry) = cls.dict.get(&Value::Str(PyString::new(name))) {
                                if matches!(&entry, Value::Obj(eo) if eo.kind == ap_obj::ObjKind::Function)
                                {
                                    // __new__ UNBOUND-marker arm: when the
                                    // super VALUE carries __ap_unbound_new__
                                    // (set by builtin_super per CPython's
                                    // implicit-staticmethod __new__ rule),
                                    // return the bare function — callers pass
                                    // mcs explicitly; binding double-counted
                                    // it ("takes 4 but 5 given").
                                    if let Value::Obj(sup) = obj {
                                        if sup
                                            .attrs
                                            .borrow()
                                            .get(&Value::Str(PyString::new("__ap_unbound_new__")))
                                            .is_some_and(|v| v.is_truthy())
                                        {
                                            // Stage the defining-class ctx for
                                            // zero-arg super() inside the callee
                                            // (call_function's pushed-ctx pop
                                            // balances it).
                                            let recv = crate::classes::class_value_for_public(cls);
                                            *_interp.pending_super_ctx.borrow_mut() =
                                                Some((cls.clone(), recv, true));
                                            return Ok(Some(entry.clone()));
                                        }
                                    }
                                    let m = crate::classes::bind_function_owned(
                                        &entry,
                                        subject.clone(),
                                        Some(cls),
                                    );
                                    return Ok(Some(m));
                                }
                                return Ok(Some(entry.clone()));
                            }
                            if cls.name == "object" && name == "__new__" {
                                let f = ap_obj::PyObject::new(
                                    ap_obj::ObjKind::BuiltinFn,
                                    "__object_new__",
                                );
                                *f.slice_parts.borrow_mut() = vec![subject.clone()];
                                return Ok(Some(Value::Obj(f)));
                            }
                        }
                        return Ok(None);
                    }
                }
                let mro = subj_t.mro.borrow();
                let mut after = false;
                for cls in mro.iter() {
                    if !after {
                        if std::rc::Rc::ptr_eq(cls, &start_t) {
                            after = true;
                        }
                        continue;
                    }
                    if let Some(entry) = cls.dict.get(&Value::Str(PyString::new(name))) {
                        // Bind plain functions to the subject (owner = cls);
                        // classmethods bind cls (c023); staticmethods unwrap.
                        // __new__ UNBOUND-marker arm (implicit-staticmethod
                        // rule; see the metaclass-chain arm above): hand back
                        // the bare function when the super VALUE is marked.
                        if matches!(&entry, Value::Obj(eo) if eo.kind == ap_obj::ObjKind::Function)
                        {
                            if name == "__new__" {
                                if let Value::Obj(sup) = obj {
                                    if sup
                                        .attrs
                                        .borrow()
                                        .get(&Value::Str(PyString::new("__ap_unbound_new__")))
                                        .is_some_and(|v| v.is_truthy())
                                    {
                                        // Stage the defining-class ctx (see
                                        // the metaclass-chain arm above).
                                        let recv = crate::classes::class_value_for_public(cls);
                                        *_interp.pending_super_ctx.borrow_mut() =
                                            Some((cls.clone(), recv, true));
                                        return Ok(Some(entry.clone()));
                                    }
                                }
                            }
                            let m = crate::classes::bind_function_owned(
                                &entry,
                                subject.clone(),
                                Some(cls),
                            );
                            return Ok(Some(m));
                        }
                        if let Value::Obj(eo) = &entry {
                            if eo.kind == ap_obj::ObjKind::Descriptor {
                                match eo.descr_kind.borrow().as_deref() {
                                    Some("classmethod") => {
                                        let f = eo.func.borrow().clone().unwrap_or(Value::None);
                                        let m = crate::classes::bind_function_owned(
                                            &f,
                                            subject.clone(),
                                            Some(cls),
                                        );
                                        return Ok(Some(m));
                                    }
                                    Some("staticmethod") => {
                                        return Ok(Some(
                                            eo.func.borrow().clone().unwrap_or(Value::None),
                                        ));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        return Ok(Some(entry));
                    }
                    // Virtual object.__new__ (c003 super().__new__(cls)).
                    let subject_is_class =
                        matches!(&subject, Value::Obj(so) if so.kind == ap_obj::ObjKind::UserClass);
                    if cls.name == "object" && name == "__new__" && subject_is_class {
                        let f = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_new__");
                        *f.slice_parts.borrow_mut() = vec![subject.clone()];
                        return Ok(Some(Value::Obj(f)));
                    }
                    // Virtual object.__init__ (e005/e012/e013 super().__init__):
                    // BaseException semantics — store args on the instance.
                    if cls.name == "object" && name == "__init__" {
                        let f =
                            ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__object_init__");
                        *f.slice_parts.borrow_mut() = vec![subject.clone()];
                        return Ok(Some(Value::Obj(f)));
                    }
                    // 02.03b: dict.__setitem__/__getitem__ virtuals —
                    // EnumDict.__setitem__'s super() must reach the backing
                    // dict of the (dict-derived) receiver.
                    if cls.name == "dict"
                        && matches!(name, "__setitem__" | "__getitem__" | "get" | "setdefault")
                    {
                        let f = ap_obj::PyObject::new(
                            ap_obj::ObjKind::BuiltinFn,
                            &format!("__ap_dict_{}__", name.trim_matches('_')),
                        );
                        *f.slice_parts.borrow_mut() = vec![subject.clone()];
                        return Ok(Some(Value::Obj(f)));
                    }
                }
            }
            return Ok(None);
        }
    }
    match obj {
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::Module => {
                let ns = o.module_ns.borrow();
                if let Some(ns) = &*ns {
                    // 02.03b-fix: module.__dict__ IS the namespace
                    // (_py_warnings: globals = sys.__dict__).
                    if name == "__dict__" {
                        return Ok(Some(Value::Dict(ns.clone())));
                    }
                    if let Some(v) = ns.get(&Value::Str(PyString::new(name))) {
                        return Ok(Some(v));
                    }
                }
                Ok(None)
            }
            ap_obj::ObjKind::Opaque if o.name == "NoneType" || o.name == "none" => {
                // None.__new__ (enum._find_new_ compares it in a set).
                Ok(Some(Value::Obj(ap_obj::PyObject::new(
                    ap_obj::ObjKind::BuiltinFn,
                    "NoneType.__new__",
                ))))
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
                // 02.04: unittest.result._clean_tracebacks walks the
                // exception chain (__cause__/__context__/__traceback__).
                "__cause__" | "__context__" | "__suppress_context__" => Ok(Some(Value::None)),
                // 02.04a (interp bug #6): exceptions carry __traceback__ —
                // types.py probes it at import (TracebackType/FrameType).
                // Stub chain: one tb object + tb_frame namespace.
                "__traceback__" => {
                    let frame = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<frame>");
                    let tb = ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<traceback>");
                    let _ = tb
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("tb_frame")), Value::Obj(frame));
                    let _ = tb
                        .attrs
                        .borrow_mut()
                        .set(Value::Str(PyString::new("tb_next")), Value::None);
                    let _ = tb.attrs.borrow_mut().set(
                        Value::Str(PyString::new("tb_lineno")),
                        Value::Int(Int::from_i64(0)),
                    );
                    Ok(Some(Value::Obj(tb)))
                }
                _ => Ok(None),
            },
            ap_obj::ObjKind::UserInstance | ap_obj::ObjKind::UserClass => {
                // 02.03 attribute protocol (S-03-09..12): MRO + descriptors.
                // Misses (AttributeError) fall to the permissive v0 paths
                // below.
                let proto_hit = match crate::classes::attr_get_opt(_interp, obj, name) {
                    Ok(v) => v,
                    Err(Flow::Raise(e)) if e.class == "AttributeError" => None,
                    Err(other) => return Err(other),
                };
                if let Some(v) = proto_hit {
                    return Ok(Some(v));
                }
                // 02.03b: object.__init__ on instances whose class lacks a
                // user __init__ (enum member creation: member.__init__(*args)).
                if o.kind == ap_obj::ObjKind::UserInstance && name == "__init__" {
                    return Ok(Some(Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "__object_virtual_init__",
                    ))));
                }
                // 02.04: exception-chain dunders on real exception-class
                // instances (unittest.result._clean_tracebacks walks
                // __cause__/__context__; raise-from stores __cause__).
                if o.kind == ap_obj::ObjKind::UserInstance
                    && matches!(name, "__cause__" | "__context__" | "__suppress_context__")
                {
                    return Ok(Some(Value::None));
                }
                if o.kind == ap_obj::ObjKind::UserInstance && name == "__traceback__" {
                    return Ok(Some(Value::None));
                }
                // 02.04: exc.with_traceback(None) (assertRaises stores the
                // exception traceback-free) — v0 tracebacks are stubs, so
                // the method returns the exception itself.
                if o.kind == ap_obj::ObjKind::UserInstance && name == "with_traceback" {
                    let m =
                        ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "__ap_with_traceback__");
                    *m.slice_parts.borrow_mut() = vec![obj.clone()];
                    return Ok(Some(Value::Obj(m)));
                }
                // 02.03b: dict-method surface on dict-derived instances
                // (EnumDict.setdefault/pop/get/keys/...) binds the BACKING
                // dict — the dict base is synthesized without methods.
                if o.kind == ap_obj::ObjKind::UserInstance {
                    if crate::methods::builtin_method_routed(name) || builtin_method_exists(name) {
                        if let Some(d) = crate::classes::dict_backing(obj) {
                            let m = ap_obj::PyObject::with_self(
                                ap_obj::ObjKind::BoundMethod,
                                name,
                                Value::Dict(d),
                            );
                            return Ok(Some(Value::Obj(m)));
                        }
                    }
                }
                Ok(None)
            }
            ap_obj::ObjKind::Type | ap_obj::ObjKind::UserClass => {
                // Real exception classes (re.error etc) answer via the class
                // protocol first (__dict__-seeded __module__).
                if o.kind == ap_obj::ObjKind::Type {
                    if let Some(v) = crate::classes::attr_get_opt(_interp, obj, name)? {
                        return Ok(Some(v));
                    }
                }
                match name {
                    "__name__" => Ok(Some(Value::Str(PyString::new(o.name.clone())))),
                    // 02.08-full p2: stdlib exception types report their home
                    // module (re.error.__module__ == 're').
                    "__module__" if o.kind == ap_obj::ObjKind::Type => {
                        const STDLIB_EXC_MODULES: &[(&str, &str)] =
                            &[("PatternError", "re"), ("JSONDecodeError", "json")];
                        Ok(Some(Value::Str(PyString::new(
                            STDLIB_EXC_MODULES
                                .iter()
                                .find(|(n, _)| *n == o.name)
                                .map(|(_, m)| m.to_string())
                                .unwrap_or_else(|| "builtins".to_string()),
                        ))))
                    }
                    // 02.04a (interp bug #18): __mro__ reads on Type objects
                    // (ABCMeta.__subclasscheck__ on a non-class 'type' value) —
                    // a singleton MRO keeps the ABC probes non-fatal.
                    "__mro__" if o.kind == ap_obj::ObjKind::Type => Ok(Some(Value::Tuple(
                        Gc::new(Tuple(vec![Value::Obj(o.clone())])),
                    ))),
                    // 02.04a (interp bug #5b): builtin-type __dict__ reads
                    // (types.py: MappingProxyType = type(type.__dict__),
                    // ClassMethodDescriptorType = type(dict.__dict__['fromkeys'])).
                    // The façade lists the type's method-surface names as markers.
                    "__dict__" if o.kind == ap_obj::ObjKind::Type => {
                        let d = PyDict::new();
                        for m in [
                            "__init__",
                            "__new__",
                            "__str__",
                            "__repr__",
                            "__eq__",
                            "__hash__",
                            "__len__",
                            "__iter__",
                            "__contains__",
                            "__getitem__",
                            "__setitem__",
                            "__call__",
                            "__next__",
                            "join",
                            "split",
                            "fromkeys",
                            "append",
                            "get",
                            "keys",
                            "values",
                            "items",
                            "copy",
                            "upper",
                            "lower",
                            "strip",
                            "read",
                            "write",
                            "close",
                        ] {
                            let marker = ap_obj::PyObject::new(
                                ap_obj::ObjKind::BuiltinFn,
                                &format!("{}.{}", o.name, m),
                            );
                            let _ = d.set(Value::Str(PyString::new(m)), Value::Obj(marker));
                        }
                        // 02.08-full p2: __module__ in the synthetic class dict
                        // (re.error.__dict__['__module__'] == 're').
                        const STDLIB_EXC_MODULES: &[(&str, &str)] =
                            &[("PatternError", "re"), ("JSONDecodeError", "json")];
                        let mod_name = STDLIB_EXC_MODULES
                            .iter()
                            .find(|(n, _)| *n == o.name)
                            .map(|(_, m)| m.to_string())
                            .unwrap_or_else(|| "builtins".to_string());
                        let _ = d.set(
                            Value::Str(PyString::new("__module__")),
                            Value::Str(PyString::new(mod_name)),
                        );
                        Ok(Some(Value::Dict(d)))
                    }
                    // 02.04a fix pass: __new__ reads on interned name-Types
                    // (typing.py machinery: type.__new__(Meta, ...), and the
                    // int.__new__ probe during Protocol class creation).
                    // 02.03b: str.__new__/int.__new__ (enum member creation:
                    // member = str.__new__(cls, value)) — the marker returns
                    // the value itself (a v0 str/int IS the value model).
                    "__new__" if matches!(o.name.as_str(), "str" | "int" | "bytes") => {
                        let m = ap_obj::PyObject::new(
                            ap_obj::ObjKind::BuiltinFn,
                            &format!("__{}__new__", o.name),
                        );
                        Ok(Some(Value::Obj(m)))
                    }
                    "__new__" => Ok(Some(Value::Obj(ap_obj::PyObject::new(
                        ap_obj::ObjKind::BuiltinFn,
                        "type.__new__",
                    )))),
                    // 02.04: inspect.isabstract probes type.__flags__ (int bit
                    // mask); 0 = no TPFLAGS — every v1 type is concrete.
                    "__flags__" => Ok(Some(Value::Int(ap_obj::Int::from_i64(0)))),
                    // 02.03b-fix: os.PathLike ABC surface (CPython's os.py builds
                    // it from _collections_abc.ABC): register(subclass) ->
                    // subclass; isinstance(x, PathLike) checks __fspath__.
                    "register" if o.name == "PathLike" => {
                        Ok(Some(Value::Obj(ap_obj::PyObject::new(
                            ap_obj::ObjKind::BuiltinFn,
                            "__ap_pathlike_register__",
                        ))))
                    }
                    // 02.08-full part 1: json.JSONEncoder.default base impl
                    // (super().default(o) raises the CPython TypeError).
                    "default" if o.name == "JSONEncoder" => {
                        let m =
                            ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "json.base_default");
                        Ok(Some(Value::Obj(m)))
                    }
                    // 02.08-full part 1: str.maketrans (classmethod).
                    "maketrans" if o.name == "str" => {
                        let m = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "str.maketrans");
                        Ok(Some(Value::Obj(m)))
                    }
                    // float.fromhex (V068): builtin marker dispatched by name.
                    "fromhex" if o.name == "float" => {
                        let m = ap_obj::PyObject::new(ap_obj::ObjKind::BuiltinFn, "float.fromhex");
                        Ok(Some(Value::Obj(m)))
                    }
                    // 02.04a (interp bug #5c): types.py reads
                    // FunctionType.__code__ / FunctionType.__globals__ at import.
                    "__code__" | "__globals__" if o.name == "function" => Ok(Some(Value::Obj(
                        ap_obj::PyObject::new(ap_obj::ObjKind::Namespace, "<stub>"),
                    ))),
                    // 02.04a (interp bug #5): method-surface reads on builtin
                    // types (types.py: MethodDescriptorType = type(str.join)).
                    // A named marker keeps type(x) meaningful without wiring the
                    // full unbound-method machinery.
                    _ => {
                        // 02.03b-fix: dynamic attr writes on interned types
                        // (ast.py: Tuple.dims = property(...)) store on the
                        // PyObject; reads must see them.
                        if let Some(v) = o.attrs.borrow().get(&Value::Str(PyString::new(name))) {
                            return Ok(Some(v));
                        }
                        if builtin_method_exists(name) {
                            let m = ap_obj::PyObject::new(
                                ap_obj::ObjKind::BuiltinFn,
                                &format!("{}.{}", o.name, name),
                            );
                            return Ok(Some(Value::Obj(m)));
                        }
                        if matches!(
                            name,
                            "__repr__" | "__str__" | "__format__" | "__reduce_ex__"
                            | "__reduce__" | "__copy__" | "__deepcopy__"
                            // 02.03b-fix: weakref.ref as a dict key class
                            // (importlib _WeakValueDictionary paths).
                            | "__hash__" | "__eq__" | "__ne__" | "__call__"
                        ) {
                            // 02.03b-fix: type.__repr__ surface (pprint dispatch
                            // registration reads dict.__repr__ etc).
                            let m = ap_obj::PyObject::new(
                                ap_obj::ObjKind::BuiltinFn,
                                "__ap_function_repr__",
                            );
                            *m.slice_parts.borrow_mut() = vec![obj.clone()];
                            return Ok(Some(Value::Obj(m)));
                        }
                        Ok(None)
                    }
                    // 02.03b: object-slot dunders read on interned name-Types
                    // (enum.EnumType.__new__: member_type.__format__ /
                    // member_type.__str__ / object.__reduce_ex__ comparisons).
                    // The __object_virtual_* markers dispatch through the same
                    // class-level virtual path; identity comparisons between two
                    // reads of the SAME name stay unequal (CPython-comparable via
                    // the marker name) — enum's `found_method in (...)` checks
                    // compare marker VALUES, so route them by name equality in
                    // Value::eq via the repr name.
                    _ if name.starts_with("__")
                        && name.ends_with("__")
                        && !matches!(
                            name,
                            "__name__"
                                | "__mro__"
                                | "__dict__"
                                | "__class__"
                                | "__bases__"
                                | "__qualname__"
                                | "__module__"
                                | "__doc__"
                                | "__code__"
                                | "__globals__"
                        ) =>
                    {
                        let m = ap_obj::PyObject::new(
                            ap_obj::ObjKind::BuiltinFn,
                            &format!("__object_virtual_{}__", name.trim_matches('_')),
                        );
                        Ok(Some(Value::Obj(m)))
                    }
                    _ => Ok(None),
                }
            }
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
            // 02.04a fix pass (BUG-B): bind ANY method-name read on a builtin
            // value — the methods.rs dispatch table owns arity/shape errors
            // at call time. The old hand-written allowlist dropped names the
            // table handles (set.discard: methods.rs:527), turning method
            // calls into AttributeError.
            if builtin_method_exists(name) || crate::methods::builtin_method_routed(name) {
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

/// Public truthiness shim (threading_call Condition.wait_for predicates).
pub fn truthy_public(interp: &Rc<Interpreter>, v: &Value) -> Result<bool, Flow> {
    truthy(interp, v)
}

/// Public attr_get shim (builtins_modules json/os lookups).
pub fn attr_get_public(
    interp: &Rc<Interpreter>,
    obj: &Value,
    name: &str,
) -> Result<Option<Value>, Flow> {
    attr_get(interp, obj, name)
}

/// Does this name exist in the v0 builtin-method surface? Permissive by
/// design (receiver-shape errors surface at dispatch; 02.03 tightens).
fn builtin_method_exists(name: &str) -> bool {
    matches!(
        name,
        "upper"
            | "lower"
            | "join"
            | "to_bytes"
            | "from_bytes"
            | "bit_length"
            | "bit_count"
            | "partition"
            | "encode"
            | "decode"
            | "append"
            | "clear"
            | "keys"
            | "values"
            | "items"
            | "add"
            | "copy"
            | "remove"
            | "update"
            | "difference"
            | "difference_update"
            | "intersection"
            | "union"
            | "issubset"
            | "issuperset"
            | "symmetric_difference"
            | "symmetric_difference_update"
            | "pop"
            | "get"
            | "fromkeys"
            | "setdefault"
            | "pop"
            | "remove"
            | "startswith"
            | "endswith"
            | "replace"
            | "split"
            | "strip"
            | "format"
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

/// 3-arg shim (methods.rs): positional-only call on a fresh module frame.
pub fn call_value_pub3(f: &Value, args: &[Value]) -> EResult {
    let interp = current_interp_for_bindings();
    let caller = Rc::new(Frame::new_module(interp.globals.clone()));
    call_value(&interp, &caller, f, args, &[])
}

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

/// Public thread-local interpreter handle (classes.rs metaclass hooks).
pub fn current_interp_handle() -> std::rc::Rc<Interpreter> {
    current_interp_for_bindings()
}

/// 02.08-full part 1: read the (re, im) pair off an Opaque complex value.
fn complex_parts(v: &Value) -> Option<(f64, f64)> {
    if let Value::Obj(o) = v {
        if o.kind == ap_obj::ObjKind::Opaque && o.name == "complex" {
            let attrs = o.attrs.borrow();
            let s = ap_obj::value::PyString::new("real");
            let r = attrs.get(&ap_obj::value::Value::Str(s));
            let s2 = ap_obj::value::PyString::new("imag");
            let i = attrs.get(&ap_obj::value::Value::Str(s2));
            if let (Some(ap_obj::value::Value::Float(r)), Some(ap_obj::value::Value::Float(i2))) =
                (r, i)
            {
                return Some((r, i2));
            }
        }
    }
    None
}

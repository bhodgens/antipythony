//! PEP-342 resumable generators (lazy suspension).
//!
//! The v0 executor materialized generator bodies eagerly: the first pull ran
//! the whole body, catching `Flow::YieldBack` into a sink, then replayed the
//! stream. That is unobservable only for programs that never interleave
//! observation with production — pytest's pluggy wrapper hooks (issue #7),
//! contextlib._GeneratorContextManager, and async are not such programs.
//!
//! This module replaces the def-generator path with a STACKFUL FIBER per
//! generator (corosensei): the fiber's Rust stack *is* the suspended frame —
//! locals, call depth, loop cursors, and the try/except stack simply live on
//! it between resumes. `Expr::Yield` suspends by handing the value to the
//! parent stack; `send`/`throw`/`close` resume the fiber with a GenInput
//! that the yield site turns into the yield-expression's value, a raise at
//! the suspension point, or GeneratorExit.
//!
//! NESTING (yield from): each fiber's identity is an explicit (fiber_id,
//! yielder) pair on a thread-local stack. The invariant: the TOP entry is
//! always the fiber that is about to execute the next statement. Suspending
//! a fiber moves its entry to PARKED (so a nested fiber can run with a
//! correct top); resuming moves it back on top. gen_yield always suspends
//! the ACTIVE top.
//!
//! Generator expressions keep the materialization path (documented).

use std::cell::RefCell;

use corosensei::{Coroutine, CoroutineResult};

use crate::frame::FnEntry;
use crate::Flow;
use ap_obj::value::Value;

/// What the resumer hands INTO the fiber.
pub enum GenInput {
    /// send(v): v becomes the yield expression's value (None on first resume).
    Send(Value),
    /// throw(e): raise e AT the suspension point.
    Throw(crate::exc_hier::ExceptionValue),
    /// close(): raise GeneratorExit at the suspension point.
    Close,
}

/// What the fiber hands back OUT at a suspension.
pub enum GenOutput {
    /// yield v — the fiber is now suspended.
    Yield(Value),
}

type FiberYielder = corosensei::Yielder<GenInput, GenOutput>;

/// Per-generator state (stored in interp.generators behind GenFrame::Live).
pub struct GenState {
    pub fiber: Option<Coroutine<GenInput, GenOutput, ()>>,
    pub started: bool,
    pub running: bool,
    /// Terminal: closed()/exhausted without the fiber ever having run (the
    /// unstarted close()/throw() early-retire). All later resumes are
    /// StopIteration — the fiber must never be started after retirement.
    pub retired: bool,
    /// W112-T1 (scrapy DepthMiddleware async-gen super()): the bound-method
    /// zero-arg super() context staged when the (async) generator function
    /// was CALLED. The fiber body runs on the DRIVER's stack (Task.__step /
    /// gen_pull), where no method ctx sits — CPython keeps __class__ + self
    /// on the generator's OWN frame across resumes, so resume_live pushes
    /// this ctx around every resume slice and pops it after.
    pub super_ctx: Option<(std::rc::Rc<ap_obj::classes::TypeObject>, Value, bool)>,
}

thread_local! {
    /// Stack of (fiber_id, yielder) for fibers whose body is executing on
    /// this thread. TOP = the fiber that executes the next statement.
    static ACTIVE: RefCell<Vec<(u64, *const FiberYielder)>> = RefCell::new(Vec::new());
    /// Parked (fiber_id, yielder) for fibers SUSPENDED at a yield while an
    /// outer fiber continues running (nested generators). Re-activated on
    /// resume.
    static PARKED: RefCell<Vec<(u64, *const FiberYielder)>> = RefCell::new(Vec::new());
    /// The fiber id whose body is executing on this stack segment.
    static CURRENT: RefCell<Vec<u64>> = RefCell::new(Vec::new());
    static NEXT_FIBER: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// Run the fiber body with its identity established as the ACTIVE top.
/// The fiber id of the generator body CURRENTLY executing on this
/// thread (0 = none). Used to key pending generator errors per fiber
/// so a nested resume (unraisable hook during alloc) cannot steal the
/// outer body's staged error.
pub fn current_running_fiber_id() -> u64 {
    CURRENT.with(|c| c.borrow().last().copied().unwrap_or(0))
}

fn run_fiber_body<R>(fiber_id: u64, y: *const FiberYielder, f: impl FnOnce() -> R) -> R {
    CURRENT.with(|c| c.borrow_mut().push(fiber_id));
    ACTIVE.with(|a| a.borrow_mut().push((fiber_id, y)));
    let r = f();
    ACTIVE.with(|a| {
        let mut b = a.borrow_mut();
        if b.last().map(|(id, _)| *id) == Some(fiber_id) {
            b.pop();
        } else if let Some(pos) = b.iter().rposition(|(id, _)| *id == fiber_id) {
            b.remove(pos);
        }
    });
    CURRENT.with(|c| {
        c.borrow_mut().pop();
    });
    r
}

/// True when the current Rust stack is inside a generator fiber body.
pub fn in_fiber() -> bool {
    CURRENT.with(|c| !c.borrow().is_empty())
}

/// The `yield` expression: suspend the ACTIVE-top fiber. While suspended its
/// identity is moved to PARKED so nested fibers see a consistent top.
pub fn gen_yield(v: Value) -> Result<Value, Flow> {
    let fiber_id = ACTIVE.with(|a| a.borrow().last().map(|(id, _)| *id));
    let Some(fiber_id) = fiber_id else {
        return Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "SyntaxError",
            "'yield' outside function",
        )));
    };
    // Lift our entry from ACTIVE to PARKED for the suspension window.
    let y: *const FiberYielder = {
        let mut parked_ok = false;
        let y = ACTIVE.with(|a| {
            let mut b = a.borrow_mut();
            if let Some(pos) = b.iter().rposition(|(id, _)| *id == fiber_id) {
                let (_, yptr) = b.remove(pos);
                parked_ok = true;
                yptr
            } else {
                std::ptr::null()
            }
        });
        if parked_ok {
            PARKED.with(|p| p.borrow_mut().push((fiber_id, y)));
        }
        y
    };
    if y.is_null() {
        return Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "SyntaxError",
            "'yield' outside function",
        )));
    }
    // SAFETY: the yielder lives in the suspended coroutine's initial frame;
    // the coroutine object stays alive in interp.generators for the whole
    // suspension window.
    let resume_in = unsafe { (*y).suspend(GenOutput::Yield(v)) };
    // Resumed: move our identity back from PARKED to ACTIVE (on top).
    PARKED.with(|p| {
        let mut b = p.borrow_mut();
        if let Some(pos) = b.iter().rposition(|(id, _)| *id == fiber_id) {
            let (_, yptr) = b.remove(pos);
            drop(b);
            ACTIVE.with(|a| a.borrow_mut().push((fiber_id, yptr)));
        }
    });
    match resume_in {
        GenInput::Send(val) => Ok(val),
        GenInput::Throw(e) => Err(Flow::Raise(e)),
        GenInput::Close => Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "GeneratorExit",
            "",
        ))),
    }
}

/// Run one generator body inside its fiber. Terminal states:
/// - Ok / Flow::Return: body done (PEP-342: return X == StopIteration(X);
///   the caller maps Done -> StopIteration carrying interp's si_value).
/// - Flow::Raise(StopIteration): PEP-479 — surfaced as RuntimeError by
///   pump_outcome.
/// - other Flow::Raise: propagate to the resumer (pending-error slot).
fn gen_fiber_body(
    interp: std::rc::Rc<crate::frame::Interpreter>,
    entry: FnEntry,
    func_obj: ap_obj::int::Gc<ap_obj::PyObject>,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
) {
    let def = entry.def.clone();
    let lambda_body = interp
        .lambda_bodies
        .borrow()
        .get(&func_obj.fn_id.unwrap_or(u32::MAX))
        .cloned();
    let bound = crate::interp::bind_params_for_generator(&interp, &entry, &def, &func_obj, &args, &kwargs);
    let (locals, my_cells, free_cells) = match bound {
        Ok(b) => b,
        Err(f) => {
            // Argument binding errors surface on FIRST resume (CPython
            // defers them too — the call itself cannot raise).
            crate::interp::set_pending_generator_error(f);
            return;
        }
    };
    let frame = std::rc::Rc::new(crate::frame::Frame::new_function(
        entry.globals.clone(),
        &entry.scope_path,
        &entry.qualname,
        crate::interp::scope_globals_pub(&interp, &entry),
        crate::interp::scope_nonlocals_pub(&interp, &entry),
        my_cells,
        free_cells,
        locals,
    ));
    let line_track = std::rc::Rc::new(std::cell::Cell::new(0usize));
    for _ in 0..2000 {
        match interp.frame_stack.try_borrow_mut() {
            Ok(mut fs) => {
                fs.push(line_track.clone());
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
    let reg = crate::interp::interp_registry(&interp);
    let result: Result<Value, Flow> = if let Some(body) = lambda_body {
        crate::interp::eval_expr_on_frame(&interp, &frame, &body, &reg)
    } else {
        crate::interp::exec_block(&interp.clone(), &frame, &def.body.blocks, &reg)
            .map(|_| Value::None)
    };
    for _ in 0..2000 {
        match interp.frame_stack.try_borrow_mut() {
            Ok(mut fs) => {
                let _ = fs.pop();
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
    if let Err(Flow::Raise(e)) = &result {
        if e.class == "NameError" && std::env::var("APX_GENDbg").is_ok() {
            let gname = entry
                .globals
                .get(&Value::Str(ap_obj::PyString::new("__name__")))
                .map(|v| crate::builtins::py_str_of(&v).unwrap_or_default())
                .unwrap_or_default();
            let has_sys = entry
                .globals
                .get(&Value::Str(ap_obj::PyString::new("sys")))
                .is_some();
            eprintln!("W-GEN NameError gen module={} has-sys={}", gname, has_sys);
        }
    }
    match result {
        Ok(_) | Err(Flow::Return(_)) => {
            // PEP-342: `return X` == StopIteration(X) (S-02-22). Park X for
            // the resumer; a plain Ok/None return parks nothing (None).
            if let Err(Flow::Return(v)) = result {
                interp.si_value.borrow_mut().replace(v);
            }
        }
        Err(Flow::Raise(mut e)) => {
            // Frame-exit traceback bookkeeping. T2 gate finding 2: the
            // per-exception tb is what pytest's formatter reads (pluggy
            // hooks ARE generators) — a generator frame that only landed
            // in the global tb_frames table vanished from e.__traceback__.
            // Insert(0) matches call_function (innermost first at record
            // time; the table renders outermost-first). The frame's OWN
            // last_line is the true source line (line_track carries the
            // global-ordinal mapping, which misattributes across modules).
            // frame.last_line is the statement's REAL source line (the
            // Stmt::Line markers); line_track carries the legacy global
            // ordinal mapping — prefer the real one, fall back to it.
            let line = {
                let fl = frame.last_line.get();
                if fl > 0 { fl } else { line_track.get() }
            };
            let file = crate::interp::fn_entry_file(&interp, &entry);
            if std::env::var("APX_GEN_DEBUG").is_ok() {
                let msg = e
                    .args
                    .first()
                    .map(|v| {
                        crate::builtins::py_str_of(v)
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                let tbtxt: Vec<String> = e
                    .tb
                    .iter()
                    .map(|(q, l, f)| format!("  {}:{} ({})", q, l, f))
                    .collect();
                eprintln!(
                    "[GEN-RAISE] fiber={} class={} msg={} at {}:{} ({}) tb_len={}\n{}",
                    current_running_fiber_id(),
                    e.class,
                    msg,
                    entry.qualname,
                    line,
                    file,
                    e.tb.len(),
                    tbtxt.join("\n")
                );
            }
            e.tb.insert(0, (entry.qualname.clone(), line, file.clone()));
            interp
                .tb_frames
                .borrow_mut()
                .insert(0, (entry.qualname.clone(), line, file));
            // fiber_id is owned by run_fiber_body's closure scope; the
            // CURRENT stack still holds it while the body executes.
            let fid = current_running_fiber_id();
            crate::interp::set_pending_generator_error_for(fid, Flow::Raise(e));
        }
        Err(_) => {}
    }
}

/// Resume a live generator (in interp.generators[id]) with input.
/// Ok(Some(v)) = yielded v; Ok(None) = exhausted (StopIteration);
/// Err(Flow::Raise) = the body's exception (surfaces to the CALLER).
pub fn resume_live(
    interp: &std::rc::Rc<crate::frame::Interpreter>,
    id: u64,
    input: GenInput,
) -> Result<Option<Value>, Flow> {
    // Pull the live state out of the registry for the duration of the
    // resume (a re-entrant resume of the SAME generator must not find it).
    let mut live = {
        let mut gens = interp.generators.borrow_mut();
        match gens.remove(&id) {
            Some(crate::interp::GenFrame::Live(st)) => st,
            other => {
                // Not live (shouldn't happen: ensure_fiber ran). Put back.
                if let Some(g) = other {
                    interp.generators.borrow_mut().insert(id, g);
                }
                return match input {
                    GenInput::Throw(e) => Err(Flow::Raise(e)),
                    _ => Ok(None),
                };
            }
        }
    };
    if live.running {
        interp
            .generators
            .borrow_mut()
            .insert(id, crate::interp::GenFrame::Live(live));
        return Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
            "ValueError",
            "generator already executing",
        )));
    }
    let Some(mut fiber) = live.fiber.take() else {
        interp
            .generators
            .borrow_mut()
            .insert(id, crate::interp::GenFrame::Live(live));
        return match input {
            GenInput::Throw(e) => Err(Flow::Raise(e)),
            _ => Ok(None),
        };
    };
    if live.retired || fiber.done() {
        live.fiber = Some(fiber);
        interp
            .generators
            .borrow_mut()
            .insert(id, crate::interp::GenFrame::Live(live));
        return match input {
            GenInput::Throw(e) => {
                // throw() on an exhausted/closed generator: StopIteration
                // (CPython), not the thrown object.
                if e.class == "StopIteration" || e.class == "GeneratorExit" {
                    Err(Flow::Raise(e))
                } else {
                    Err(Flow::Raise(crate::exc_hier::ExceptionValue::msg(
                        "StopIteration",
                        "",
                    )))
                }
            }
            _ => Ok(None),
        };
    }
    live.running = true;
    interp.enter_call()?;
    // PEP-342: a throw()/close() into a NOT-YET-STARTED generator never
    // runs the body. Stage the error for the resumer; the single resume
    // starts the closure, which unwinds immediately without body effects
    // (the closure itself never observes the throw — the error surfaces
    // from the pending slot on Return).
    if std::env::var("APX_GEN_DEBUG").is_ok() {
        if let GenInput::Throw(e) = &input {
            let tbtxt: Vec<String> = e
                .tb
                .iter()
                .map(|(q, l, f)| format!("  {}:{} ({})", q, l, f))
                .collect();
            eprintln!(
                "[THROW-IN] fiber={} id={} class={} tb_len={}\n{}",
                current_running_fiber_id(),
                id,
                e.class,
                e.tb.len(),
                tbtxt.join("\n")
            );
        }
    }
    if !live.started {
        let pre_start: Option<crate::exc_hier::ExceptionValue> = match &input {
            GenInput::Throw(e) => Some(e.clone()),
            GenInput::Close => {
                Some(crate::exc_hier::ExceptionValue::msg("GeneratorExit", ""))
            }
            GenInput::Send(_) => None,
        };
        if let Some(e) = pre_start {
            crate::interp::set_pending_generator_error(Flow::Raise(e));
            // Retire the fiber WITHOUT starting it: corosensei has no
            // "abort before start", so force-reset the state; `retired`
            // makes every later resume a clean StopIteration.
            live.running = false;
            live.fiber = Some(fiber);
            live.started = true;
            live.retired = true;
            interp
                .generators
                .borrow_mut()
                .insert(id, crate::interp::GenFrame::Live(live));
            interp.exit_call();
            if let Some(f) = crate::interp::take_pending_generator_error() {
                return Err(f);
            }
            return Ok(None);
        }
    }
    let out = {
        // W112-T1: a generator method's zero-arg super() context rides the
        // fiber, not the driver stack. Push it just for this resume slice
        // (the body runs to the next yield or completion), then pop. A
        // nested plain call inside the body pushes/pops its own sentinel
        // above/below within the slice, leaving our ctx on top when the
        // body finally suspends — the pop below restores parity.
        let ctx = live.super_ctx.clone();
        if let Some(c) = &ctx {
            interp.super_stack.borrow_mut().push(c.clone());
        }
        let r = fiber.resume(input);
        if ctx.is_some() {
            interp.super_stack.borrow_mut().pop();
        }
        r
    };
    interp.exit_call();
    live.running = false;
    if fiber.done() {
        live.fiber = None;
    } else {
        live.fiber = Some(fiber);
    }
    live.started = true;
    interp
        .generators
        .borrow_mut()
        .insert(id, crate::interp::GenFrame::Live(live));
    match out {
        CoroutineResult::Yield(GenOutput::Yield(v)) => Ok(Some(v)),
        CoroutineResult::Return(()) => {
            if let Some(f) = crate::interp::take_pending_generator_error() {
                return Err(f);
            }
            Ok(None)
        }
    }
}

/// Build the fiber for a def-generator (does NOT start it — corosensei runs
/// the closure lazily on first resume). The fiber stack is 64MB: nested
/// genexpr/listcomp evaluation inside a generator body costs real Rust
/// stack, and the default 1MB corosensei stack overflows on
/// vendored-stdlib shapes.
pub fn make_fiber(
    interp: std::rc::Rc<crate::frame::Interpreter>,
    entry: FnEntry,
    func_obj: ap_obj::int::Gc<ap_obj::PyObject>,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
) -> Coroutine<GenInput, GenOutput, ()> {
    let fiber_id = NEXT_FIBER.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    Coroutine::with_stack(
        corosensei::stack::DefaultStack::new(64 * 1024 * 1024).expect("generator fiber stack"),
        move |y, _first_input| {
            let yptr: *const FiberYielder = y;
            run_fiber_body(fiber_id, yptr, || {
                gen_fiber_body(interp, entry, func_obj, args, kwargs)
            })
        },
    )
}

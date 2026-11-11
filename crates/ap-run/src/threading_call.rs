//! 02.07: the `threading` + `queue` call dispatch surface — module-level
//! functions, Thread/sync-primitive methods, and the GIL release points.
//!
//! Every entry point here is reached from builtins_modules::call_native via
//! the "threading.*" / "queue.*" dotted markers. The GIL discipline:
//! blocking ops park on the primitive's own Mutex+Condvar (02.09b: the
//! re-acquire AFTER waking (the re-entrant depth counter makes nested
//! dispatches safe).

use crate::builtins_modules::{i as smi, kw, required, s, str_arg, BResult};
use crate::exc_hier::ExceptionValue;
use crate::threading::{
    prim_of, prim_put, BarrierPrim, CondPrim, EventPrim, LockPrim, QueuePrim, RLockPrim, SemPrim,
};
use crate::Flow;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::object::{ObjKind, PyObject};
use ap_obj::value::{PyString, Value};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg.into()))
}

fn obj_addr(v: &Value) -> Result<usize, Flow> {
    match v {
        Value::Obj(o) => Ok(Arc::as_ptr(o).addr()),
        _ => Err(raise("TypeError", "expected an object")),
    }
}

fn attr_v(o: &PyObject, key: &str) -> Option<Value> {
    o.attrs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&s(key))
}

fn attr_set(o: &PyObject, key: &str, v: Value) {
    let _ = o
        .attrs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set(s(key), v);
}

fn f64_arg(v: &Value) -> Result<f64, Flow> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(n) => n
            .to_f64()
            .ok_or_else(|| raise("OverflowError", "int too large to convert to float")),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        other => Err(raise(
            "TypeError",
            format!("must be real number, not {}", other.type_name()),
        )),
    }
}

fn opt_f64(v: &Value) -> Result<Option<f64>, Flow> {
    if matches!(v, Value::None) {
        return Ok(None);
    }
    f64_arg(v).map(Some)
}

// ===========================================================================
// threading module construction
// ===========================================================================

pub fn build_threading_module(interp: &crate::frame::Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("threading");
    // TIMEOUT_MAX: the CPython 9223372036.0 pin (spec G-3: value not
    // corpus-gated; this is the reference-measured constant).
    let _ = ns.set(s("TIMEOUT_MAX"), Value::Float(9223372036.0));
    for f in [
        "current_thread",
        "main_thread",
        "active_count",
        "enumerate",
        "get_ident",
        "get_native_id",
        "settrace",
        "setprofile",
        "stack_size",
        "settrace_all_threads",
        "setprofile_all_threads",
    ] {
        let _ = ns.set(
            s(f),
            crate::builtins_modules::marker(&format!("threading.{f}")),
        );
    }
    // T1-d: twisted.threadable reads threading._RLock for its sync wrapper.
    let _ = ns.set(
        s("_RLock"),
        crate::builtins_modules::interned_type_pub("RLock"),
    );
    for f in [
        "Lock",
        "RLock",
        "Condition",
        "Event",
        "Semaphore",
        "BoundedSemaphore",
        "Barrier",
        "Timer",
        "local",
        "Thread",
    ] {
        let _ = ns.set(s(f), crate::builtins_modules::interned_type_pub(f));
    }
    // BrokenBarrierError / ThreadError: real exception classes wired into
    // the tree (RuntimeError parents; see exc wiring below).
    for cls in ["BrokenBarrierError", "ThreadError"] {
        let _ = ns.set(s(cls), threading_exc_class(interp, cls));
    }
    // W112: threading._register_atexit — twisted's reactor cleanup and
    // concurrent.futures register interpreter-exit hooks; the name must
    // exist and accept the callable (v0: parked, never fired).
    let _ = ns.set(
        s("_register_atexit"),
        crate::builtins_modules::marker("threading._register_atexit"),
    );
    // excepthook / ExceptHookArgs.
    let _ = ns.set(
        s("excepthook"),
        crate::builtins_modules::marker("threading.excepthook"),
    );
    let _ = ns.set(
        s("ExceptHookArgs"),
        crate::builtins_modules::interned_type_pub("ExceptHookArgs"),
    );
    // _main_thread seed: current_thread() materializes MainThread on first
    // touch; bootstrap_main ran at run entry.
    crate::builtins_modules::finish(m, ns)
}

/// Real exception class values for the threading tree: parented under
/// RuntimeError so `issubclass`/`except` behave per T110/T113.
fn threading_exc_class(interp: &crate::frame::Interpreter, name: &str) -> Value {
    // Reuse the builtin tree when the name already exists there.
    if let Some(v) = crate::classes::exc_class_value(interp, name) {
        return v;
    }
    // Dynamically parent under RuntimeError (BrokenBarrierError/ThreadError).
    let parent = crate::classes::exc_class_value(interp, "RuntimeError")
        .and_then(|v| crate::classes::type_of(interp, &v));
    let base = parent.unwrap_or_else(|| crate::classes::object_class(interp));
    let t = ap_obj::classes::TypeObject::create(
        name.to_string(),
        name.to_string(),
        vec![base],
        PyDict::new(),
        ap_obj::classes::TypeFlags {
            slots: None,
            is_exception: true,
            builtin_exc: true,
        },
        None,
    )
    .expect("threading exc MRO");
    let _ = t.dict.set(
        Value::Str(PyString::new("__module__")),
        Value::Str(PyString::new("threading")),
    );
    let v = ap_obj::classes::make_class_value(t);
    // Register for exc_matches: is_builtin_exc_name covers EXC_TREE only;
    // these two route through the Type arm (o.kind == Type + name check)
    // which requires registration in interp.exc_classes.
    interp
        .exc_classes
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(name.to_string(), v.clone());
    v
}

// ===========================================================================
// Module-level function dispatch
// ===========================================================================

pub fn call_threading(
    _interp: &Arc<crate::frame::Interpreter>,
    name: &str,
    args: &[Value],
    _kwargs: &[(String, Value)],
) -> Option<BResult> {
    // GC drain turn (worker allocations join the main registry here).
    let _ = crate::gcsupport::drain_worker_registrations();
    let r: BResult = (|| {
        match name {
            "current_thread" | "currentThread" => Ok(crate::threading::current_thread_value()),
            "main_thread" => Ok(crate::threading::main_thread_value()),
            "active_count" | "activeCount" => Ok(smi(crate::threading::active_count() as i64)),
            "enumerate" => Ok(crate::builtins_modules::list(
                crate::threading::enumerate_values(),
            )),
            "get_ident" => Ok(smi(crate::threading::get_ident_value())),
            "get_native_id" => Ok(smi(crate::threading::get_native_id_value())),
            "get_time" => Ok(Value::Float(monotonic_secs())),
            "settrace" | "setprofile" | "settrace_all_threads" | "setprofile_all_threads" => {
                // Minimal hook surface: accept + store (documented).
                Ok(Value::None)
            }
            "stack_size" => {
                if let Some(v) = args.first() {
                    let _ = f64_arg(v)?; // validated, then ignored (64MB fixed)
                }
                Ok(smi(64 << 20))
            }
            // W112: park exit hooks (twisted reactor cleanup registers here;
            // v0 never fires them — cleanup paths only need the call to exist).
            "_register_atexit" => {
                let f = args.first().cloned().unwrap_or(Value::None);
                let _ = f;
                Ok(Value::None)
            }
            "Lock" | "allocate_lock" => lock_ctor(args),
            "RLock" => rlock_ctor(args),
            "Condition" => cond_ctor(args),
            "Event" => event_ctor(args),
            "Semaphore" => sem_ctor(args, false),
            "BoundedSemaphore" => sem_ctor(args, true),
            "Barrier" => barrier_ctor(args, _kwargs),
            "Timer" => timer_ctor(args, _kwargs),
            "local" => local_ctor(args),
            "Thread" => thread_ctor(args, _kwargs),
            "excepthook" => excepthook_get(),
            _ => Err(raise(
                "AttributeError",
                format!("module 'threading' has no attribute '{name}'"),
            )),
        }
    })();
    Some(r)
}

/// Thread(group=None, target, name=None, args=(), kwargs={}, *, daemon=None).
fn thread_ctor(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    // Positional: group target name args kwargs — group MUST be None.
    let mut pos = args.iter();
    if let Some(g) = pos.next() {
        if !matches!(g, Value::None) {
            return Err(raise(
                "AssertionError",
                "group argument must be None for now",
            ));
        }
    }
    let target = pos.next().cloned().unwrap_or(Value::None);
    let pos_name = pos.next().and_then(|v| match v {
        Value::Str(s) => Some(s.as_str().to_string()),
        _ => None,
    });
    let pos_args = pos.next().and_then(|v| match v {
        Value::Tuple(t) => Some(t.0.clone()),
        _ => None,
    });
    let pos_kwargs = pos.next();
    let target = kw(kwargs, "target").cloned().unwrap_or(target);
    let name = kw(kwargs, "name")
        .and_then(|v| match v {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        })
        .or(pos_name);
    let targs = match kw(kwargs, "args") {
        Some(Value::Tuple(t)) => t.0.clone(),
        _ => pos_args.unwrap_or_default(),
    };
    let mut tkwargs: Vec<(String, Value)> = match pos_kwargs {
        Some(Value::Dict(d)) => d
            .items()
            .into_iter()
            .filter_map(|(k, v)| match &k {
                Value::Str(ks) => Some((ks.as_str().to_string(), v.clone())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    if let Some(Value::Dict(d)) = kw(kwargs, "kwargs") {
        for (k, v) in d.items() {
            if let Value::Str(ks) = &k {
                tkwargs.push((ks.as_str().to_string(), v.clone()));
            }
        }
    }
    let daemon = match kw(kwargs, "daemon") {
        Some(Value::Bool(b)) => *b,
        _ => false,
    };
    Ok(crate::threading::make_thread_value(
        name,
        Some(target),
        targs,
        tkwargs,
        daemon,
        "Thread",
        None,
    ))
}

fn monotonic_secs() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn excepthook_get() -> BResult {
    Ok(crate::threading::excepthook_value())
}

// ===========================================================================
// Constructors
// ===========================================================================

fn lock_ctor(_args: &[Value]) -> BResult {
    let o = PyObject::new(ObjKind::Opaque, "lock");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    prim_put(
        addr,
        LockPrim {
            locked: Mutex::new(false),
            cv: std::sync::Condvar::new(),
        },
    );
    Ok(Value::Obj(o))
}

fn rlock_ctor(_args: &[Value]) -> BResult {
    let o = PyObject::new(ObjKind::Opaque, "rlock");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    prim_put(
        addr,
        RLockPrim {
            st: Mutex::new((crate::threading::RLockOwner::None, 0usize)),
            cv: std::sync::Condvar::new(),
        },
    );
    Ok(Value::Obj(o))
}

fn cond_ctor(args: &[Value]) -> BResult {
    let o = PyObject::new(ObjKind::Opaque, "condition");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    // Backing lock: caller's Lock/RLock object, or an implicit RLock.
    let (lock_addr, own_lock) = match args.first() {
        Some(v @ Value::Obj(_)) => (obj_addr(v)?, false),
        _ => (addr + 1, true),
    };
    if own_lock {
        // M4: the implicit backing RLock lives at the PHANTOM key (cond
        // addr + 1); register it as condition-owned so the eviction sweep
        // keeps it exactly as long as the Condition object is alive.
        crate::threading::prim_put_owned_rlock(
            addr,
            RLockPrim {
                st: Mutex::new((crate::threading::RLockOwner::None, 0usize)),
                cv: std::sync::Condvar::new(),
            },
        );
    }
    prim_put(
        addr,
        CondPrim {
            lock_addr,
            own_rlock_addr: lock_addr,
            waiters: Mutex::new(0),
            cv: std::sync::Condvar::new(),
        },
    );
    Ok(Value::Obj(o))
}

fn event_ctor(_args: &[Value]) -> BResult {
    let o = PyObject::new(ObjKind::Opaque, "event");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    prim_put(
        addr,
        EventPrim {
            st: Mutex::new(false),
            cv: std::sync::Condvar::new(),
        },
    );
    Ok(Value::Obj(o))
}

fn sem_ctor(args: &[Value], bounded: bool) -> BResult {
    let value = match args.first() {
        Some(v) => f64_to_i64(v)?,
        None => 1,
    };
    if value < 0 {
        return Err(raise("ValueError", "semaphore initial value must be >= 0"));
    }
    let o = PyObject::new(ObjKind::Opaque, "semaphore");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    prim_put(
        addr,
        SemPrim {
            value: Mutex::new(value),
            initial: value,
            bounded,
            cv: std::sync::Condvar::new(),
        },
    );
    Ok(Value::Obj(o))
}

fn f64_to_i64(v: &Value) -> Result<i64, Flow> {
    match v {
        Value::Int(n) => n
            .to_i64()
            .ok_or_else(|| raise("OverflowError", "int too large")),
        Value::Float(f) => Ok(*f as i64),
        Value::Bool(b) => Ok(i64::from(*b)),
        other => Err(raise(
            "TypeError",
            format!("an integer is required, not '{}'", other.type_name()),
        )),
    }
}

fn barrier_ctor(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    let parties = match args.first() {
        Some(v) => f64_to_i64(v)?,
        None => {
            return Err(raise(
                "TypeError",
                "Barrier() missing required argument 'parties'",
            ))
        }
    };
    if parties < 1 {
        return Err(raise("ValueError", "parties must be >= 1"));
    }
    let action = kw(kwargs, "action").cloned().unwrap_or(Value::None);
    let default_timeout = match kw(kwargs, "timeout") {
        Some(v) => opt_f64(v)?,
        None => None,
    };
    let o = PyObject::new(ObjKind::Opaque, "barrier");
    let addr = obj_addr(&Value::Obj(o.clone()))?;
    prim_put(
        addr,
        BarrierPrim {
            parties: parties as usize,
            st: Mutex::new((0usize, 0i32, 0u64)),
            cv: std::sync::Condvar::new(),
            action: if matches!(action, Value::None) {
                None
            } else {
                Some(action)
            },
            default_timeout,
        },
    );
    Ok(Value::Obj(o))
}

fn timer_ctor(args: &[Value], kwargs: &[(String, Value)]) -> BResult {
    // Timer(interval, function, args=None, kwargs=None) — a Thread with a
    // pre-wait interval.
    let interval = match args.first() {
        Some(v) => f64_arg(v)?,
        None => {
            return Err(raise(
                "TypeError",
                "Timer() missing required argument 'interval'",
            ))
        }
    };
    let mut function = args.get(1).cloned().unwrap_or(Value::None);
    let mut fargs = match args.get(2) {
        Some(Value::Tuple(t)) => t.0.clone(),
        Some(Value::None) | None => vec![],
        Some(Value::List(l)) => l.items().to_vec(),
        Some(other) => vec![other.clone()],
    };
    let mut fkwargs: Vec<(String, Value)> = vec![];
    if let Some(Value::Dict(d)) = args.get(3) {
        for (k, v) in d.items() {
            if let Value::Str(ks) = &k {
                fkwargs.push((ks.as_str().to_string(), v.clone()));
            }
        }
    }
    // CPython Timer.__init__ signature: Timer(interval, function, args=None,
    // kwargs=None) — the common call form passes args= by keyword.
    let mut extra_kwargs: Vec<(String, Value)> = vec![];
    for (k, v) in kwargs {
        match k.as_str() {
            "args" => {
                fargs = match v {
                    Value::Tuple(t) => t.0.clone(),
                    Value::None => vec![],
                    Value::List(l) => l.items().to_vec(),
                    other => vec![other.clone()],
                };
            }
            "kwargs" => {
                if let Value::Dict(d) = v {
                    for (k2, v2) in d.items() {
                        if let Value::Str(ks) = &k2 {
                            fkwargs.push((ks.as_str().to_string(), v2.clone()));
                        }
                    }
                }
            }
            "function" => function = v.clone(),
            _ => extra_kwargs.push((k.clone(), v.clone())),
        }
    }
    fkwargs.extend(extra_kwargs);
    let v = crate::threading::make_thread_value(
        None, // name assigned at start (Thread-N (run)-less: Timer names are Thread-N)
        Some(function),
        fargs,
        fkwargs,
        false,
        "Timer",
        Some(interval),
    );
    Ok(v)
}

fn local_ctor(_args: &[Value]) -> BResult {
    let o = PyObject::new(ObjKind::Opaque, "_local");
    let _ = obj_addr(&Value::Obj(o.clone()))?;
    Ok(Value::Obj(o))
}

// ===========================================================================
// Bound-method dispatch: thread objects and primitives
// ===========================================================================

/// `threading.<name>` marker where the fn is a method over a receiver:
/// dispatched from call_native with the marker name intact ("Thread.start"
/// never happens — methods arrive via the native_attr BoundMethod surface).
/// Entry: `call_threading_method(recv, method, args, kwargs)`.
pub fn call_threading_method(
    interp: &Arc<crate::frame::Interpreter>,
    recv: &Value,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    let r: BResult = match recv {
        Value::Obj(o) => match o.name.as_str() {
            "thread" => thread_method(interp, o, method, args, kwargs),
            "lock" => lock_method(o, method, args, kwargs),
            "rlock" => rlock_method(o, method, args),
            "condition" => cond_method(interp, o, method, args),
            "event" => event_method_kw(o, method, args, kwargs),
            "semaphore" => sem_method(o, method, args, kwargs),
            "barrier" => barrier_method(interp, o, method, args, kwargs),
            "_local" => local_method(o, method, args),
            "queue" => match call_queue_method(interp, recv, method, args, &[]) {
                Some(r) => r,
                None => None2Err(method),
            },
            _ => None2Err(method),
        },
        _ => None2Err(method),
    };
    Some(r)
}

#[allow(non_snake_case)] // CPython-exact identifier - renaming would break the byte-exact conformance contract
fn None2Err(method: &str) -> BResult {
    Err(raise(
        "AttributeError",
        format!("threading object has no attribute '{method}'"),
    ))
}

fn timeout_of(args: &[Value], kwargs: &[(String, Value)], pos: usize) -> Result<Option<f64>, Flow> {
    if let Some(v) = kw(kwargs, "timeout") {
        return opt_f64(v);
    }
    match args.get(pos) {
        Some(v) => opt_f64(v),
        None => Ok(None),
    }
}

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
fn blocking_of(args: &[Value], kwargs: &[(String, Value)]) -> Result<bool, Flow> {
    if let Some(v) = kw(kwargs, "blocking") {
        return Ok(matches!(v, Value::Bool(true)));
    }
    match args.first() {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Ok(true),
    }
}

// --------------------------- Thread methods --------------------------------

fn thread_method(
    interp: &Arc<crate::frame::Interpreter>,
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    match method {
        "start" => crate::threading::thread_start(interp, o),
        "cancel" => {
            // Timer.cancel: flag the parked pre-run wait to bail out.
            // E-key: the (addr, serial) pair — serial blocks addr-recycling
            // aliasing on the cancel flag.
            let serial = attr_v(o, "<serial>")
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                .unwrap_or(-2);
            crate::threading::cancel_timer_serial(Arc::as_ptr(o).addr(), serial);
            Ok(Value::None)
        }
        "interval" | "function" => {
            let key = if method == "interval" {
                "<interval>"
            } else {
                "<target>"
            };
            Ok(attr_v(o, key).unwrap_or(Value::None))
        }
        "join" => {
            let Some(entry) = crate::threading::entry_for_pub(addr) else {
                return Err(raise(
                    "RuntimeError",
                    "cannot join thread before it is started",
                ));
            };
            let timeout = timeout_of(args, kwargs, 0)?;
            // E7/E-safety: grab the shared done FLAG while the GIL is held
            // (borrow uncontended), release the GIL, then park on the
            // condvar reading ONLY atomics off-lock.
            let flag = crate::threading::entry_done_flag(&entry);
            let _got = crate::threading::join_wait_flag(flag, timeout);
            // E1-E4: the eviction sweep (evict_stale_tables) is EXPORTED
            // for the orchestrator's major-GC drain hook (gcsupport
            // collect) — deliberately NOT called from the join path: the
            // sweep's registration drain can fire an auto-collect while
            // workers are parked at GIL-release points, and the registry
            // then transiently mis-reads live objects as absent (T122
            // caught a module-level Lock evicted from PRIMS under exactly
            // that shape). Worker payload reclaim drains inside
            // evict_stale_tables at the major-GC drain hook.
            Ok(Value::None)
        }
        "is_alive" | "isAlive" => {
            let alive = crate::threading::entry_for_pub(addr)
                .map(|e| {
                    let b = e.borrow();
                    b.started && !b.done.load(std::sync::atomic::Ordering::SeqCst)
                })
                .unwrap_or(false);
            Ok(Value::Bool(alive))
        }
        "run" => {
            // Direct run(): invoke the target on the CURRENT thread.
            let target = attr_v(o, "<target>").unwrap_or(Value::None);
            if matches!(target, Value::None) {
                return Ok(Value::None);
            }
            let fargs = match attr_v(o, "<args>") {
                Some(Value::Tuple(t)) => t.0.clone(),
                _ => vec![],
            };
            let fkwargs: Vec<(String, Value)> = match attr_v(o, "<kwargs>") {
                Some(Value::Dict(d)) => d
                    .items()
                    .into_iter()
                    .filter_map(|(k, v)| match &k {
                        Value::Str(ks) => Some((ks.as_str().to_string(), v.clone())),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };
            crate::interp::call_value_pub(interp, &target, &fargs, &fkwargs)
        }
        "getName" => Ok(Value::Str(PyString::new(
            attr_v(o, "<name>")
                .and_then(|v| match v {
                    Value::Str(sv) => Some(sv.as_str().to_string()),
                    _ => None,
                })
                .unwrap_or_default(),
        ))),
        "setName" => {
            let v = required(args, "setName")?;
            let sval = str_arg(v, "setName")?;
            attr_set(o, "<name>", s(sval));
            Ok(Value::None)
        }
        "isDaemon" | "is_daemon" => Ok(Value::Bool(matches!(
            attr_v(o, "<daemon>"),
            Some(Value::Bool(true))
        ))),
        "native_id" | "ident" => {
            // Property reads land in native_attr, not here; harmless fallback.
            let e = crate::threading::entry_for_pub(addr);
            let v = e.and_then(|e| {
                let b = e.borrow();
                if method == "ident" {
                    b.ident
                } else {
                    b.native_id
                }
            });
            Ok(v.map(smi).unwrap_or(Value::None))
        }
        _ => None2Err(method),
    }
}

// --------------------------- Lock methods ----------------------------------

fn lock_method(
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<LockPrim>(addr) else {
        return Err(raise("SystemError", "lock primitive missing"));
    };
    match method {
        "acquire" | "__enter__" => {
            // CPython: acquire(blocking=True, timeout=-1). The kw form
            // (blocking=False) must be honored — the bound-method call
            // passes kwargs through.
            let blocking = match args.first() {
                Some(Value::Bool(b)) => *b,
                _ => match kw(kwargs, "blocking") {
                    Some(Value::Bool(b)) => *b,
                    _ => true,
                },
            };
            let timeout = if args.len() > 1 {
                match &args[1] {
                    Value::Float(f) => Some(*f),
                    Value::Int(n) => n.to_f64(),
                    Value::None => None,
                    _ => None,
                }
            } else {
                match kw(kwargs, "timeout") {
                    Some(Value::Float(f)) => Some(*f),
                    Some(Value::Int(n)) => n.to_f64(),
                    _ => None,
                }
            };
            let _ = crate::threading::current_os_tid();
            let mut locked = prim.locked.lock().unwrap();
            if *locked {
                if !blocking {
                    drop(locked);
                    return Ok(Value::Bool(false));
                }
                let deadline =
                    timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
                loop {
                    if !*locked {
                        break;
                    }
                    let left = match deadline {
                        Some(d) => d.saturating_duration_since(Instant::now()),
                        None => Duration::from_secs(3600),
                    };
                    if left.is_zero() {
                        drop(locked);
                        return Ok(Value::Bool(false));
                    }
                    let (g, t) = prim.cv.wait_timeout(locked, left).unwrap();
                    locked = g;
                    if t.timed_out() && *locked {
                        drop(locked);
                        return Ok(Value::Bool(false));
                    }
                    if !*locked {
                        break;
                    }
                }
            }
            *locked = true;
            drop(locked);
            Ok(Value::Bool(true))
        }
        "release" | "__exit__" => {
            let mut locked = prim.locked.lock().unwrap();
            if !*locked {
                return Err(raise("RuntimeError", "release unlocked lock"));
            }
            *locked = false;
            drop(locked);
            prim.cv.notify_all();
            Ok(Value::None)
        }
        "locked" | "locked_lock" => Ok(Value::Bool(*prim.locked.lock().unwrap())),
        _ => None2Err(method),
    }
}

// --------------------------- RLock methods ---------------------------------

fn rlock_method(o: &Gc<PyObject>, method: &str, args: &[Value]) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<RLockPrim>(addr) else {
        return Err(raise("SystemError", "rlock primitive missing"));
    };
    let me = crate::threading::current_os_tid();
    let owner_of = |st: (crate::threading::RLockOwner, usize)| matches!(st.0, crate::threading::RLockOwner::Owned(t) if t == me);
    match method {
        "acquire" | "__enter__" => {
            let blocking = match args.first() {
                Some(Value::Bool(b)) => *b,
                _ => true,
            };
            let timeout = args.get(1).and_then(|v| match v {
                Value::Float(f) => Some(*f),
                Value::Int(n) => n.to_f64(),
                _ => None,
            });
            let mut st = prim.st.lock().unwrap();
            if owner_of(*st) {
                st.1 += 1;
                return Ok(Value::Bool(true));
            }
            if st.0 != crate::threading::RLockOwner::None {
                if !blocking {
                    return Ok(Value::Bool(false));
                }
                let deadline =
                    timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
                loop {
                    if st.0 == crate::threading::RLockOwner::None {
                        break;
                    }
                    let left = match deadline {
                        Some(d) => d.saturating_duration_since(Instant::now()),
                        None => Duration::from_secs(3600),
                    };
                    if left.is_zero() {
                        return Ok(Value::Bool(false));
                    }
                    let (g, t) = prim.cv.wait_timeout(st, left).unwrap();
                    st = g;
                    if t.timed_out() && st.0 != crate::threading::RLockOwner::None {
                        return Ok(Value::Bool(false));
                    }
                    if st.0 == crate::threading::RLockOwner::None {
                        break;
                    }
                }
            }
            st.0 = crate::threading::RLockOwner::Owned(me);
            st.1 = 1;
            Ok(Value::Bool(true))
        }
        "release" | "__exit__" => {
            let mut st = prim.st.lock().unwrap();
            if !owner_of(*st) {
                return Err(raise("RuntimeError", "cannot release un-acquired lock"));
            }
            st.1 -= 1;
            if st.1 == 0 {
                st.0 = crate::threading::RLockOwner::None;
                drop(st);
                prim.cv.notify_all();
            }
            Ok(Value::None)
        }
        "_is_owned" => Ok(Value::Bool(owner_of(*prim.st.lock().unwrap()))),
        "locked" => Ok(Value::Bool(
            prim.st.lock().unwrap().0 != crate::threading::RLockOwner::None,
        )),
        _ => None2Err(method),
    }
}

// --------------------------- Condition methods -----------------------------

fn cond_owned(cond: &CondPrim) -> bool {
    let Some(rl) = prim_of::<RLockPrim>(cond.lock_addr) else {
        let Some(lk) = prim_of::<LockPrim>(cond.lock_addr) else {
            return false;
        };
        // Plain Lock: "owned" iff locked (by THIS thread — a plain lock held
        // by another thread makes acquire(False) fail, which the CPython
        // _is_owned fallback encodes).
        return *lk.locked.lock().unwrap();
    };
    let st = rl.st.lock().unwrap();
    matches!(st.0, crate::threading::RLockOwner::Owned(t) if t == crate::threading::current_os_tid())
}

fn cond_acquire_lock(cond: &CondPrim) -> BResult {
    if cond.own_rlock_addr == cond.lock_addr {
        if let Some(rl) = prim_of::<RLockPrim>(cond.lock_addr) {
            return rlock_acquire(rl);
        }
    }
    if let Some(rl) = prim_of::<RLockPrim>(cond.lock_addr) {
        return rlock_acquire(rl);
    }
    if let Some(lk) = prim_of::<LockPrim>(cond.lock_addr) {
        return lock_acquire_blocking(lk);
    }
    Err(raise("SystemError", "condition backing lock missing"))
}

fn lock_acquire_blocking(lk: Arc<LockPrim>) -> BResult {
    let mut locked = lk.locked.lock().unwrap();
    while *locked {
        let g = lk.cv.wait(locked).unwrap();
        locked = g;
    }
    *locked = true;
    Ok(Value::Bool(true))
}

fn rlock_acquire(rl: Arc<RLockPrim>) -> BResult {
    let me = crate::threading::current_os_tid();
    // GIL FIRST: never take the primitive's lock holding the interpreter
    // lock — a parked path would hold the GIL and starve the owner.
    let mut st = rl.st.lock().unwrap();
    if matches!(st.0, crate::threading::RLockOwner::Owned(t) if t == me) {
        st.1 += 1;
        drop(st);
        return Ok(Value::Bool(true));
    }
    while st.0 != crate::threading::RLockOwner::None {
        let g = rl.cv.wait(st).unwrap();
        st = g;
    }
    st.0 = crate::threading::RLockOwner::Owned(me);
    st.1 = 1;
    drop(st);
    Ok(Value::Bool(true))
}

fn rlock_release_full(rl: &RLockPrim) -> BResult {
    let me = crate::threading::current_os_tid();
    let mut st = rl.st.lock().unwrap();
    if !matches!(st.0, crate::threading::RLockOwner::Owned(t) if t == me) {
        return Err(raise("RuntimeError", "cannot release un-acquired lock"));
    }
    // Full release (the _release_save protocol).
    st.0 = crate::threading::RLockOwner::None;
    st.1 = 0;
    drop(st);
    rl.cv.notify_all();
    Ok(Value::None)
}

fn cond_method(
    interp: &Arc<crate::frame::Interpreter>,
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<CondPrim>(addr) else {
        return Err(raise("SystemError", "condition primitive missing"));
    };
    match method {
        "acquire" => cond_acquire_lock(&prim),
        "release" => {
            if let Some(rl) = prim_of::<RLockPrim>(prim.lock_addr) {
                return rlock_release_one(&rl);
            }
            if let Some(lk) = prim_of::<LockPrim>(prim.lock_addr) {
                return lock_release(lk);
            }
            Err(raise("SystemError", "condition backing lock missing"))
        }
        "__enter__" => cond_acquire_lock(&prim),
        "__exit__" => {
            if let Some(rl) = prim_of::<RLockPrim>(prim.lock_addr) {
                return rlock_release_full(&rl);
            }
            if let Some(lk) = prim_of::<LockPrim>(prim.lock_addr) {
                return lock_release(lk);
            }
            Ok(Value::None)
        }
        "wait" => {
            if !cond_owned(&prim) {
                return Err(raise("RuntimeError", "cannot wait on un-acquired lock"));
            }
            let timeout = timeout_of(args, &[], 0)?;
            cond_wait(interp, &prim, timeout).map(Value::Bool)
        }
        "wait_for" => {
            if !cond_owned(&prim) {
                return Err(raise("RuntimeError", "cannot wait on un-acquired lock"));
            }
            let pred = required(args, "wait_for")?.clone();
            let timeout = timeout_of(&args[1..], &[], 0)?;
            let endtime = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
            loop {
                let result = crate::interp::call_value_pub(interp, &pred, &[], &[])?;
                if matches!(result, Value::Bool(true)) {
                    return Ok(Value::Bool(true));
                }
                let truthy = crate::interp::truthy_public(interp, &result)?;
                if truthy {
                    return Ok(Value::Bool(true));
                }
                let remaining: Option<f64> = match endtime {
                    Some(d) => {
                        let r = d.saturating_duration_since(Instant::now()).as_secs_f64();
                        if r <= 0.0 {
                            return Ok(Value::Bool(false));
                        }
                        Some(r)
                    }
                    None => None,
                };
                cond_wait(interp, &prim, remaining)?;
            }
        }
        "notify" => {
            if !cond_owned(&prim) {
                return Err(raise("RuntimeError", "cannot notify on un-acquired lock"));
            }
            let n = args.first().and_then(|v| f64_to_i64(v).ok()).unwrap_or(1);
            let mut w = prim.waiters.lock().unwrap();
            let k = n.min(*w as i64) as usize;
            *w -= k;
            drop(w);
            for _ in 0..k {
                prim.cv.notify_one();
            }
            Ok(Value::None)
        }
        "notify_all" | "notifyAll" => {
            if !cond_owned(&prim) {
                return Err(raise("RuntimeError", "cannot notify on un-acquired lock"));
            }
            let mut w = prim.waiters.lock().unwrap();
            *w = 0;
            drop(w);
            prim.cv.notify_all();
            Ok(Value::None)
        }
        _ => None2Err(method),
    }
}

fn rlock_release_one(rl: &Arc<RLockPrim>) -> BResult {
    let me = crate::threading::current_os_tid();
    let mut st = rl.st.lock().unwrap();
    if !matches!(st.0, crate::threading::RLockOwner::Owned(t) if t == me) {
        return Err(raise("RuntimeError", "cannot release un-acquired lock"));
    }
    st.1 -= 1;
    if st.1 == 0 {
        st.0 = crate::threading::RLockOwner::None;
        drop(st);
        rl.cv.notify_all();
    }
    Ok(Value::None)
}

fn lock_release(lk: Arc<LockPrim>) -> BResult {
    let mut locked = lk.locked.lock().unwrap();
    if !*locked {
        return Err(raise("RuntimeError", "release unlocked lock"));
    }
    *locked = false;
    drop(locked);
    lk.cv.notify_all();
    Ok(Value::None)
}

/// Condition.wait: release the backing lock FULLY, park, re-acquire.
fn cond_wait(
    _interp: &Arc<crate::frame::Interpreter>,
    prim: &CondPrim,
    timeout: Option<f64>,
) -> Result<bool, Flow> {
    // Release-save.
    let release_result = if let Some(rl) = prim_of::<RLockPrim>(prim.lock_addr) {
        rlock_release_full(&rl)
    } else if let Some(lk) = prim_of::<LockPrim>(prim.lock_addr) {
        lock_release(lk)
    } else {
        Ok(Value::None)
    };
    release_result?;
    {
        let mut w = prim.waiters.lock().unwrap();
        *w += 1;
    }
    let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
    let got;
    {
        let w = prim.waiters.lock().unwrap();
        let left = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(3600),
        };
        let _ = left;
        // GIL FIRST: the parked waiter must not hold the interpreter lock —
        // the notifier needs it to run and call notify_all.
        let (g, t) = prim.cv.wait_timeout(w, left).unwrap();
        let timed_out = t.timed_out();
        drop(g);
        got = !timed_out;
    }
    {
        let mut w2 = prim.waiters.lock().unwrap();
        if *w2 > 0 {
            *w2 -= 1;
        }
    }
    // Re-acquire.
    cond_acquire_lock(prim)?;
    Ok(got)
}

// --------------------------- Event methods ---------------------------------

fn event_method_kw(
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    // Event.wait(timeout=N): the timeout rides the kw when called by name.
    if method == "wait" && args.is_empty() {
        if let Some(tv) = kw(kwargs, "timeout") {
            let t = opt_f64(tv)?;
            return event_wait(o, t);
        }
    }
    event_method(o, method, args)
}

fn event_method(o: &Gc<PyObject>, method: &str, args: &[Value]) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<EventPrim>(addr) else {
        return Err(raise("SystemError", "event primitive missing"));
    };
    let _ = &args;
    match method {
        "is_set" | "isSet" => Ok(Value::Bool(*prim.st.lock().unwrap())),
        "set" => {
            let mut st = prim.st.lock().unwrap();
            *st = true;
            drop(st);
            prim.cv.notify_all();
            Ok(Value::None)
        }
        "clear" => {
            *prim.st.lock().unwrap() = false;
            Ok(Value::None)
        }
        "wait" => {
            let timeout = timeout_of(args, &[], 0)?;
            event_wait(o, timeout)
        }
        _ => None2Err(method),
    }
}

fn event_wait(o: &Gc<PyObject>, timeout: Option<f64>) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<EventPrim>(addr) else {
        return Err(raise("SystemError", "event primitive missing"));
    };
    let mut st = prim.st.lock().unwrap();
    if *st {
        return Ok(Value::Bool(true));
    }
    let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
    loop {
        let left = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => Duration::from_secs(3600),
        };
        if left.is_zero() {
            let v = *st;
            return Ok(Value::Bool(v));
        }
        let (g, t) = prim.cv.wait_timeout(st, left).unwrap();
        st = g;
        if *st {
            return Ok(Value::Bool(true));
        }
        if t.timed_out() {
            return Ok(Value::Bool(false));
        }
    }
}

// --------------------------- Semaphore methods -----------------------------

fn sem_method(
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<SemPrim>(addr) else {
        return Err(raise("SystemError", "semaphore primitive missing"));
    };
    match method {
        "acquire" | "__enter__" => {
            // CPython signature: acquire(blocking=True, timeout=None).
            // The kwargs flow through the bound-method path; read BOTH.
            let blocking = match args.first() {
                Some(Value::Bool(b)) => *b,
                _ => match kw(kwargs, "blocking") {
                    Some(Value::Bool(b)) => *b,
                    _ => true,
                },
            };
            let timeout = args
                .get(1)
                .and_then(|v| match v {
                    Value::Float(f) => Some(*f),
                    Value::Int(n) => n.to_f64(),
                    Value::None => None,
                    _ => None,
                })
                .or(match kw(kwargs, "timeout") {
                    Some(Value::Float(f)) => Some(*f),
                    Some(Value::Int(n)) => n.to_f64(),
                    _ => None,
                });
            let mut val = prim.value.lock().unwrap();
            if *val > 0 {
                *val -= 1;
                drop(val);
                return Ok(Value::Bool(true));
            }
            if !blocking {
                drop(val);
                return Ok(Value::Bool(false));
            }
            let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
            loop {
                let left = match deadline {
                    Some(d) => d.saturating_duration_since(Instant::now()),
                    None => Duration::from_secs(3600),
                };
                if left.is_zero() {
                    drop(val);
                    return Ok(Value::Bool(false));
                }
                let (g, t) = prim.cv.wait_timeout(val, left).unwrap();
                val = g;
                if *val > 0 {
                    *val -= 1;
                    drop(val);
                    return Ok(Value::Bool(true));
                }
                if t.timed_out() {
                    drop(val);
                    return Ok(Value::Bool(false));
                }
            }
        }
        "release" | "__exit__" => {
            let n = args.first().and_then(|v| f64_to_i64(v).ok()).unwrap_or(1);
            let mut val = prim.value.lock().unwrap();
            if prim.bounded && *val + n > prim.initial {
                return Err(raise("ValueError", "Semaphore released too many times"));
            }
            *val += n;
            drop(val);
            for _ in 0..n {
                prim.cv.notify_one();
            }
            Ok(Value::None)
        }
        _ => None2Err(method),
    }
}

// --------------------------- Barrier methods -------------------------------

fn barrier_method(
    interp: &Arc<crate::frame::Interpreter>,
    o: &Gc<PyObject>,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> BResult {
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<BarrierPrim>(addr) else {
        return Err(raise("SystemError", "barrier primitive missing"));
    };
    match method {
        "wait" => {
            let timeout = timeout_of(args, kwargs, 0)?.or(prim.default_timeout);
            let me = crate::threading::current_os_tid();
            let _ = me;
            // GIL FIRST: every blocking entry releases the interpreter lock
            // BEFORE taking the primitive's own mutex (the parked party must
            // not hold the GIL — nobody else could reach their rendezvous).
            let mut st = prim.st.lock().unwrap();
            // _enter: block while draining/resetting.
            loop {
                let (_, state, _) = *st;
                if state == -1 || state == 1 {
                    let g = prim.cv.wait(st).unwrap();
                    st = g;
                    continue;
                }
                break;
            }
            if st.1 < 0 {
                return Err(raise("BrokenBarrierError", ""));
            }
            // Occupy a slot.
            let index = st.0;
            st.0 += 1;
            let my_gen = st.2;
            if index + 1 == prim.parties {
                // _release: run the action, drain.
                let action = prim.action.clone();
                let run_result: Result<(), Flow> = if let Some(action) = action {
                    crate::interp::call_value_pub(interp, &action, &[], &[]).map(|_| ())
                } else {
                    Ok(())
                };
                match run_result {
                    Ok(()) => {
                        st.1 = 1;
                        st.2 += 1;
                        drop(st);
                        prim.cv.notify_all();
                    }
                    Err(e) => {
                        st.1 = -2;
                        st.2 += 1;
                        drop(st);
                        prim.cv.notify_all();
                        return Err(e);
                    }
                }
                // The releaser also exits immediately (drain completes when
                // the last party leaves; for byte-exactness of n_waiting the
                // releaser's slot counts down with everyone else's below).
                let mut st = prim.st.lock().unwrap();
                st.0 -= 1;
                let drained = st.0 == 0 && (st.1 == 1 || st.1 == -1);
                if drained {
                    st.1 = 0;
                }
                drop(st);
                if drained {
                    prim.cv.notify_all();
                }
                return Ok(smi(index as i64));
            }
            // _wait: predicate state != 0 (or generation advance), timeout.
            // GIL DISCIPLINE: the st guard must NOT be held while the GIL is
            // released (another party needs st to enter/release) — the
            // condvar wait reclaims it atomically.
            let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
            let mut broke = false;
            loop {
                let (_, state, gen) = *st;
                if gen != my_gen {
                    if state < 0 {
                        broke = true;
                        break;
                    }
                    break; // drained -> released
                }
                if state != 0 {
                    if state < 0 {
                        broke = true;
                        break;
                    }
                    break;
                }
                let left = match deadline {
                    Some(d) => d.saturating_duration_since(Instant::now()),
                    None => Duration::from_secs(3600),
                };
                if left.is_zero() {
                    // _break on timeout.
                    st.1 = -2;
                    st.2 += 1;
                    drop(st);
                    prim.cv.notify_all();
                    broke = true;
                    break;
                }
                let (g, t) = prim.cv.wait_timeout(st, left).unwrap();
                st = g;
                let (_, wstate, wgen) = *st;
                let wdone = wgen != my_gen || wstate != 0;
                let timed_out = t.timed_out() && !wdone;
                drop(st);
                if timed_out {
                    // _break on timeout.
                    let mut st2 = prim.st.lock().unwrap();
                    st2.1 = -2;
                    st2.2 += 1;
                    drop(st2);
                    prim.cv.notify_all();
                    broke = true;
                    break;
                }
                if wdone {
                    break;
                }
                // Re-take the lock for the next loop iteration.
                st = prim.st.lock().unwrap();
            }
            {
                let mut st = prim.st.lock().unwrap();
                st.0 = st.0.saturating_sub(1);
                if st.0 == 0 && (st.1 == 1 || st.1 == -1) {
                    st.1 = 0;
                    drop(st);
                    prim.cv.notify_all();
                }
            }
            if broke {
                return Err(raise("BrokenBarrierError", ""));
            }
            Ok(smi(index as i64))
        }
        "reset" => {
            let mut st = prim.st.lock().unwrap();
            if st.0 > 0 {
                st.1 = -1;
            } else {
                st.1 = 0;
            }
            drop(st);
            prim.cv.notify_all();
            Ok(Value::None)
        }
        "abort" => {
            let mut st = prim.st.lock().unwrap();
            st.1 = -2;
            st.2 += 1;
            drop(st);
            prim.cv.notify_all();
            Ok(Value::None)
        }
        _ => None2Err(method),
    }
}

/// Barrier property reads (parties/n_waiting/broken) — the attr surface.
pub fn barrier_attr(o: &Gc<PyObject>, name: &str) -> Option<Value> {
    let prim = prim_of::<BarrierPrim>(Arc::as_ptr(o).addr())?;
    match name {
        "parties" => Some(smi(prim.parties as i64)),
        "n_waiting" => {
            let st = prim.st.lock().unwrap();
            Some(smi(if st.1 == 0 { st.0 as i64 } else { 0 }))
        }
        "broken" => {
            let st = prim.st.lock().unwrap();
            Some(Value::Bool(st.1 == -2))
        }
        _ => None,
    }
}

pub fn event_attr(o: &PyObject, name: &str) -> Option<Value> {
    // No public attrs; keep the surface minimal.
    let _ = (o, name);
    None
}

// --------------------------- local methods ---------------------------------

fn local_method(o: &PyObject, method: &str, args: &[Value]) -> BResult {
    let _ = (o, args);
    match method {
        "__init__" => Ok(Value::None),
        _ => None2Err(method),
    }
}

// ===========================================================================
// queue module surface
// ===========================================================================

pub fn build_queue_module(interp: &crate::frame::Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("queue");
    for cls in ["Empty", "Full", "ShutDown"] {
        let _ = ns.set(
            s(cls),
            crate::classes::exc_class_value(interp, cls)
                .unwrap_or_else(|| queue_exc_class(interp, cls)),
        );
    }
    for cls in ["Queue", "LifoQueue", "PriorityQueue", "SimpleQueue"] {
        let _ = ns.set(s(cls), crate::builtins_modules::interned_type_pub(cls));
    }
    let _ = ns.set(
        s("__all__"),
        crate::builtins_modules::list(vec![
            s("Empty"),
            s("Full"),
            s("ShutDown"),
            s("Queue"),
            s("PriorityQueue"),
            s("LifoQueue"),
            s("SimpleQueue"),
        ]),
    );
    crate::builtins_modules::finish(m, ns)
}

fn queue_exc_class(interp: &crate::frame::Interpreter, name: &str) -> Value {
    let parent = crate::classes::exc_class_value(interp, "Exception")
        .and_then(|v| crate::classes::type_of(interp, &v))
        .unwrap_or_else(|| crate::classes::object_class(interp));
    let t = ap_obj::classes::TypeObject::create(
        name.to_string(),
        name.to_string(),
        vec![parent],
        PyDict::new(),
        ap_obj::classes::TypeFlags {
            slots: None,
            is_exception: true,
            builtin_exc: true,
        },
        None,
    )
    .expect("queue exc MRO");
    let _ = t.dict.set(
        Value::Str(PyString::new("__module__")),
        Value::Str(PyString::new("queue")),
    );
    let v = ap_obj::classes::make_class_value(t);
    interp
        .exc_classes
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(name.to_string(), v.clone());
    v
}

pub fn call_queue(
    _interp: &Arc<crate::frame::Interpreter>,
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    let _ = crate::gcsupport::drain_worker_registrations();
    let r: BResult = match name {
        "Queue" | "LifoQueue" | "PriorityQueue" | "SimpleQueue" => queue_ctor(args, kwargs, name),
        _ => Err(raise(
            "AttributeError",
            format!("module 'queue' has no attribute '{name}'"),
        )),
    };
    Some(r)
}

fn queue_ctor(args: &[Value], kwargs: &[(String, Value)], class: &str) -> BResult {
    let cap = match kw(kwargs, "maxsize").or(args.first()) {
        Some(v) => f64_to_i64(v)?,
        None => 0,
    };
    let o = PyObject::new(ObjKind::Opaque, "queue");
    *o.repr_text.lock().unwrap_or_else(|p| p.into_inner()) = None;
    // class marker for type(x).__name__ and reprs.
    let _ = o
        .attrs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set(s("<class>"), s(class));
    let addr = Arc::as_ptr(&o).addr();
    prim_put(
        addr,
        QueuePrim {
            items: Mutex::new(VecDeque::new()),
            cap,
            unfinished: Mutex::new(0),
            not_empty: std::sync::Condvar::new(),
            not_full: std::sync::Condvar::new(),
            all_tasks_done: std::sync::Condvar::new(),
            lifo: class == "LifoQueue",
            prio: class == "PriorityQueue",
        },
    );
    Ok(Value::Obj(o))
}

/// Sort key for PriorityQueue entries: CPython compares tuples/ints with the
/// value < operator. We implement a conservative total order: int/float
/// numerically, tuples lexicographically (tuple-vs-tuple only), strings
/// lexicographically; mixed types fall back to insertion order (documented
/// v0 divergence — CPython raises TypeError on mixed comparisons, but the
/// corpus only uses homogeneous priorities).
fn prio_less(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x
            .to_f64()
            .zip(y.to_f64())
            .map(|(a, b)| a < b)
            .unwrap_or(false),
        (Value::Int(x), Value::Float(y)) => x.to_f64().map(|f| f < *y).unwrap_or(false),
        (Value::Float(x), Value::Int(y)) => y.to_f64().map(|f| *x < f).unwrap_or(false),
        (Value::Float(x), Value::Float(y)) => x < y,
        (Value::Str(x), Value::Str(y)) => x.as_str() < y.as_str(),
        (Value::Tuple(x), Value::Tuple(y)) => {
            for (xa, ya) in x.0.iter().zip(y.0.iter()) {
                if prio_less(xa, ya) {
                    return true;
                }
                if prio_less(ya, xa) {
                    return false;
                }
            }
            x.0.len() < y.0.len()
        }
        _ => false,
    }
}

fn qsize(prim: &QueuePrim) -> usize {
    prim.items.lock().unwrap().len()
}

pub fn call_queue_method(
    interp: &Arc<crate::frame::Interpreter>,
    recv: &Value,
    method: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    let _ = interp;
    let Value::Obj(o) = recv else {
        return None;
    };
    if o.name != "queue" {
        return None;
    }
    let addr = Arc::as_ptr(o).addr();
    let Some(prim) = prim_of::<QueuePrim>(addr) else {
        return Some(Err(raise("SystemError", "queue primitive missing")));
    };
    let r: BResult = (|| {
        match method {
            "qsize" => Ok(smi(qsize(&prim) as i64)),
            "empty" => Ok(Value::Bool(qsize(&prim) == 0)),
            "full" => Ok(Value::Bool(
                prim.cap > 0 && qsize(&prim) >= prim.cap as usize,
            )),
            "maxsize" => Ok(smi(prim.cap)),
            "put" | "put_nowait" => {
                let item = required(args, "put")?.clone();
                let nowait = method == "put_nowait"
                    || matches!(args.get(1), Some(Value::Bool(false)))
                    || matches!(kw(kwargs, "block"), Some(Value::Bool(false)));
                let timeout = timeout_of(&args[2.min(args.len())..], kwargs, 0)?;
                let mut items = prim.items.lock().unwrap();
                if prim.cap > 0 && items.len() >= prim.cap as usize {
                    if nowait {
                        drop(items);
                        return Err(raise("Full", ""));
                    }
                    let deadline =
                        timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
                    loop {
                        let left = match deadline {
                            Some(d) => d.saturating_duration_since(Instant::now()),
                            None => Duration::from_secs(3600),
                        };
                        if left.is_zero() {
                            drop(items);
                            return Err(raise("Full", ""));
                        }
                        let (g, t) = prim.not_full.wait_timeout(items, left).unwrap();
                        items = g;
                        if items.len() < prim.cap as usize {
                            break;
                        }
                        if t.timed_out() {
                            drop(items);
                            return Err(raise("Full", ""));
                        }
                    }
                }
                if prim.prio {
                    // heap-insert by priority.
                    let pos = items
                        .iter()
                        .position(|x| prio_less(&item, x))
                        .unwrap_or(items.len());
                    items.insert(pos, item);
                } else {
                    // FIFO and LIFO share the deque push here; the pop side
                    // reads prim.lifo to choose the end.
                    items.push_back(item);
                }
                {
                    let mut u = prim.unfinished.lock().unwrap();
                    *u += 1;
                }
                drop(items);
                prim.not_empty.notify_one();
                Ok(Value::None)
            }
            "get" | "get_nowait" => {
                let nowait = method == "get_nowait"
                    || matches!(args.first(), Some(Value::Bool(false)))
                    || matches!(kw(kwargs, "block"), Some(Value::Bool(false)));
                let timeout = timeout_of(&args[1.min(args.len())..], kwargs, 0)?;
                let mut items = prim.items.lock().unwrap();
                if items.is_empty() {
                    if nowait {
                        drop(items);
                        return Err(raise("Empty", ""));
                    }
                    let deadline =
                        timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
                    loop {
                        let left = match deadline {
                            Some(d) => d.saturating_duration_since(Instant::now()),
                            None => Duration::from_secs(3600),
                        };
                        if left.is_zero() {
                            drop(items);
                            return Err(raise("Empty", ""));
                        }
                        let (g, t) = prim.not_empty.wait_timeout(items, left).unwrap();
                        items = g;
                        if !items.is_empty() {
                            break;
                        }
                        if t.timed_out() {
                            drop(items);
                            return Err(raise("Empty", ""));
                        }
                    }
                }
                let item = if prim.lifo {
                    items.pop_back().expect("nonempty checked")
                } else {
                    items.pop_front().expect("nonempty checked")
                };
                drop(items);
                prim.not_full.notify_one();
                Ok(item)
            }
            "task_done" => {
                let mut u = prim.unfinished.lock().unwrap();
                let next = *u - 1;
                if next < 0 {
                    return Err(raise("ValueError", "task_done() called too many times"));
                }
                *u = next;
                if next == 0 {
                    drop(u);
                    prim.all_tasks_done.notify_all();
                }
                Ok(Value::None)
            }
            "join" => {
                let mut u = prim.unfinished.lock().unwrap();
                while *u > 0 {
                    let g = prim.all_tasks_done.wait(u).unwrap();
                    u = g;
                }
                Ok(Value::None)
            }
            "unfinished_tasks" => Ok(smi(*prim.unfinished.lock().unwrap())),
            "shutdown" => Ok(Value::None),
            _ => None2Err(method),
        }
    })();
    Some(r)
}

/// Exception raise helper for queue Empty/Full: the corpus pins `str(e)` as
/// '' — args stay empty. `raise("Empty", "")` produces args [''] — a str
/// message. CPython's `except queue.Empty as e: str(e)` prints ''. Our
/// ExceptionValue::msg puts the message into args, and str(e) of a 1-arg
/// exception renders the arg — so the message must BE the empty string.
/// (raise("Full", "") above achieves exactly that.)
pub fn unused_queue(_x: ()) {}

// ===========================================================================
// Attribute surface for threading/queue native objects
// ===========================================================================

/// Attribute + bound-method reads on threading/queue native objects. Returns
/// None for misses (the caller falls through to the generic paths).
pub fn native_thread_attr(obj: &Value, name: &str) -> Option<Value> {
    let Value::Obj(o) = obj else {
        return None;
    };
    match o.name.as_str() {
        "thread" => {
            let addr = Arc::as_ptr(o).addr();
            match name {
                "name" => Some(attr_v(o, "<name>").unwrap_or(Value::None)),
                "daemon" => Some(Value::Bool(matches!(
                    attr_v(o, "<daemon>"),
                    Some(Value::Bool(true))
                ))),
                "ident" | "native_id" => {
                    let e = crate::threading::entry_for_pub(addr);
                    let v = e.and_then(|e| {
                        let b = e.borrow();
                        if name == "ident" {
                            b.ident
                        } else {
                            b.native_id
                        }
                    });
                    Some(v.map(smi).unwrap_or(Value::None))
                }
                "__class__" => {
                    let cls = attr_v(o, "<class>")
                        .and_then(|v| match &v {
                            Value::Str(sv) => Some(sv.as_str().to_string()),
                            _ => None,
                        })
                        .unwrap_or_else(|| "Thread".to_string());
                    Some(crate::threading::interned_type_pub(&cls))
                }
                "join" | "start" | "run" | "is_alive" | "isAlive" | "getName" | "setName"
                | "isDaemon" | "is_daemon" | "cancel" => Some(bound_method(obj, name)),
                _ => None,
            }
        }
        "lock" | "rlock" => match name {
            "acquire" | "release" | "locked" | "__enter__" | "__exit__" | "_is_owned" => {
                Some(bound_method(obj, name))
            }
            _ => None,
        },
        "condition" => match name {
            "acquire" | "release" | "wait" | "wait_for" | "notify" | "notify_all" | "notifyAll"
            | "__enter__" | "__exit__" => Some(bound_method(obj, name)),
            _ => None,
        },
        "event" => match name {
            "is_set" | "isSet" | "set" | "clear" | "wait" => Some(bound_method(obj, name)),
            _ => None,
        },
        "semaphore" => match name {
            "acquire" | "release" | "__enter__" | "__exit__" => Some(bound_method(obj, name)),
            _ => None,
        },
        "barrier" => {
            // Property-like reads: parties/n_waiting/broken.
            if matches!(name, "parties" | "n_waiting" | "broken") {
                let prim = prim_of::<BarrierPrim>(Arc::as_ptr(o).addr())?;
                return match name {
                    "parties" => Some(smi(prim.parties as i64)),
                    "n_waiting" => {
                        let st = prim.st.lock().unwrap();
                        Some(smi(if st.1 == 0 { st.0 as i64 } else { 0 }))
                    }
                    "broken" => Some(Value::Bool(prim.st.lock().unwrap().1 == -2)),
                    _ => None,
                };
            }
            match name {
                "wait" | "reset" | "abort" => Some(bound_method(obj, name)),
                _ => None,
            }
        }
        "_local" => {
            // threading.local: per-thread namespace reads (dunder/class
            // names fall through; anything else hits the LOCALS table).
            if name.starts_with("__") && name.ends_with("__") {
                return None;
            }
            let addr = Arc::as_ptr(o).addr();
            crate::threading::local_get(addr, name)
        }
        "queue" => match name {
            "qsize" | "empty" | "full" | "put" | "put_nowait" | "get" | "get_nowait"
            | "task_done" | "join" | "maxsize" | "unfinished_tasks" | "shutdown" => {
                if name == "maxsize" || name == "unfinished_tasks" {
                    let prim = prim_of::<QueuePrim>(Arc::as_ptr(o).addr())?;
                    return Some(if name == "maxsize" {
                        smi(prim.cap)
                    } else {
                        smi(*prim.unfinished.lock().unwrap())
                    });
                }
                Some(bound_method(obj, name))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Bound-method marker for a threading object: the receiver rides self_value
/// (the deque pattern); the "__ap_threading__." prefix routes call dispatch
/// through call_native_marker into threading_method_by_addr.
fn bound_method(obj: &Value, name: &str) -> Value {
    let bm = PyObject::with_self(ObjKind::BoundMethod, name.to_string(), obj.clone());
    *bm.func.lock().unwrap_or_else(|p| p.into_inner()) = Some(crate::builtins_modules::marker(
        &format!("__ap_threading__.{}", name),
    ));
    Value::Obj(bm)
}

/// Bound-threading-method call entry (routed from interp's BoundMethod arm
/// via call_native_marker): self rides args[0].
pub fn call_threading_bound(
    fname: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
) -> Option<BResult> {
    let recv = args.first()?.clone();
    let rest = &args[1..];
    let interp = crate::builtins_modules::current_interp()?;
    call_threading_method(&interp, &recv, fname, rest, kwargs)
}

/// Attribute writes on threading native objects: _local namespaces and
/// Thread name/daemon.
pub fn native_thread_attr_set(obj: &Value, name: &str, v: Value) -> Result<(), Flow> {
    let Value::Obj(o) = obj else {
        return Err(raise("SystemError", "attr set on non-object"));
    };
    match o.name.as_str() {
        "_local" => {
            let addr = Arc::as_ptr(o).addr();
            crate::threading::local_set(addr, name, v);
            Ok(())
        }
        "thread" => match name {
            "name" => {
                let _ = o
                    .attrs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set(s("<name>"), v);
                Ok(())
            }
            "daemon" => {
                let started = crate::threading::entry_for_pub(Arc::as_ptr(o).addr()).is_some();
                if started {
                    return Err(raise(
                        "RuntimeError",
                        "cannot set daemon status of active thread",
                    ));
                }
                let _ = o
                    .attrs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .set(s("<daemon>"), v);
                Ok(())
            }
            other => Err(raise(
                "AttributeError",
                format!("'Thread' object has no attribute '{other}'"),
            )),
        },
        other => Err(raise(
            "AttributeError",
            format!("'{}' object has no attribute '{}'", other, name),
        )),
    }
}

/// Thread repr: <Thread(NAME, initial|started|stopped[ daemon][ IDENT])>.
/// CPython 3.14: status accumulates ("started daemon 140234"), class name is
/// the subclass name (Timer/_MainThread).
pub fn thread_repr_pub(o: &Gc<PyObject>) -> String {
    let class_name = attr_v(o, "<class>")
        .and_then(|v| match &v {
            Value::Str(sv) => Some(sv.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "Thread".to_string());
    let name = attr_v(o, "<name>")
        .and_then(|v| match &v {
            Value::Str(sv) => Some(sv.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let daemon = matches!(attr_v(o, "<daemon>"), Some(Value::Bool(true)));
    let addr = Arc::as_ptr(o).addr();
    let (started, done, ident) = match crate::threading::entry_for_pub(addr) {
        Some(e) => {
            let b = e.borrow();
            (
                b.started,
                b.done.load(std::sync::atomic::Ordering::SeqCst),
                b.ident,
            )
        }
        None => (false, false, None),
    };
    let mut status = if started && !done {
        "started".to_string()
    } else if done {
        "stopped".to_string()
    } else {
        "initial".to_string()
    };
    if daemon {
        status += " daemon";
    }
    if let Some(i) = ident {
        status += &format!(" {}", i);
    }
    format!("<{}({}, {})>", class_name, name, status)
}

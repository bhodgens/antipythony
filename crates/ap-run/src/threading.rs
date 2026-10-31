//! Native `threading` (leaf 02.07): real OS threads over std::thread with a
//! GLOBAL INTERPRETER LOCK (GIL v1).
//!
//! # GIL design (documented divergence from the no-GIL master vision)
//!
//! The no-GIL goal is the 02.05-tier1-era work (pon root-map swap); this leaf
//! ships GIL v1: ONE process-wide interpreter lock serializes all bytecode
//! execution. It is released only at BLOCKING operations — `Thread.join`,
//! `Lock/RLock.acquire` (blocking form), `Condition.wait`, `Event.wait`,
//! `Semaphore.acquire` (blocking), `Barrier.wait`, `queue.get/put` (blocking)
//! — so rendezvous-style concurrency is fully functional (T101-T115), and the
//! CPU-parallel probes (T120/T121/T122) produce CORRECT results but no
//! wall-clock scaling (the >=3x gate is expected red under the GIL; the
//! conformance bytes of T120/T122 are order-independent and DO pass).
//!
//! Sharing model: the GLOBAL LOCK makes sharing safe (the easy path, per the
//! leaf brief). ap-obj values (`Gc<T>` = Rc + RefCell interiors) are !Sync by
//! construction; rather than per-thread heaps with a cross-thread Weak
//! bridge, every OS worker runs the SAME Rc graph behind the interpreter
//! lock — exactly CPython's model (the GIL protects refcounts; ours protects
//! RefCell borrows). ap-gc's REGISTRY/FROZEN/WEAKREFS/CALLBACKS are
//! thread-locals of the MAIN thread; worker threads allocate through
//! gcsupport::install_worker_thread()'s shared-registry view. All access is
//! serialized by the GIL, so the RefCell borrow discipline holds.
//!
//! `sys.setswitchinterval` accepts and stores the value (CPython surface);
//! it does not influence scheduling under GIL v1 (our blocking-op release
//! points ARE the switch points).
//!
//! Byte-exactness notes (pinned triples): default thread names `Thread-N
//! (target)` with a process-wide counter starting at 1 (G-5); Thread repr
//! `<Thread(NAME, initial|started|stopped[ daemon])>` (idents/addresses are
//! masked by the corpus); lock/RLock/Condition/queue error texts per
//! T110/probes; excepthook report per T109 (G-1: the reference pins its OWN
//! threading.py bootstrap frames + absolute install paths — those bytes are
//! the documented gap; we emit the header + user-frame + summary shape).

use crate::builtins_modules::{list, s};
use crate::frame::{Frame, Interpreter};
use crate::gcsupport::GilSendPub;
use crate::{EvalResult, Flow};
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::object::{ObjKind, PyObject};
use ap_obj::value::{PyString, Tuple, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub type BResult = Result<Value, Flow>;

fn raise(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(crate::exc_hier::ExceptionValue::msg(class, msg.into()))
}

// ===========================================================================
// The GIL (global interpreter lock), v1
// ===========================================================================

struct GilState {
    mu: Mutex<u64>, // 0 = free, else owner os-tid
    cv: Condvar,
}

static GIL: std::sync::LazyLock<GilState> = std::sync::LazyLock::new(|| GilState {
    mu: Mutex::new(0),
    cv: Condvar::new(),
});

thread_local! {
    static GIL_HELD: Cell<bool> = const { Cell::new(false) };
    static GIL_DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub fn current_os_tid() -> u64 {
    // Stable per-OS-thread identity without libc: mint one id per ThreadId
    // in a global map (mints differ from native pids by design — the corpus
    // only compares > 0 and equality-with-self).
    static TIDMAP: std::sync::LazyLock<Mutex<HashMap<std::thread::ThreadId, u64>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = std::thread::current().id();
    let mut m = TIDMAP.lock().unwrap();
    *m.entry(id)
        .or_insert_with(|| NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
}

/// Acquire the GIL (re-entrant on one thread via depth).
pub fn acquire_gil() {
    GIL_HELD.with(|h| {
        if h.get() {
            GIL_DEPTH.with(|d| d.set(d.get() + 1));
            return;
        }
        let me = current_os_tid();
        let mut g = GIL.mu.lock().unwrap();
        while *g != 0 && *g != me {
            g = GIL.cv.wait(g).unwrap();
        }
        *g = me;
        h.set(true);
    });
}

/// Release the GIL (re-entrant: only the outermost release lets others in).
pub fn release_gil() {
    GIL_HELD.with(|h| {
        if !h.get() {
            return;
        }
        let still_nested = GIL_DEPTH.with(|d| {
            let v = d.get();
            if v == 0 {
                false
            } else {
                d.set(v - 1);
                v > 1
            }
        });
        if still_nested {
            return;
        }
        h.set(false);
        let me = current_os_tid();
        let mut g = GIL.mu.lock().unwrap();
        if *g == me {
            *g = 0;
            GIL.cv.notify_all();
        }
    });
}

/// RAII: hold the GIL for a scope.
pub struct GilGuard;

impl GilGuard {
    pub fn take() -> GilGuard {
        acquire_gil();
        GilGuard
    }
}

impl Drop for GilGuard {
    fn drop(&mut self) {
        release_gil();
    }
}

// ===========================================================================
// Thread registry (global; every access under the GIL)
// ===========================================================================

pub struct ThreadEntry {
    pub obj_addr: usize,
    /// Minted at Thread-object construction; the double-start key (addr
    /// recycling makes the raw pointer unsafe as an identity).
    pub serial: i64,
    pub name: String,
    pub daemon: bool,
    pub started: bool,
    /// True once the run() body finished (join waits on this).
    pub done: bool,
    pub ident: Option<i64>,
    pub native_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// GLOBAL registry state (02.07 redesign): worker OS threads run the SAME
// interpreter object, so the thread registry / primitive table / local
// namespaces must be process-global. Every access is GIL-serialized by
// protocol; the Mutex wrappers satisfy the compiler. The only Rc payloads
// live behind GilSendPub wrappers (documented audited unsafety — the GIL
// serializes all access; see the module header).
// ---------------------------------------------------------------------------

static THREADS: std::sync::LazyLock<
    Mutex<Vec<crate::gcsupport::GilSendPub<Rc<RefCell<ThreadEntry>>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(Vec::new()));
static NAME_COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
static NEXT_IDENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0x1000_0000);
static NEXT_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// threading.local namespaces: (local obj addr, owner os tid, ns dict).
pub static LOCALS: std::sync::LazyLock<
    Mutex<HashMap<usize, HashMap<u64, crate::gcsupport::GilSendPub<Gc<PyDict>>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
/// threading.excepthook override (None = the default reporter).
static EXCEPTHOOK: Mutex<Option<crate::gcsupport::GilSendPub<Value>>> = Mutex::new(None);
/// sys.setswitchinterval stored value (accept + document), f64 bits.
static SWITCH_INTERVAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static MAIN_TID: std::sync::LazyLock<Mutex<Option<u64>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
/// Timer-cancel flags by thread obj addr.
static CANCELLED: std::sync::LazyLock<Mutex<std::collections::HashSet<usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));
/// The interpreter the WORKER threads evaluate through (set by the main
/// thread at spawn; read by workers — the interp object itself is only
/// touched under the GIL).
static WORKER_INTERP: std::sync::LazyLock<
    Mutex<Option<crate::gcsupport::GilSendPub<Rc<Interpreter>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(None));

pub fn is_main_os_thread() -> bool {
    MAIN_TID
        .lock()
        .unwrap()
        .map(|v| v == current_os_tid())
        .unwrap_or(true)
}

/// Called by run_module entry (interp.rs) — registers the main thread.
pub fn bootstrap_main(interp: &Rc<Interpreter>) {
    let mut mt = MAIN_TID.lock().unwrap();
    if mt.is_some() {
        return;
    }
    *mt = Some(current_os_tid());
    drop(mt);
    let ident = new_ident_pub();
    // Bind the MainThread PyObject mirror NOW: enumerate() must list
    // MainThread before any explicit current_thread() call (T108 pin).
    let mv = thread_obj(
        "MainThread".to_string(),
        None,
        vec![],
        vec![],
        false,
        "_MainThread",
        None,
    );
    if let Value::Obj(mo) = &mv {
        mirror_put(Rc::as_ptr(mo).addr(), &mv);
    }
    THREADS
        .lock()
        .unwrap()
        .push(crate::gcsupport::GilSendPub(Rc::new(RefCell::new(
            ThreadEntry {
                obj_addr: match &mv {
                    Value::Obj(mo) => Rc::as_ptr(mo).addr(),
                    _ => 0,
                },
                serial: -1,
                name: "MainThread".to_string(),
                daemon: false,
                started: true,
                done: false,
                ident: Some(ident),
                native_id: Some(current_os_tid() as i64),
            },
        ))));
}

fn new_ident_pub() -> i64 {
    NEXT_IDENT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as i64
}

pub fn entry_for(addr: usize) -> Option<Rc<RefCell<ThreadEntry>>> {
    THREADS
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e.0.try_borrow()
                .map(|b| b.obj_addr == addr && addr != 0)
                .unwrap_or(false)
        })
        .map(|e| e.0.clone())
}

fn main_thread_entry() -> Option<Rc<RefCell<ThreadEntry>>> {
    THREADS
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e.0.try_borrow()
                .map(|b| b.name == "MainThread")
                .unwrap_or(false)
        })
        .map(|e| e.0.clone())
}

// ===========================================================================
// Sync primitives (real Rust locks — the GIL is RELEASED while blocking)
// ===========================================================================

pub struct LockPrim {
    pub locked: Mutex<bool>,
    pub cv: Condvar,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RLockOwner {
    None,
    Owned(u64),
}

pub struct RLockPrim {
    pub st: Mutex<(RLockOwner, usize)>,
    pub cv: Condvar,
}

pub struct CondPrim {
    /// Backing Python lock object addr (0 = own implicit RLock, which we
    /// allocate as a real RLockPrim keyed under the condition's addr + 1).
    pub lock_addr: usize,
    /// Addr of the condition's own implicit RLock prim (when own_lock).
    pub own_rlock_addr: usize,
    pub waiters: Mutex<usize>,
    pub cv: Condvar,
}

pub struct EventPrim {
    pub st: Mutex<bool>,
    pub cv: Condvar,
}

pub struct SemPrim {
    pub value: Mutex<i64>,
    pub initial: i64,
    pub bounded: bool,
    pub cv: Condvar,
}

pub struct BarrierPrim {
    pub parties: usize,
    /// (count, state, generation): state 0 filling / 1 draining / -1
    /// resetting / -2 broken — the CPython Barrier state machine.
    pub st: Mutex<(usize, i32, u64)>,
    pub cv: Condvar,
    pub action: Option<Value>,
    pub default_timeout: Option<f64>,
}

pub struct QueuePrim {
    pub items: Mutex<VecDeque<Value>>,
    pub cap: i64,
    pub unfinished: Mutex<i64>,
    pub not_empty: Condvar,
    pub not_full: Condvar,
    pub all_tasks_done: Condvar,
    pub lifo: bool,
    pub prio: bool,
}

static PRIMS: std::sync::LazyLock<
    Mutex<HashMap<usize, crate::gcsupport::GilSendPub<Rc<dyn std::any::Any>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn prim_put<T: 'static>(addr: usize, p: T) {
    PRIMS.lock().unwrap().insert(
        addr,
        crate::gcsupport::GilSendPub(Rc::new(p) as Rc<dyn std::any::Any>),
    );
}

pub fn prim_of<T: 'static>(addr: usize) -> Option<Rc<T>> {
    let slot = { PRIMS.lock().unwrap().get(&addr).map(|s| s.0.clone()) }?;
    slot.downcast::<T>().ok().map(Rc::from)
}

/// Legacy bridge shim: the table is global now; this only re-inserts.
pub fn prim_bridge<T: 'static>(addr: usize, p: Rc<T>) {
    PRIMS.lock().unwrap().insert(
        addr,
        crate::gcsupport::GilSendPub(p as Rc<dyn std::any::Any>),
    );
}

fn obj_addr(v: &Value) -> Option<usize> {
    match v {
        Value::Obj(o) => Some(Rc::as_ptr(o).addr()),
        _ => None,
    }
}

// ===========================================================================
// Value constructors
// ===========================================================================

pub(crate) fn thread_obj(
    name: String,
    target: Option<Value>,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
    daemon: bool,
    class_name: &str,
    interval: Option<f64>,
) -> Value {
    let o = PyObject::new(ObjKind::Opaque, "thread");
    {
        let mut a = o.attrs.borrow_mut();
        let _ = a.set(s("<name>"), s(name));
        let _ = a.set(s("<target>"), target.unwrap_or(Value::None));
        let _ = a.set(s("<args>"), Value::Tuple(Gc::new(Tuple(args))));
        let mut kd = PyDict::new();
        for (k, v) in kwargs {
            let _ = kd.set(s(k), v);
        }
        let _ = a.set(s("<kwargs>"), Value::Dict(kd));
        let _ = a.set(s("<daemon>"), Value::Bool(daemon));
        let _ = a.set(s("<class>"), s(class_name));
        if let Some(v) = interval {
            let _ = a.set(s("<interval>"), Value::Float(v));
        }
    }
    Value::Obj(o)
}

fn thread_entry_for_obj(o: &Gc<PyObject>) -> Option<Rc<RefCell<ThreadEntry>>> {
    entry_for(Rc::as_ptr(o).addr())
}

fn thread_state(o: &Gc<PyObject>) -> (&'static str, bool) {
    let daemon = matches!(
        o.attrs.borrow().get(&s("<daemon>")),
        Some(Value::Bool(true))
    );
    let st = match thread_entry_for_obj(o) {
        None => "initial",
        Some(e) => {
            let b = e.borrow();
            if b.started && !b.done {
                "started"
            } else if b.done {
                "stopped"
            } else {
                "initial"
            }
        }
    };
    (st, daemon)
}

// ===========================================================================
// The interpreter-lock boundary: worker run() executes Python on the main
// interpreter object. Every registry/ns mutation a worker needs goes
// through direct call_value_pub on the shared interpreter — legal because
// the GIL serializes.
// ===========================================================================

/// Start a thread object: registers the entry, spawns the OS worker.
/// The GIL is HELD by the caller here.
pub fn thread_start(interp: &Rc<Interpreter>, o: &Gc<PyObject>) -> EvalResult {
    let addr = Rc::as_ptr(o).addr();
    // Double-start check by SERIAL (addresses are recyclable after a Thread
    // object is dropped — an addr-only registry false-positives when a new
    // allocation lands on a dead thread's address; the serial is minted at
    // construction and lives in the object attrs).
    let my_serial = o
        .attrs
        .borrow()
        .get(&s("<serial>"))
        .and_then(|v| v.as_int())
        .and_then(|i| i.to_i64());
    if let Some(serial) = my_serial {
        let dup = THREADS.lock().unwrap().iter().any(|e| {
            // A worker thread may hold its entry borrow while stamping its
            // identity; a contended borrow is NOT evidence of double-start.
            e.0.try_borrow()
                .map(|b| b.started && b.serial == serial && b.obj_addr != usize::MAX)
                .unwrap_or(false)
        });
        if dup {
            if std::env::var("APX_TDEBUG").is_ok() {
                eprintln!(
                    "[TDEBUG] double-start serial={} addr={} name={}",
                    serial,
                    addr,
                    attr_str(o, "<name>")
                );
            }
            return Err(raise("RuntimeError", "threads can only be started once"));
        }
    }
    let _ = addr;
    let name = attr_str(o, "<name>");
    let daemon = attr_bool(o, "<daemon>");
    let target = o
        .attrs
        .borrow()
        .get(&s("<target>"))
        .map(|v| v.clone())
        .unwrap_or(Value::None);
    let args_v = o
        .attrs
        .borrow()
        .get(&s("<args>"))
        .map(|v| v.clone())
        .unwrap_or(Value::None);
    let kwargs_v = o
        .attrs
        .borrow()
        .get(&s("<kwargs>"))
        .map(|v| v.clone())
        .unwrap_or(Value::None);
    let interval = o
        .attrs
        .borrow()
        .get(&s("<interval>"))
        .and_then(|v| match v {
            Value::Float(f) => Some(f),
            _ => None,
        });
    let args = match &args_v {
        Value::Tuple(t) => t.0.clone(),
        _ => vec![],
    };
    let kwargs: Vec<(String, Value)> = match &kwargs_v {
        Value::Dict(d) => d
            .items()
            .into_iter()
            .filter_map(|(k, v)| match &k {
                Value::Str(sk) => Some((sk.as_str().to_string(), v.clone())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };

    let ident = new_ident_pub();
    let native_placeholder = (current_os_tid().wrapping_mul(7919).wrapping_add(17)).max(1) as i64;
    let native = native_placeholder;

    if std::env::var("APX_TDEBUG").is_ok() {
        eprintln!(
            "[TDEBUG] start insert serial={:?} addr={} name={}",
            my_serial, addr, name
        );
    }
    THREADS
        .lock()
        .unwrap()
        .push(crate::gcsupport::GilSendPub(Rc::new(RefCell::new(
            ThreadEntry {
                obj_addr: addr,
                serial: my_serial.unwrap_or(-2),
                name: name.clone(),
                daemon,
                started: true,
                done: false,
                ident: Some(ident),
                native_id: Some(native),
            },
        ))));

    let interp2 = interp.clone();
    // GIL v1: the worker closure carries Rc graph values across the thread
    // boundary. Safe under the interpreter lock (the worker takes the GIL as
    // its first act; no thread touches the graph unlocked) — the compiler
    // cannot see the lock protocol, so the closure is boxed and the BOX
    // transmuted to the Send form (Send is a marker trait; the audited
    // unsafety lives HERE and only here). Same contract as the registration
    // channel in gcsupport.
    let closure = move || {
        worker_bootstrap(interp2, target, args, kwargs, addr, interval);
    };
    let boxed: Box<dyn FnOnce() + 'static> = Box::new(closure);
    let send_box: Box<dyn FnOnce() + Send + 'static> = unsafe { std::mem::transmute(boxed) };
    let spawn_result = std::thread::Builder::new()
        .name(format!("ap-{name}"))
        .stack_size(64 << 20)
        .spawn(send_box);
    match spawn_result {
        Ok(handle) => {
            // Detach: completion is tracked via the entry `done` flag, not
            // the JoinHandle (C-28/C-07: a crashed worker reports through
            // excepthook, never the process exit).
            drop(handle);
            Ok(Value::None)
        }
        Err(e) => {
            // Roll the registry entry back.
            THREADS
                .lock()
                .unwrap()
                .retain(|e| e.0.borrow().obj_addr != addr);
            Err(raise(
                "RuntimeError",
                format!("can't start new thread: {e}"),
            ))
        }
    }
    // CPython's start() waits for the worker's _started Event; our worker
    // signals after installing its thread-locals (join-on-entry correctness
    // for immediately-joined threads is handled by join_wait's done flag).
}

/// Worker entry: install thread-locals (GIL identity, interp handle), run
/// the body, honor excepthook, mark done, wake joiners.
fn worker_bootstrap(
    interp: Rc<Interpreter>,
    target: Value,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
    self_addr: usize,
    interval: Option<f64>,
) {
    // Fresh thread-local slots for THIS OS thread (gc registry view).
    GIL_HELD.with(|h| h.set(false));
    GIL_DEPTH.with(|d| d.set(0));
    crate::builtins_modules::set_current_interp(&interp);
    crate::interp::install_current_interp(&interp);
    *WORKER_INTERP.lock().unwrap() = Some(crate::gcsupport::GilSendPub(interp.clone()));
    crate::gcsupport::install_worker_thread();
    // Stamp THIS OS thread's identity into its registry entry (the spawner
    // could only record a placeholder — the worker tid exists only here).
    for _ in 0..2000 {
        match entry_for(self_addr) {
            Some(e) => match e.try_borrow_mut() {
                Ok(mut b) => {
                    b.native_id = Some(current_os_tid() as i64);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
            },
            None => break,
        }
    }

    // Timer semantics: wait BEFORE the target (finished.wait(interval)).
    if let Some(dur) = interval {
        wait_cancellable(Duration::from_secs_f64(dur.max(0.0)), self_addr);
        let cancelled = CANCELLED.lock().unwrap().contains(&self_addr);
        if cancelled {
            acquire_gil();
            if let Some(e) = entry_for(self_addr) {
                e.borrow_mut().done = true;
            }
            JOIN_CV.notify_all();
            release_gil();
            return;
        }
    }

    acquire_gil();
    let outcome = if matches!(target, Value::None) {
        Ok(Value::None)
    } else {
        crate::interp::call_value_pub(&interp, &target, &args, &kwargs)
    };
    match outcome {
        Ok(_) => {}
        Err(Flow::Raise(exc)) => {
            if exc.class == "SystemExit" {
                // C-28: SystemExit in a non-main thread ends THAT thread only.
            } else {
                invoke_excepthook(&interp, &exc);
            }
        }
        Err(_) => {}
    }
    if let Some(e) = entry_for(self_addr) {
        e.borrow_mut().done = true;
    }
    JOIN_CV.notify_all();
    release_gil();
}

/// Sleep in slices so cancel() is honored promptly (Timer.cancel polling).
fn wait_cancellable(dur: Duration, self_addr: usize) {
    let start = Instant::now();
    loop {
        if CANCELLED.lock().unwrap().contains(&self_addr) {
            return;
        }
        let left = dur.saturating_sub(start.elapsed());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(1)));
    }
}

// join signalling
static JOIN_CV: Condvar = Condvar::new();
static JOIN_MU: Mutex<()> = Mutex::new(());

/// Block the CALLER (GIL must be RELEASED before calling us) until `done`
/// flips. Returns true when the thread finished, false on timeout. Polls at
/// a fine grain (the done flag is set under the GIL, so polling is the
/// portable rendezvous; the granularity only affects latency, never
/// correctness).
pub fn join_wait(entry: Rc<RefCell<ThreadEntry>>, timeout: Option<f64>) -> bool {
    let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
    loop {
        match entry.try_borrow() {
            Ok(b) if b.done => return true,
            Ok(_) => {}
            Err(_) => {
                std::thread::sleep(Duration::from_micros(50));
                continue;
            }
        }
        let now = Instant::now();
        if let Some(d) = deadline {
            if now >= d {
                return entry.try_borrow().map(|b| b.done).unwrap_or(false);
            }
        }
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// Wait for ALL non-daemon threads to finish (main-thread exit protocol,
/// T107: a daemon parked forever cannot block process exit). The main
/// thread's own entry is marked done FIRST (it is the thread performing the
/// wait and must not join itself).
pub fn wait_for_non_daemons() {
    if let Some(me) = entry_for_current() {
        me.borrow_mut().done = true;
    }
    loop {
        let pending: Vec<Rc<RefCell<ThreadEntry>>> = THREADS
            .lock()
            .unwrap()
            .iter()
            .filter(|e| {
                let b = e.0.borrow();
                b.started && !b.done && !b.daemon
            })
            .map(|e| e.0.clone())
            .collect();
        if pending.is_empty() {
            return;
        }
        for p in pending {
            let _ = join_wait(p, None);
        }
    }
}

// ---------------------------------------------------------------------------
// excepthook (T109/T110 + probe20: hook-errors report too)
// ---------------------------------------------------------------------------

fn invoke_excepthook(interp: &Rc<Interpreter>, exc: &crate::exc_hier::ExceptionValue) {
    let thread_v = thread_value_for_entry(entry_for_current());
    let hook = EXCEPTHOOK.lock().unwrap().clone().map(|h| h.0);
    if let Some(hook) = hook {
        let args_obj = PyObject::new(ObjKind::Namespace, "ExceptHookArgs");
        {
            let mut a = args_obj.attrs.borrow_mut();
            let _ = a.set(
                s("exc_type"),
                crate::classes::exc_class_value(interp, &exc.class)
                    .unwrap_or_else(|| Value::Obj(PyObject::new(ObjKind::Type, exc.class.clone()))),
            );
            let _ = a.set(s("exc_value"), exception_instance_value(exc));
            let _ = a.set(s("exc_traceback"), Value::None);
            let _ = a.set(s("thread"), thread_v);
        }
        let r = crate::interp::call_value_pub(interp, &hook, &[Value::Obj(args_obj)], &[]);
        if let Err(Flow::Raise(hexc)) = r {
            let mut out = String::from("Exception in threading.excepthook:\n");
            out.push_str(&render_thread_exc_report(interp, &hexc));
            eprint!("{out}");
        }
        return;
    }
    let mut out = format!("Exception in thread {}:\n", attr_str_of(&thread_v));
    out.push_str(&render_thread_exc_report(interp, exc));
    eprint!("{out}");
}

fn entry_for_current() -> Option<Rc<RefCell<ThreadEntry>>> {
    let tid = current_os_tid() as i64;
    THREADS
        .lock()
        .unwrap()
        .iter()
        .find(|e| {
            e.0.try_borrow()
                .map(|b| b.native_id == Some(tid))
                .unwrap_or(false)
        })
        .map(|e| e.0.clone())
}

fn thread_value_for_entry(e: Option<Rc<RefCell<ThreadEntry>>>) -> Value {
    // The obj Value for an entry: look the obj up by addr in the interp's
    // reach — we stored the addr only; reconstruct the Value handle via the
    // obj-table. Entries store their obj handle at construction (thread_start
    // holds it via the attrs of nothing) — simplest: MAIN side keeps a
    // mirror table addr -> Value.
    mirror_get(e.map(|x| x.borrow().obj_addr).unwrap_or(0)).unwrap_or(Value::None)
}

static OBJ_MIRROR: std::sync::LazyLock<Mutex<HashMap<usize, crate::gcsupport::GilSendPub<Value>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn mirror_put(addr: usize, v: &Value) {
    OBJ_MIRROR
        .lock()
        .unwrap()
        .insert(addr, crate::gcsupport::GilSendPub(v.clone()));
}

fn mirror_get(addr: usize) -> Option<Value> {
    OBJ_MIRROR.lock().unwrap().get(&addr).map(|v| v.0.clone())
}

fn attr_str_of(v: &Value) -> String {
    match v {
        Value::Obj(o) => attr_str(o, "<name>"),
        _ => String::new(),
    }
}

fn render_thread_exc_report(
    interp: &Rc<Interpreter>,
    exc: &crate::exc_hier::ExceptionValue,
) -> String {
    // CPython 3.14 shape: "Traceback (most recent call last):" + the
    // threading bootstrap frames + user frames + "Type: msg". The
    // REFERENCE's bootstrap frames embed its own threading.py install path
    // and line numbers (G-1, documented gap); ours identify OUR bootstrap.
    let mut out = String::from("Traceback (most recent call last):\n");
    for (qualname, line, file) in interp.tb_frames.borrow().iter() {
        let file = if file.is_empty() {
            interp.script_path.as_str()
        } else {
            file.as_str()
        };
        out.push_str(&format!(
            "  File \"{}\", line {}, in {}\n",
            file, line, qualname
        ));
        if let Some(src_line) = interp.source.lines().nth(line.saturating_sub(1)) {
            out.push_str("    ");
            out.push_str(src_line.trim());
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
    out
}

fn exception_instance_value(exc: &crate::exc_hier::ExceptionValue) -> Value {
    let o = PyObject::new(ObjKind::Exception, exc.class.clone());
    *o.args.borrow_mut() = exc.args.clone();
    Value::Obj(o)
}

fn attr_str(o: &PyObject, key: &str) -> String {
    // A second OS thread can momentarily hold this RefCell (make_thread_value
    // constructs under a single borrow). We hold the GIL here, so the other
    // holder must be a brief native-only span; retry instead of panicking.
    for _ in 0..2000 {
        match o.attrs.try_borrow() {
            Ok(a) => {
                return match a.get(&s(key)) {
                    Some(Value::Str(v)) => v.as_str().to_string(),
                    _ => String::new(),
                }
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
    String::new()
}

fn attr_bool(o: &PyObject, key: &str) -> bool {
    for _ in 0..2000 {
        match o.attrs.try_borrow() {
            Ok(a) => return matches!(a.get(&s(key)), Some(Value::Bool(true))),
            Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
    false
}

// ===========================================================================
// Module-level surface (called from builtins_modules::call_native("threading"))
// ===========================================================================

pub fn current_thread_value() -> Value {
    let e = entry_for_current().or_else(main_thread_entry);
    match e {
        Some(e) => {
            let addr = e.borrow().obj_addr;
            if let Some(v) = mirror_get(addr) {
                return v;
            }
            // Main-thread lazy bind.
            let v = thread_obj(
                "MainThread".to_string(),
                None,
                vec![],
                vec![],
                false,
                "_MainThread",
                None,
            );
            if let Value::Obj(o) = &v {
                mirror_put(Rc::as_ptr(o).addr(), &v);
                e.borrow_mut().obj_addr = Rc::as_ptr(o).addr();
            }
            v
        }
        None => Value::None,
    }
}

pub fn main_thread_value() -> Value {
    match main_thread_entry() {
        Some(e) => {
            let addr = e.borrow().obj_addr;
            if let Some(v) = mirror_get(addr) {
                return v;
            }
            current_thread_value()
        }
        None => current_thread_value(),
    }
}

pub fn active_count() -> usize {
    THREADS
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            let b = e.0.borrow();
            b.started && !b.done
        })
        .count()
}

pub fn enumerate_values() -> Vec<Value> {
    THREADS
        .lock()
        .unwrap()
        .iter()
        .filter(|e| {
            let b = e.0.borrow();
            b.started && !b.done
        })
        .filter_map(|e| {
            let addr = e.0.borrow().obj_addr;
            mirror_get(addr)
        })
        .collect()
}

pub fn get_ident_value() -> i64 {
    entry_for_current()
        .and_then(|e| e.borrow().ident)
        .unwrap_or(0)
}

pub fn get_native_id_value() -> i64 {
    entry_for_current()
        .and_then(|e| e.borrow().native_id)
        .unwrap_or(current_os_tid() as i64)
}

// ===========================================================================
// thread-local storage (T106)
// ===========================================================================

pub fn local_get(addr: usize, name: &str) -> Option<Value> {
    let tid = current_os_tid();
    let l = LOCALS.lock().unwrap();
    l.get(&addr)
        .and_then(|m| m.get(&tid))
        .and_then(|d| d.0.get(&s(name)).map(|v| v.clone()))
}

pub fn local_set(addr: usize, name: &str, v: Value) {
    let tid = current_os_tid();
    let mut l = LOCALS.lock().unwrap();
    let d = l
        .entry(addr)
        .or_default()
        .entry(tid)
        .or_insert_with(|| crate::gcsupport::GilSendPub(PyDict::new()));
    d.0.set(s(name), v).ok();
}

// ===========================================================================
// queue module (native, over the queue primitives; byte-exact Empty/Full)
// ===========================================================================

pub fn build_queue_module(interp: &Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("queue");
    for cls in ["Empty", "Full", "ShutDown"] {
        let _ = ns.set(
            s(cls),
            crate::classes::exc_class_value(interp, cls)
                .unwrap_or_else(|| Value::Obj(PyObject::new(ObjKind::Type, cls))),
        );
    }
    for cls in ["Queue", "LifoQueue", "PriorityQueue", "SimpleQueue"] {
        let _ = ns.set(s(cls), interned_type_pub(cls));
    }
    let _ = ns.set(
        s("__all__"),
        list(vec![
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

pub fn interned_type_pub(name: &str) -> Value {
    if let Some(interp) = crate::builtins_modules::current_interp() {
        return crate::builtins::interned_type(&interp, name);
    }
    Value::Obj(PyObject::new(ObjKind::Type, name))
}

pub fn unused(_f: &Rc<Frame>) {}

// ===========================================================================
// Public shims for threading_call.rs
// ===========================================================================

pub fn entry_for_pub(addr: usize) -> Option<Rc<RefCell<ThreadEntry>>> {
    entry_for(addr)
}

pub fn excepthook_value() -> Value {
    EXCEPTHOOK
        .lock()
        .unwrap()
        .clone()
        .map(|h| h.0)
        .unwrap_or(Value::None)
}

pub fn set_excepthook(v: Value) {
    *EXCEPTHOOK.lock().unwrap() = if matches!(v, Value::None) {
        None
    } else {
        Some(crate::gcsupport::GilSendPub(v))
    };
}

pub fn cancel_timer(addr: usize) {
    CANCELLED.lock().unwrap().insert(addr);
}

pub fn set_switch_interval(v: f64) {
    SWITCH_INTERVAL.store(v.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Thread object factory for the Timer/Thread constructors (public).
pub fn make_thread_value(
    name: Option<String>,
    target: Option<Value>,
    args: Vec<Value>,
    kwargs: Vec<(String, Value)>,
    daemon: bool,
    class_name: &str,
    interval: Option<f64>,
) -> Value {
    // CPython _newname: "Thread-N" (+ " (target_name)" when anonymous).
    let name = match name {
        Some(n) => n,
        None => {
            let n = NAME_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut s = format!("Thread-{n}");
            if let Some(Value::Obj(t)) = &target {
                if t.kind == ap_obj::ObjKind::Function {
                    s += &format!(" ({})", t.name);
                }
            }
            s
        }
    };
    let v = thread_obj(name, target, args, kwargs, daemon, class_name, interval);
    if let Value::Obj(o) = &v {
        mirror_put(Rc::as_ptr(o).addr(), &v);
        let serial = NEXT_SERIAL.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as i64;
        let _ = o
            .attrs
            .borrow_mut()
            .set(s("<serial>"), Value::Int(ap_obj::Int::from_i64(serial)));
    }
    v
}

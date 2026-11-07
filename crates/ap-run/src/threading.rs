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
use crate::{EvalResult, Flow};
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::object::{ObjKind, PyObject};
use ap_obj::value::{Tuple, Value};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
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

thread_local! {
    /// E7 (memperf audit 2026-11-06): the minted os-tid, cached per thread.
    /// current_os_tid used to lock the global TIDMAP on EVERY acquire_gil /
    /// release_gil / local op / rlock op; the mint is once-per-thread now.
    static OS_TID: Cell<u64> = const { Cell::new(0) };
}

pub fn current_os_tid() -> u64 {
    // Stable per-OS-thread identity without libc: mint one id per ThreadId
    // in a global map (mints differ from native pids by design — the corpus
    // only compares > 0 and equality-with-self). The Cell is fresh-zero on
    // each new OS thread, so the map lock is paid at most once per thread.
    let cached = OS_TID.with(|c| c.get());
    if cached != 0 {
        return cached;
    }
    static TIDMAP: std::sync::LazyLock<Mutex<HashMap<std::thread::ThreadId, u64>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = std::thread::current().id();
    let tid = {
        let mut m = TIDMAP.lock().unwrap();
        *m.entry(id)
            .or_insert_with(|| NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst))
    };
    OS_TID.with(|c| c.set(tid));
    tid
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
    /// E-safety (stress probe): a SHARED ATOMIC mirrored on the ThreadSlot.
    /// The worker's completion mark goes through the slot (mark_done) —
    /// never via borrow_mut on the entry RefCell — so it can never panic
    /// against a peer's legitimate borrow of the same cell.
    pub done: Arc<std::sync::atomic::AtomicBool>,
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

/// M2 (memperf audit 2026-11-06): the push-only Vec made EVERY threading API
/// call (`entry_for` x4 sites, `entry_for_current` x2, the dup-start scan) a
/// linear scan over the cumulative set of threads ever started, under one
/// global Mutex, borrowing every entry on the way. The table now keeps side
/// indexes (obj addr / native tid / serial) so lookups are O(1), and
/// `evict_stale_tables` (below) purges dead entries so the table stays
/// bounded by the live+joinable working set instead of growing forever.
///
/// The slot mirrors (addr/serial/tid) carry the index keys OUTSIDE the
/// RefCell so index maintenance never borrows an entry: a worker thread
/// stamps its real tid WITHOUT the table lock discipline the GIL gives the
/// scanner, so the tid mirror is an AtomicI64 (Relaxed load is safe for an
/// i64 identity — torn reads are impossible on the tier-1 targets, and the
/// corpus only compares tid equality/self-consistency).
pub struct ThreadSlot {
    pub entry: crate::gcsupport::GilSendPub<Rc<RefCell<ThreadEntry>>>,
    /// E-safety: clone of the entry's done atom — the worker marks
    /// completion through the table index WITHOUT borrowing the RefCell.
    pub done: Arc<std::sync::atomic::AtomicBool>,
    /// Mirror of ThreadEntry.daemon (immutable after push; lets
    /// wait_for_non_daemons filter via atomics — zero Rc clones).
    pub daemon: bool,
    /// Mirror of ThreadEntry.obj_addr (the MainThread lazy-bind path
    /// refreshes it via reindex_addr).
    pub addr: usize,
    /// Mirror of ThreadEntry.serial (immutable after push).
    pub serial: i64,
    /// Mirror of ThreadEntry.native_id (spawner placeholder -> real tid).
    pub tid: std::sync::atomic::AtomicI64,
}

pub struct ThreadsTable {
    pub entries: Vec<ThreadSlot>,
    by_addr: HashMap<usize, Vec<usize>>,
    by_tid: HashMap<i64, Vec<usize>>,
    by_serial: HashMap<i64, Vec<usize>>,
}

impl ThreadsTable {
    fn index_add(&mut self, idx: usize) {
        let s = &self.entries[idx];
        self.by_addr.entry(s.addr).or_default().push(idx);
        let tid = s.tid.load(std::sync::atomic::Ordering::Relaxed);
        self.by_tid.entry(tid).or_default().push(idx);
        self.by_serial.entry(s.serial).or_default().push(idx);
    }
    fn push_slot(&mut self, slot: ThreadSlot) {
        self.entries.push(slot);
        let idx = self.entries.len() - 1;
        self.index_add(idx);
    }
    fn rebuild_indexes(&mut self) {
        self.by_addr.clear();
        self.by_tid.clear();
        self.by_serial.clear();
        for i in 0..self.entries.len() {
            self.index_add(i);
        }
    }
    /// E7: move one slot's tid index entry after the worker swapped its real
    /// os-tid in (O(1), no entry borrow — uses the mirrored keys).
    fn reindex_tid(&mut self, idx: usize, old: i64, new: i64) {
        if old == new {
            return;
        }
        if let Some(v) = self.by_tid.get_mut(&old) {
            v.retain(|&i| i != idx);
            if v.is_empty() {
                self.by_tid.remove(&old);
            }
        }
        self.by_tid.entry(new).or_default().push(idx);
    }
    /// MainThread lazy-bind path (current_thread_value): an entry's obj_addr
    /// mirror changed; refresh mirrors + rebuild. At most once per process.
    fn reindex_addr(&mut self) {
        for s in self.entries.iter_mut() {
            // try_borrow: a worker may hold its entry mutably mid-stamp
            // (native thread, no GIL); that path never changes obj_addr,
            // so the mirrored value stays correct for a skipped slot.
            if let Ok(b) = s.entry.0.try_borrow() {
                s.addr = b.obj_addr;
            }
        }
        self.rebuild_indexes();
    }
}

static THREADS: std::sync::LazyLock<Mutex<ThreadsTable>> = std::sync::LazyLock::new(|| {
    Mutex::new(ThreadsTable {
        entries: Vec::new(),
        by_addr: HashMap::new(),
        by_tid: HashMap::new(),
        by_serial: HashMap::new(),
    })
});
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
/// Timer-cancel events by thread obj addr. E7 (memperf audit 2026-11-06):
/// the value is a Condvar event pair — `wait_cancellable` blocks on the
/// per-Timer Condvar with the remaining time as its WAIT TIMEOUT instead of
/// locking the GLOBAL map once per 1 ms slice (a 60 s Timer used to take
/// 60k acquisitions + 60k wakeups). The map lock is now taken exactly once
/// by the waiter (fetch), once by cancel_timer (set + notify), and by the
/// eviction sweep.
pub struct CancelEvent {
    pub cancelled: Mutex<bool>,
    pub cv: Condvar,
}
/// Keyed (timer obj addr, construction serial): the serial blocks the
/// addr-recycling alias where a stale pre-cancelled flag of a dead Timer
/// would suppress a fresh Timer allocated on the same address.
static CANCELLED: std::sync::LazyLock<Mutex<HashMap<(usize, i64), Arc<CancelEvent>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// E7: entry lookup by addr that WAITS (bounded) for the entry to become
/// available, instead of the old 2000×50µs sleep-retry loop. The spawner
/// inserts the entry BEFORE the OS thread exists (thread_start), so on the
/// GIL protocol the entry is already visible when the worker stamps its
/// identity; a contended borrow only happens if a main-side op holds it
/// briefly. One short bounded wait, then proceed without stamping (the
/// entry keeps its placeholder native_id — is_alive/join are addr-keyed and
/// unaffected; the corpus only compares native_id > 0 + self-equality).
fn stamp_worker_identity(self_addr: usize) {
    let me = current_os_tid() as i64;
    // Bounded: 200 x 50us = 10ms worst case, then proceed with the
    // placeholder tid (native_id reads are the only consumer; is_alive/
    // join/exit protocol are addr-keyed and unaffected).
    for _ in 0..200 {
        let e = entry_for(self_addr);
        match e {
            Some(e) => match e.try_borrow_mut() {
                Ok(mut b) => {
                    let old_tid = b.native_id.unwrap_or(0);
                    b.native_id = Some(me);
                    drop(b);
                    // M2: swap the slot's tid index entry from the spawner
                    // placeholder to the real worker tid — O(1), mirror
                    // based (no entry borrow while the table lock is held).
                    let mut t = THREADS.lock().unwrap();
                    if let Some(&idx) = t.by_addr.get(&self_addr).and_then(|v| v.last()) {
                        t.entries[idx]
                            .tid
                            .store(me, std::sync::atomic::Ordering::Relaxed);
                        t.reindex_tid(idx, old_tid, me);
                    }
                    return;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_micros(50)),
            },
            None => std::thread::sleep(std::time::Duration::from_micros(50)),
        }
    }
}
/// E-safety (stress probe): worker closures carry Rc payload clones (the
/// Thread target/args/kwargs + interp handle) whose natural Drop runs AFTER
/// release_gil — decrementing a shared non-atomic strong_count while the
/// GIL holder mutates the same graphs is the debug-runtime UB class the
/// 300-thread churn exposes (baseline crashes the same probe). Workers now
/// MOVE their tail payloads into this queue (a plain Vec push, zero Rc
/// traffic) and the MAIN thread drops them at a GIL-held safe point
/// (drain_worker_reclaim, called from evict_stale_tables at the major-GC
/// drain hook). Mirrors the gc-registration forward-channel design.
static WORKER_RECLAIM: std::sync::LazyLock<
    Mutex<Vec<crate::gcsupport::GilSendPub<Box<dyn std::any::Any>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

/// The boxed payload is !Send (Rc graph) — the audited GIL-protocol
/// crossing used by every registry table in this module (GilSendPub).
fn reclaim_worker_payload<T: std::any::Any>(vals: T) {
    WORKER_RECLAIM
        .lock()
        .unwrap()
        .push(crate::gcsupport::GilSendPub(Box::new(vals)));
}

/// Drain + drop reclaimed worker payloads. Call ONLY on the main thread
/// with the GIL held — non-main callers must not steal the drop queue (the
/// payloads are main-heap Rc clones; a worker-side drop defeats the point).
pub fn drain_worker_reclaim() -> usize {
    if !is_main_os_thread() {
        return 0;
    }
    let batches: Vec<Box<dyn std::any::Any>> = {
        let mut q = WORKER_RECLAIM.lock().unwrap();
        q.drain(..).map(|b| b.0).collect()
    };
    let n = batches.len();
    drop(batches);
    n
}

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
    let mtid = current_os_tid() as i64;
    let maddr = match &mv {
        Value::Obj(mo) => Rc::as_ptr(mo).addr(),
        _ => 0,
    };
    let main_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    THREADS.lock().unwrap().push_slot(ThreadSlot {
        entry: crate::gcsupport::GilSendPub(Rc::new(RefCell::new(ThreadEntry {
            obj_addr: maddr,
            serial: -1,
            name: "MainThread".to_string(),
            daemon: false,
            started: true,
            done: main_done.clone(),
            ident: Some(ident),
            native_id: Some(mtid),
        }))),
        addr: maddr,
        serial: -1,
        tid: std::sync::atomic::AtomicI64::new(mtid),
        done: main_done,
        daemon: false,
    });
}

/// E-safety: mark an entry done THROUGH THE SLOT atom — zero RefCell
/// borrows (the old `entry.borrow_mut().done = true` panicked against a
/// peer's stamp-time borrow of the same cell under addr-recycling).
/// The exit rendezvous marks the MainThread slot done (serial -1, slot
/// order stable) without borrowing the entry RefCell off-lock.
fn mark_done_main() {
    let t = THREADS.lock().unwrap();
    for s in t.entries.iter() {
        if s.serial == -1 {
            s.done.store(true, std::sync::atomic::Ordering::SeqCst);
            break;
        }
    }
}

fn mark_done(addr: usize) {
    {
        let t = THREADS.lock().unwrap();
        if let Some(&idx) = t.by_addr.get(&addr).and_then(|v| v.last()) {
            if let Some(s) = t.entries.get(idx) {
                s.done.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
    // E7: the done ATOM is the join predicate — wake every parked joiner
    // (lock-then-notify: a joiner that checks under JOIN_MU right before
    // we take it either sees the flag or blocks and receives this wake).
    let _g = JOIN_MU.lock().unwrap();
    JOIN_CV.notify_all();
}

/// E-safety: completion read — never panics (try_borrow-based).
fn entry_done(entry: &Rc<RefCell<ThreadEntry>>) -> bool {
    entry
        .try_borrow()
        .map(|b| b.done.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

fn new_ident_pub() -> i64 {
    NEXT_IDENT.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as i64
}

pub fn entry_for(addr: usize) -> Option<Rc<RefCell<ThreadEntry>>> {
    if addr == 0 {
        return None;
    }
    let t = THREADS.lock().unwrap();
    // M2: O(1) addr index (was a full linear scan + try_borrow per entry).
    t.by_addr
        .get(&addr)
        .and_then(|v| v.last())
        .and_then(|&i| t.entries.get(i))
        .map(|s| s.entry.0.clone())
}

fn main_thread_entry() -> Option<Rc<RefCell<ThreadEntry>>> {
    // M2: MainThread is slot 0 (bootstrap_main installs it first; the
    // eviction sweep never drops it). O(1) fast path, scan as fallback.
    let t = THREADS.lock().unwrap();
    if let Some(s) = t.entries.first() {
        if s.serial == -1 {
            return Some(s.entry.0.clone());
        }
    }
    t.entries
        .iter()
        .find(|s| {
            s.entry
                .0
                .try_borrow()
                .map(|b| b.name == "MainThread")
                .unwrap_or(false)
        })
        .map(|s| s.entry.0.clone())
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

/// M4: keys of PRIMS entries that are a Condition's IMPLICIT backing RLock
/// (key = condition addr + 1 — a phantom address that is never a real
/// object, so registry liveness must be read off the OWNING Condition
/// object instead). The eviction sweep drops these once the condition dies.
static OWN_RLOCKS: std::sync::LazyLock<Mutex<std::collections::HashSet<usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// cond_ctor's implicit RLock: register the (cond addr + 1) key as owned so
/// eviction keys its liveness off the Condition object.
pub fn prim_put_owned_rlock(cond_addr: usize, p: RLockPrim) {
    let key = cond_addr + 1;
    OWN_RLOCKS.lock().unwrap().insert(key);
    prim_put(key, p);
}

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
            if b.started && !b.done.load(std::sync::atomic::Ordering::SeqCst) {
                "started"
            } else if b.done.load(std::sync::atomic::Ordering::SeqCst) {
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
        // M2: O(1) serial index (was a linear scan borrowing every entry).
        // Clone candidate Rc's under the lock, check them outside it.
        let candidates: Vec<Rc<RefCell<ThreadEntry>>> = {
            let t = THREADS.lock().unwrap();
            t.by_serial
                .get(&serial)
                .map(|v| {
                    v.iter()
                        .filter_map(|&i| t.entries.get(i))
                        .map(|s| s.entry.0.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let dup = candidates.iter().any(|e| {
            // A worker thread may hold its entry borrow while stamping its
            // identity; a contended borrow is NOT evidence of double-start.
            e.try_borrow()
                .map(|b| b.started && b.obj_addr != usize::MAX)
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
    // E7: a Timer's worker parks on its per-(addr, serial) CancelEvent;
    // register it at start so cancel_timer / wait_cancellable rendezvous
    // without ever polling the global map. A pre-cancelled flag (cancel()
    // before start()) is preserved: only mint when absent.
    if interval.is_some() {
        let key = (addr, my_serial.unwrap_or(-2));
        let mut m = CANCELLED.lock().unwrap();
        if !m.contains_key(&key) {
            m.insert(
                key,
                Arc::new(CancelEvent {
                    cancelled: Mutex::new(false),
                    cv: Condvar::new(),
                }),
            );
        }
    }

    if std::env::var("APX_TDEBUG").is_ok() {
        eprintln!(
            "[TDEBUG] start insert serial={:?} addr={} name={}",
            my_serial, addr, name
        );
    }
    let wdone = Arc::new(std::sync::atomic::AtomicBool::new(false));
    THREADS.lock().unwrap().push_slot(ThreadSlot {
        entry: crate::gcsupport::GilSendPub(Rc::new(RefCell::new(ThreadEntry {
            obj_addr: addr,
            serial: my_serial.unwrap_or(-2),
            name: name.clone(),
            daemon,
            started: true,
            done: wdone.clone(),
            ident: Some(ident),
            native_id: Some(native),
        }))),
        addr,
        serial: my_serial.unwrap_or(-2),
        tid: std::sync::atomic::AtomicI64::new(native),
        done: wdone,
        daemon,
    });

    // E-safety: reclaim drained at owner-side safe points (pure drops,
    // zero registration traffic — unlike the sweep, cannot fire an
    // auto-collect mid-flight).
    drain_worker_reclaim();
    // E-safety: publish the worker-interp handle HERE (main side, GIL
    // held) — the clone is a GIL-ordered refcount write; workers only
    // read the Rc pointer under the GIL afterwards.
    *WORKER_INTERP.lock().unwrap() = Some(crate::gcsupport::GilSendPub(interp.clone()));
    // E1-E4 parity: pin the mirror while the thread runs — enumerate()/
    // active_count()/excepthook must see a RUNNING thread even when the
    // script dropped its reference (CPython's _active holds it). The
    // eviction sweep drops the pin the first sweep after done flips.
    mirror_pin(addr, &Value::Obj(o.clone()));

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
            let mut t = THREADS.lock().unwrap();
            t.entries.retain(|s| s.addr != addr);
            t.rebuild_indexes();
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
    // WORKER_INTERP is (re)installed by the SPAWNER under the GIL (see
    // thread_start): a worker-side interp.clone() is off-lock Rc traffic.
    crate::gcsupport::install_worker_thread();
    // E-safety: the identity stamp is deferred to AFTER acquire_gil
    // (below) — pre-GIL it cloned/dropped entry Rc's, racing main-side
    // mutation (the stress-probe UB class; baseline had the same hole).
    // Nothing between here and acquire_gil touches Rc counts.

    // Timer semantics: wait BEFORE the target (finished.wait(interval)).
    if let Some(dur) = interval {
        // E-safety: read the serial from the SLOT MIRROR under the table
        // lock — entry_for() clones an Rc, and pre-GIL Rc traffic is the
        // stress-probe UB class (see WORKER_RECLAIM).
        let my_serial = {
            let t = THREADS.lock().unwrap();
            t.by_addr
                .get(&self_addr)
                .and_then(|v| v.last())
                .and_then(|&i| t.entries.get(i))
                .map(|s| s.serial)
        }
        .unwrap_or(-2);
        wait_cancellable(Duration::from_secs_f64(dur.max(0.0)), self_addr, my_serial);
        let cancelled = timer_cancelled(self_addr, my_serial);
        if cancelled {
            acquire_gil();
            stamp_worker_identity(self_addr);
            mark_done(self_addr);
            // E-safety: move the Rc payloads to the main thread's reclaim
            // queue instead of dropping them at the post-release_gil tail
            // (see WORKER_RECLAIM).
            reclaim_worker_payload((interp, target, args, kwargs));
            release_gil();
            return;
        }
    }

    acquire_gil();
    // E-safety: the TLS interp handles (each holds an Rc clone) install
    // AFTER the GIL is held — their strong_count writes then serialize
    // with every main-side mutation (baseline installed them pre-GIL).
    crate::builtins_modules::set_current_interp(&interp);
    crate::interp::install_current_interp(&interp);
    // Stamp THIS OS thread's identity into its registry entry (the spawner
    // could only record a placeholder — the worker tid exists only here).
    // GIL-held now: the entry Rc clone/drop inside is serialized like
    // every other graph mutation.
    stamp_worker_identity(self_addr);
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
    mark_done(self_addr);
    // E-safety: the tail payloads move to the main thread's reclaim queue
    // (dropping them here or after release_gil — the baseline pattern —
    // mutates shared non-atomic refcounts off-lock; see WORKER_RECLAIM).
    reclaim_worker_payload((interp, target, args, kwargs));
    release_gil();
}

/// E7: read the cancel state for a Timer addr (post-wait confirmation).
fn timer_cancelled(addr: usize, serial: i64) -> bool {
    match CANCELLED.lock().unwrap().get(&(addr, serial)) {
        Some(ev) => *ev.cancelled.lock().unwrap(),
        None => false,
    }
}

/// Sleep honoring Timer.cancel — E7 (memperf audit 2026-11-06): the old
/// shape locked the GLOBAL CANCELLED map once per 1 ms slice (60 s Timer =
/// 60k lock acquisitions + wakeups). Now the worker fetches the per-addr
/// CancelEvent once and parks on its Condvar with the remaining time as the
/// wait timeout: zero lock traffic while sleeping, one wakeup at cancel or
/// deadline.
fn wait_cancellable(dur: Duration, self_addr: usize, serial: i64) {
    let ev = CANCELLED.lock().unwrap().get(&(self_addr, serial)).cloned();
    let Some(ev) = ev else {
        // No event registered (cancel() never touched this Timer): plain
        // sleep for the whole interval.
        std::thread::sleep(dur);
        return;
    };
    let deadline = Instant::now() + dur;
    let mut g = ev.cancelled.lock().unwrap();
    while !*g {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        let (ng, res) = ev.cv.wait_timeout(g, left).unwrap();
        g = ng;
        if res.timed_out() {
            return;
        }
    }
}

// join signalling
static JOIN_CV: Condvar = Condvar::new();
static JOIN_MU: Mutex<()> = Mutex::new(());

/// Block the CALLER (GIL must be RELEASED before calling us) until `done`
/// flips. Returns true when the thread finished, false on timeout.
///
/// E7 (memperf audit 2026-11-06): the old shape polled the entry every
/// 200us while JOIN_CV/JOIN_MU (notified by every worker that marks done)
/// were never awaited — pure spin. Now: check under JOIN_MU, park on the
/// Condvar with a done predicate; a timeout join bounds the wait with
/// wait_timeout. The done flag itself lives in a RefCell the notifier
/// mutates under the GIL, so the predicate re-reads it on every wake; the
/// tiny race (mark done -> notify) is covered by the initial check inside
/// the lock and by the notify-while-notified protocol on JOIN_CV.
pub fn join_wait(entry: Rc<RefCell<ThreadEntry>>, timeout: Option<f64>) -> bool {
    join_wait_flag(entry_done_flag(&entry), timeout)
}

/// Resolve an entry's shared done flag. Call while the GIL is HELD (the
/// joiner does exactly that before releasing) — the borrow is then
/// uncontended, and the returned Arc<AtomicBool> lets the parked joiner
/// observe completion with atomics ONLY (no borrow-flag reads, no
/// non-atomic refcount traffic off-lock).
pub fn entry_done_flag(entry: &Rc<RefCell<ThreadEntry>>) -> Arc<std::sync::atomic::AtomicBool> {
    match entry.try_borrow() {
        Ok(b) => b.done.clone(),
        // Contended (only reachable from a non-GIL caller): the done flag
        // is process-stable once set; fall back to a bounded poll then an
        // atomically-seeded fresh flag — the worker's JOIN_CV notify still
        // wakes a waiter parked on a false snapshot (the loop re-checks).
        Err(_) => {
            for _ in 0..2000 {
                if let Ok(b) = entry.try_borrow() {
                    return b.done.clone();
                }
                std::thread::sleep(Duration::from_micros(50));
            }
            Arc::new(std::sync::atomic::AtomicBool::new(entry_done(entry)))
        }
    }
}

/// E7/E-safety (memperf audit 2026-11-06): condvar join on the done
/// PREDICATE — the flag is an Arc<AtomicBool> (atomics are safe to read
/// while parked; the baseline polled the entry's RefCell every 200us,
/// racing the borrow flag against worker borrow_mut). Callers grab the
/// flag BEFORE releasing the GIL and park here.
pub fn join_wait_flag(flag: Arc<std::sync::atomic::AtomicBool>, timeout: Option<f64>) -> bool {
    if flag.load(std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    let deadline = timeout.map(|t| Instant::now() + Duration::from_secs_f64(t.max(0.0)));
    let mut guard = JOIN_MU.lock().unwrap();
    loop {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            return true;
        }
        match deadline {
            None => {
                guard = JOIN_CV.wait(guard).unwrap();
            }
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return flag.load(std::sync::atomic::Ordering::SeqCst);
                }
                let (g, res) = JOIN_CV.wait_timeout(guard, left).unwrap();
                guard = g;
                if res.timed_out() {
                    return flag.load(std::sync::atomic::Ordering::SeqCst);
                }
            }
        }
    }
}

/// Wait for ALL non-daemon threads to finish (main-thread exit protocol,
/// T107: a daemon parked forever cannot block process exit). The main
/// thread's own entry is marked done FIRST (it is the thread performing the
/// wait and must not join itself).
pub fn wait_for_non_daemons() {
    // E-safety: mark main done via the SLOT atom (no entry borrow).
    mark_done_main();
    loop {
        // E-safety: collect FLAGS ONLY (Arc — atomic refcount, safe to
        // clone while parked). The baseline cloned entry Rc's here without
        // the GIL while workers cloned/dropped the same pointers under it
        // (non-atomic strong_count race — the stress-probe UB class).
        let pending: Vec<Arc<std::sync::atomic::AtomicBool>> = {
            let t = THREADS.lock().unwrap();
            t.entries
                .iter()
                .filter(|s| !s.daemon && !s.done.load(std::sync::atomic::Ordering::SeqCst))
                .map(|s| s.done.clone())
                .collect()
        };
        if pending.is_empty() {
            break;
        }
        for p in pending {
            let _ = join_wait_flag(p, None);
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
    // M2: O(1) native-tid index (was a linear scan).
    let t = THREADS.lock().unwrap();
    t.by_tid
        .get(&tid)
        .and_then(|v| v.last())
        .and_then(|&i| t.entries.get(i))
        .map(|s| s.entry.0.clone())
}

fn thread_value_for_entry(e: Option<Rc<RefCell<ThreadEntry>>>) -> Value {
    // The obj Value for an entry: look the obj up by addr in the interp's
    // reach — we stored the addr only; reconstruct the Value handle via the
    // obj-table. Entries store their obj handle at construction (thread_start
    // holds it via the attrs of nothing) — simplest: MAIN side keeps a
    // mirror table addr -> Value.
    mirror_get(e.map(|x| x.borrow().obj_addr).unwrap_or(0)).unwrap_or(Value::None)
}

/// M3 (memperf audit 2026-11-06): addr -> Thread/Timer PyObject mirror. The
/// table used to hold STRONG Value clones: every constructed Thread/Timer
/// (three mirror_put sites) pinned its whole object graph (<target>/<args>/
/// <kwargs> payloads included) for the process lifetime — the registry clone
/// is subtracted by the collector's reachability seeding, so a mirror clone
/// makes the object externally reachable forever and the addr is never even
/// recycled. Now: Weak handles by default (the audit's named fix shape) with
/// a STRONG pin installed only while the thread is RUNNING — CPython's
/// `_active` holds the live Thread object exactly this way — released once
/// the thread finishes, and the row pruned when the mirror goes dead.
///
/// GIL protocol: every upgrade happens while the caller holds the GIL (the
/// worker takes it before touching the registry paths that read mirrors), so
/// a Weak upgrade never races an Rc release on another thread.
pub struct MirrorRow {
    /// GilSendPub: the audited GIL-protocol wrapper (the table is only ever
    /// touched under the interpreter lock; same contract as the Value
    /// mirrors this replaced).
    weak: crate::gcsupport::GilSendPub<std::rc::Weak<PyObject>>,
    pinned: Option<crate::gcsupport::GilSendPub<Value>>,
}

static OBJ_MIRROR: std::sync::LazyLock<Mutex<HashMap<usize, MirrorRow>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn mirror_put(addr: usize, v: &Value) {
    if let Value::Obj(o) = v {
        OBJ_MIRROR.lock().unwrap().insert(
            addr,
            MirrorRow {
                weak: crate::gcsupport::GilSendPub(Rc::downgrade(o)),
                pinned: None,
            },
        );
    }
}

/// Install the running-thread strong pin on a mirror row (CPython's
/// `_active` holds the live Thread object exactly this way; the eviction
/// sweep releases the pin once the thread is done).
fn mirror_pin(addr: usize, v: &Value) {
    if let Value::Obj(o) = v {
        let mut m = OBJ_MIRROR.lock().unwrap();
        m.entry(addr)
            .and_modify(|row| row.pinned = Some(crate::gcsupport::GilSendPub(v.clone())))
            .or_insert_with(|| MirrorRow {
                weak: crate::gcsupport::GilSendPub(Rc::downgrade(o)),
                pinned: Some(crate::gcsupport::GilSendPub(v.clone())),
            });
    }
}

fn mirror_get(addr: usize) -> Option<Value> {
    let m = OBJ_MIRROR.lock().unwrap();
    match m.get(&addr) {
        Some(row) => row
            .pinned
            .as_ref()
            .map(|p| p.0.clone())
            .or_else(|| row.weak.0.upgrade().map(Value::Obj)),
        None => None,
    }
}

/// M3: True when the mirror for `addr` upgrades to a live object carrying
/// `serial` (the serial check blocks addr-recycling aliasing: a Thread
/// object dead + its slot reused by a fresh Thread must not resurrect the
/// old entry).
fn mirror_alive_with_serial(addr: usize, serial: i64) -> bool {
    let row = OBJ_MIRROR
        .lock()
        .unwrap()
        .get(&addr)
        .map(|r| (r.weak.clone(), r.pinned.as_ref().map(|p| p.0.clone())));
    let obj = match row {
        Some((_, Some(Value::Obj(o)))) => Some(o),
        Some((_, Some(_))) | None => None,
        Some((w, None)) => w.0.upgrade(),
    };
    match obj {
        Some(o) => {
            o.attrs
                .borrow()
                .get(&s("<serial>"))
                .and_then(|v| v.as_int())
                .and_then(|i| i.to_i64())
                == Some(serial)
        }
        None => false,
    }
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

/// E7 (memperf audit 2026-11-06): bounded retry budget for the attrs
/// probe — the old 2000 x 50us = 100 ms per contended read (attr_str/
/// attr_bool run 3x per thread start). 200 x 50us = 10 ms caps a pathological
/// contention without ever hanging the GIL, then degrades to the empty/false
/// default exactly like the old exhausted loop did.
const ATTR_PROBE_RETRIES: usize = 200;

fn attr_str(o: &PyObject, key: &str) -> String {
    // A second OS thread can momentarily hold this RefCell (make_thread_value
    // constructs under a single borrow). We hold the GIL here, so the other
    // holder must be a brief native-only span; retry instead of panicking.
    for _ in 0..ATTR_PROBE_RETRIES {
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
    for _ in 0..ATTR_PROBE_RETRIES {
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
                let new_addr = Rc::as_ptr(o).addr();
                let old_addr = e.borrow().obj_addr;
                mirror_put(new_addr, &v);
                e.borrow_mut().obj_addr = new_addr;
                if old_addr != new_addr {
                    // M2: the addr mirror + by_addr index must follow the
                    // lazy bind (once per process; full rebuild is fine).
                    let _ = old_addr;
                    THREADS.lock().unwrap().reindex_addr();
                }
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
        .entries
        .iter()
        .filter(|s| {
            let b = s.entry.0.borrow();
            b.started && !b.done.load(std::sync::atomic::Ordering::SeqCst)
        })
        .count()
}

pub fn enumerate_values() -> Vec<Value> {
    let addrs: Vec<usize> = THREADS
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|s| {
            let b = s.entry.0.borrow();
            b.started && !b.done.load(std::sync::atomic::Ordering::SeqCst)
        })
        .map(|s| s.addr)
        .collect();
    addrs.into_iter().filter_map(mirror_get).collect()
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

// ===========================================================================
// E1-E4 (memperf audit 2026-11-06): the shared eviction sweep.
//
// The six process-lifetime tables (LOCALS / THREADS / OBJ_MIRROR / PRIMS /
// CANCELLED / OWN_RLOCKS) were all insert-only with zero removal sites —
// every threading.local dict, every Thread/Timer ever constructed (with its
// whole <target>/<args>/<kwargs> graph), and every Lock/Queue primitive
// (including QueuePrim.items payloads) was pinned forever. ONE sweep fixes
// all of them: the orchestrator wires `evict_stale_tables()` into the
// major-GC drain hook (gcsupport.rs, next to drain_worker_registrations).
//
// Liveness oracle: `ap_gc::get_object_by_addr(addr)` — the allocation
// registry is keyed by exactly the Rc payload address, and its clone keeps
// the count >= 1 while the object is reachable (so address presence == GC
// liveness; a collected object's slot only ever re-appears with a NEW
// object, which the serial checks below guard for identity-carrying rows).
// Callers must hold the GIL: worker threads never take these table locks
// except stamp_worker_identity (documented GilSendPub unsafety), and the
// sweep uses try_borrow so a contended entry is conservatively kept.
// ===========================================================================

pub fn evict_stale_tables() -> usize {
    // The GC registry is a MAIN-thread thread_local (workers forward their
    // allocations through the registration channel); the liveness oracle is
    // only meaningful on the main thread's view. A non-main caller must not
    // even drain (that would steal registrations from the main registry).
    if !is_main_os_thread() {
        return 0;
    }
    // E-safety: drop reclaimed worker payloads FIRST (main thread, GIL
    // held — the only safe place for these Rc clones to die).
    drain_worker_reclaim();
    // Worker-allocated objects live in the forward channel until drained —
    // sweep after a drain or a live thread-local dict reads "absent".
    let drained = crate::gcsupport::drain_worker_registrations();
    let mut freed = 0usize;

    // ---- THREADS entries + OBJ_MIRROR pins (three short phases; NEVER
    // holds THREADS and OBJ_MIRROR at once) ----
    let mut dropped_entries = 0usize;
    let mut live_tids: Vec<i64> = Vec::new();
    let mut pin_ops: Vec<(usize, bool)> = Vec::new(); // (addr, want_pin)
    let snaps: Vec<(usize, i64, Option<bool>, i64)> = {
        // Phase 1: snapshot (addr, serial, done?) + main-thread tids.
        let t = THREADS.lock().unwrap();
        t.entries
            .iter()
            .map(|s| {
                (
                    s.addr,
                    s.serial,
                    // E-safety: read the SLOT atom — no entry borrow, so a
                    // peer's stamp window can never skew the snapshot.
                    Some(s.done.load(std::sync::atomic::Ordering::SeqCst)),
                    s.tid.load(std::sync::atomic::Ordering::Relaxed),
                )
            })
            .collect::<Vec<_>>()
        // (lock released before any mirror query)
    };
    // Phase 2: liveness decisions outside all table locks.
    let mut drop_idx: Vec<usize> = Vec::new();
    for (idx, (addr, serial, done, tid)) in snaps.iter().enumerate() {
        if *serial == -1 {
            // MainThread: permanent (the exit protocol marks it done;
            // eviction must never drop it). tid mirror == real main tid.
            live_tids.push(*tid);
            pin_ops.push((*addr, true));
            continue;
        }
        let effective = match done {
            Some(d) => *d,
            // Contended borrow: a worker is mid-stamp/mid-done. Keep + pin
            // this sweep (conservative; the next sweep sees it settled).
            None => {
                live_tids.push(*tid);
                pin_ops.push((*addr, true));
                continue;
            }
        };
        // The audit's retention rule: drop iff done AND the mirror is
        // absent/dead (nobody can hold the Thread object -> nobody can
        // join/is_alive/repr it). A live object (even finished, not yet
        // joined) keeps its entry: post-join repr reads "stopped", and a
        // re-start of the same object must still find its entry.
        let mirror_live = mirror_alive_with_serial(*addr, *serial);
        if effective && !mirror_live {
            drop_idx.push(idx);
            freed += 1;
        } else {
            pin_ops.push((*addr, !effective));
            live_tids.push(*tid);
        }
    }
    // Phase 3: apply drops by index (no slot shifted during the sweep —
    // stamp_worker_identity only touches the tid atomics/indexes).
    if !drop_idx.is_empty() {
        let mut t = THREADS.lock().unwrap();
        let before = t.entries.len();
        let mut i = 0usize;
        t.entries.retain(|_| {
            let keep = !drop_idx.contains(&i);
            i += 1;
            keep
        });
        dropped_entries = before - t.entries.len();
        t.rebuild_indexes();
    }
    // Mirror pin management (single pass over OBJ_MIRROR).
    {
        let mut m = OBJ_MIRROR.lock().unwrap();
        for (addr, want_pin) in pin_ops {
            if let Some(row) = m.get_mut(&addr) {
                if want_pin && row.pinned.is_none() {
                    if let Some(o) = row.weak.0.upgrade() {
                        row.pinned = Some(crate::gcsupport::GilSendPub(Value::Obj(o)));
                    }
                } else if !want_pin {
                    if row.pinned.take().is_some() {
                        freed += 1;
                    }
                }
            }
        }
    }

    // ---- OBJ_MIRROR rows: dead weak + not pinned ----
    {
        let mut m = OBJ_MIRROR.lock().unwrap();
        let before = m.len();
        m.retain(|_, row| row.pinned.is_some() || row.weak.0.upgrade().is_some());
        freed += before - m.len();
    }

    // ---- LOCALS: outer keys absent from the registry; inner rows whose
    // owner tid is no longer in the thread table (joined/dead thread) ----
    {
        let mut l = LOCALS.lock().unwrap();
        let before = l.len();
        l.retain(|addr, _| ap_gc::get_object_by_addr(*addr).is_some());
        freed += before - l.len();
        let tid_set: std::collections::HashSet<i64> = live_tids.into_iter().collect();
        let before_inner: usize = l.values().map(|v| v.len()).sum();
        for rows in l.values_mut() {
            rows.retain(|tid, _| tid_set.contains(&(*tid as i64)));
        }
        let after_inner: usize = l.values().map(|v| v.len()).sum();
        freed += before_inner - after_inner;
        l.retain(|_, rows| !rows.is_empty());
    }

    // ---- PRIMS (+OWN_RLOCKS): phantom (cond+1) keys live off the
    // Condition object; plain keys live off the object at that exact addr --
    {
        let owned: std::collections::HashSet<usize> =
            OWN_RLOCKS.lock().unwrap().iter().copied().collect();
        let mut p = PRIMS.lock().unwrap();
        let before = p.len();
        let dbg = std::env::var("APX_TDEBUG").is_ok();
        p.retain(|addr, _| {
            let keep_it = if owned.contains(addr) {
                // cond addr = key - 1
                ap_gc::get_object_by_addr(addr.saturating_sub(1)).is_some()
            } else {
                ap_gc::get_object_by_addr(*addr).is_some()
            };
            let _ = dbg;
            keep_it
        });
        let mut or = OWN_RLOCKS.lock().unwrap();
        let live_keys: std::collections::HashSet<usize> = p.keys().copied().collect();
        let before2 = or.len();
        or.retain(|k| live_keys.contains(k));
        freed += before2 - or.len();
    }

    // ---- lxml E5 side-table: compiled-XPath rows whose marker died ----
    freed += crate::lxml::prune_xpath_registry();

    // ---- CANCELLED: rows whose Timer object is gone (mirror dead AND not
    // pinned by a running thread). A waiter holds its Arc clone, so pruning
    // the map row cannot break an in-flight wait. ----
    {
        let mut c = CANCELLED.lock().unwrap();
        let before = c.len();
        c.retain(|(addr, serial), _| {
            mirror_alive_with_serial(*addr, *serial) || ap_gc::get_object_by_addr(*addr).is_some()
        });
        freed += before - c.len();
    }

    let _ = drained;
    freed + dropped_entries
}

/// Test/diagnostic hook: total table occupancy across the six sweeps (the
/// eviction fence asserts these shrink after joins + collection).
pub fn table_occupancy() -> (usize, usize, usize, usize, usize, usize) {
    (
        LOCALS.lock().unwrap().len(),
        THREADS.lock().unwrap().entries.len(),
        OBJ_MIRROR.lock().unwrap().len(),
        PRIMS.lock().unwrap().len(),
        CANCELLED.lock().unwrap().len(),
        OWN_RLOCKS.lock().unwrap().len(),
    )
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

/// Timer.cancel with the Timer object's construction serial (addr-only keys
/// alias across recycled addresses — see CANCELLED).
pub fn cancel_timer_serial(addr: usize, serial: i64) {
    // E7: set the per-Timer event + notify (the worker is parked on it);
    // if no event exists yet (cancel before start), mint one pre-cancelled
    // so the waiter sees it immediately on its first fetch.
    let mut m = CANCELLED.lock().unwrap();
    match m.get(&(addr, serial)) {
        Some(ev) => {
            *ev.cancelled.lock().unwrap() = true;
            ev.cv.notify_all();
        }
        None => {
            let ev = Arc::new(CancelEvent {
                cancelled: Mutex::new(true),
                cv: Condvar::new(),
            });
            m.insert((addr, serial), ev);
        }
    }
}

pub fn cancel_timer(addr: usize) {
    // Addr-only convenience: resolve the serial from the live object mirror
    // (the <serial> attr is minted at construction, so a NOT-yet-started
    // Timer cancel lands on the same (addr, serial) key thread_start will
    // register — cancel-before-start suppresses the fire, CPython parity).
    let serial = match mirror_get(addr) {
        Some(Value::Obj(o)) => o
            .attrs
            .borrow()
            .get(&s("<serial>"))
            .and_then(|v| v.as_int())
            .and_then(|i| i.to_i64())
            .unwrap_or(-2),
        _ => entry_for(addr).map(|e| e.borrow().serial).unwrap_or(-2),
    };
    cancel_timer_serial(addr, serial);
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

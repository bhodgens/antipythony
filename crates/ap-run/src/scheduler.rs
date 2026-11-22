//! 02.10 stages 2b/2c — the Go-style task scheduler: GMP *accounting*
//! (permits, run queues, steal-at-birth, park tokens) over explicit
//! scheduler-side Task records whose execution state is a corosensei fiber.
//!
//! TaskFiber = `Coroutine<SchedulerIn, SchedulerOut, (), PooledStack>` — the
//! SAME machinery the landed 0210a generator fibers use (generators.rs). A
//! `threading.Thread` target runs INSIDE such a fiber: the fiber's Rust
//! stack carries the body (locals, call depth, try/except), and the
//! statement-granularity safepoint poll (`maybe_poll`, fused ahead of
//! touch_line in exec_stmt) suspends the fiber at a statement boundary when
//! the task's quantum expires — the fiber unwinds into the worker loop and
//! is resumed there later. Nested generator fibers created inside the task
//! body are untouched (their yielders live in the generators module's own
//! table, keyed by generator registry ids; the task fiber's yielder lives
//! HERE, keyed by task id — disjoint id spaces).
//!
//! Scheduling rules (M1, load-bearing):
//! - steal-at-birth: only UNSTARTED tasks sit in run queues; a started
//!   task's fiber is !Send and lives ONLY in its home worker's thread-local
//!   READY deque. Zero unsafe beyond the audited yielder-pointer pattern
//!   generators.rs already uses.
//! - quantum: `APX_GOMAXPROCS`/available_parallelism workers, each draining
//!   its own suspended-task deque, then its submission queue (stealing half
//!   a peer's queue when empty), round-robin.
//! - compensation lane (2c.4): a task blocked in a primitive Condvar wait
//!   keeps its worker (the fiber cannot unwind mid-native-wait), so prim
//!   wait sites take a `ParkGuard` — when every worker is parked and
//!   unstarted tasks remain queued, temporary workers are spawned up to
//!   APX_COMP_CAP (default P×4) and idle-exit when the burst drains.
//!   Without this, Barrier(parties) with parties > P deadlocks (T113 class).
//! - APX_MAX_STARTED (2c.5): admission backstop bounding simultaneously
//!   STARTED fibers (each an 8 MiB PooledStack reservation, ~330 KB touched
//!   — generators.rs sizing). Unstarted tasks cost ~hundreds of bytes.
//!
//! TLS discipline: `current_logical_ident()` is the per-task identity for
//! threading.local namespaces and RLock ownership (the os-tid key is the
//! WORKER's identity — two tasks share one worker sequentially, and a
//! worker-shared key would silently merge their namespaces). The main
//! thread (no task mounted) falls back to its os tid exactly as before.
//! Payload drop discipline follows the audited E-safety protocol: tail
//! payloads move to the reclaim queue and die on the main thread.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::frame::Interpreter;
use ap_obj::value::Value;

use corosensei::{Coroutine, CoroutineResult};

// ---------------------------------------------------------------------------
// The safepoint word (2b.2): ONE process-global AtomicU64. Bit 0 = STW
// requested (the future MMTk collector leaf sets it; never set in this
// leaf), bit 1 = global quantum broadcast (reserved). Every statement poll
// is one Relaxed load here.
// ---------------------------------------------------------------------------

pub static SAFEPOINT: AtomicU64 = AtomicU64::new(0);

pub const SAFEPPOINT_STW: u64 = 1;
pub const SAFEPPOINT_QUANTUM: u64 = 2;

// ---------------------------------------------------------------------------
// Per-worker mounted-task state
// ---------------------------------------------------------------------------

thread_local! {
    /// The task mounted on THIS OS thread right now (0 = none). Only the
    /// home worker writes it, only while the task executes there.
    static CURRENT_TASK: Cell<u64> = const { Cell::new(0) };
    /// The mounted task's LOGICAL thread ident (TLS / RLock-owner key).
    static CURRENT_IDENT: Cell<i64> = const { Cell::new(0) };
    /// Statements left in this task's quantum. u64::MAX = disarmed (no task).
    static STMT_BUDGET: Cell<u64> = const { Cell::new(u64::MAX) };
    /// Home-worker suspended-task deque: (task record, live fiber). !Send —
    /// never leaves this thread (started tasks never migrate).
    static READY: RefCell<VecDeque<(Arc<Task>, TaskFiber)>> =
        const { RefCell::new(VecDeque::new()) };
    /// This worker's run-queue index (queue affinity for submission).
    static MY_QUEUE: Cell<usize> = const { Cell::new(0) };
    /// True for temporary compensation workers (idle-exit protocol).
    static IS_TEMP: Cell<bool> = const { Cell::new(false) };
}

pub fn current_task_id() -> u64 {
    CURRENT_TASK.with(|c| c.get())
}

/// The logical thread ident of the task running on THIS OS thread; 0 when no
/// task is mounted (main thread / worker between tasks). Callers key
/// per-Python-thread state (LOCALS namespaces, RLock owners) on this, with
/// an os-tid fallback for the main thread.
pub fn current_logical_ident() -> i64 {
    CURRENT_IDENT.with(|c| c.get())
}

fn set_mounted(task_id: u64, ident: i64) {
    CURRENT_TASK.with(|c| c.set(task_id));
    CURRENT_IDENT.with(|c| c.set(ident));
}

/// Mount the MAIN thread's logical ident (bootstrap_main): module-body code
/// keys TLS/RLock ownership on a real ident like any task, but no task
/// fiber is mounted (quantum polls stay disarmed).
pub fn mount_main_ident(ident: i64) {
    CURRENT_IDENT.with(|c| c.set(ident));
}

/// True when THIS OS thread is a scheduler pool worker (permanent or
/// temporary). Used by worker_bootstrap to decide whether its end-of-body
/// census uninstall may run (per-task uninstall would leak rows on a
/// long-lived worker).
pub fn is_pool_worker() -> bool {
    POOL.get().is_some()
}

// ---------------------------------------------------------------------------
// Quantum accounting
// ---------------------------------------------------------------------------

fn switch_interval() -> Duration {
    let secs = f64::from_bits(crate::threading::switch_interval_bits());
    if secs > 0.0 {
        Duration::from_secs_f64(secs.clamp(1e-6, 3600.0))
    } else {
        // CPython's sys.getswitchinterval() default: 5 ms.
        Duration::from_secs_f64(0.005)
    }
}

/// A tier-0 statement costs ~µs, so the quantum in statement units is
/// interval_ns / 1000 (impl doc §3.1 arithmetic). One decrement + compare
/// per statement; re-armed at mount, at quantum expiry, and after every
/// resume.
fn arm_budget() {
    let q = (switch_interval().as_nanos() / 1000).clamp(16, u64::MAX as u128) as u64;
    STMT_BUDGET.with(|b| b.set(q));
}

fn disarm_budget() {
    STMT_BUDGET.with(|b| b.set(u64::MAX));
}

/// True when a task fiber is mounted on this thread AND has a stashed
/// yielder (i.e. a quantum yield can actually suspend).
fn can_yield() -> bool {
    current_task_id() != 0 && !task_yielder_ptr(current_task_id()).is_null()
}

/// The statement poll (impl doc §3.1, P1): called fused AHEAD of touch_line
/// in exec_stmt so a suspended task never consumes a line ordinal for a
/// statement it did not execute (N2 invariant). One Relaxed SAFEPOINT load
/// (the future MMTk STW handshake consumes bit 0 of the same word — dead
/// load until that leaf lands) plus one Cell decrement. On quantum expiry
/// the mounted task fiber suspends INTO the worker loop here — at a
/// statement boundary, where no RefCell borrow guard and no prim Mutex is
/// held (the safepoint-latency invariant: bounded by one statement).
#[inline]
pub fn maybe_poll() {
    if SAFEPOINT.load(Ordering::Relaxed) & SAFEPPOINT_STW != 0 {
        // STW requested (reserved for the gc leaf; never set in this leaf).
        // The wait protocol is that leaf's; nothing to acknowledge yet.
    }
    let left = STMT_BUDGET.with(|b| {
        let v = b.get();
        if v != u64::MAX && v > 0 {
            b.set(v - 1);
        }
        v
    });
    if left <= 1 && can_yield() {
        quantum_yield();
    }
}

/// Suspend the mounted task fiber into its home worker's loop; the worker
/// requeues the task round-robin and resumes it (or runs a peer) with a
/// fresh quantum. Defensive no-op when no yielder is stashed.
fn quantum_yield() {
    let tid = current_task_id();
    let y = task_yielder_ptr(tid);
    if y.is_null() {
        arm_budget();
        return;
    }
    sched_trace(&format!("task={tid} quantum-yield"));
    // SAFETY: the yielder lives in the suspended coroutine's initial frame;
    // the Coroutine stays in THIS worker's READY deque for the whole
    // suspension window (started tasks never migrate). Same audited pattern
    // as generators.rs gen_yield.
    let resumed = unsafe { (*y).suspend(SchedulerOut::YieldScheduler) };
    let _ = resumed;
    // Fresh quantum for the resumed slice.
    arm_budget();
}

// ---------------------------------------------------------------------------
// Task record (impl doc §2, trimmed to what the landed shape needs)
// ---------------------------------------------------------------------------

pub type Ident = i64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskState {
    /// In a run queue, fiber never started (stealable at birth).
    Unstarted = 0,
    /// Fiber started; lives in its home worker's READY deque.
    Running = 1,
    /// Body finished; done flag flipped. Registry row kept until eviction.
    Done = 2,
}

/// What a task runs: exactly the payload `worker_bootstrap` consumes, plus
/// the interpreter handle (Send-staged per the 02.09a marker contract — the
/// pre-scheduler worker closure carried the same captures).
pub struct TaskSpec {
    pub interp: Arc<Interpreter>,
    pub target: Value,
    pub args: Vec<Value>,
    pub kwargs: Vec<(String, Value)>,
    pub addr: usize,
    pub serial: i64,
    pub daemon: bool,
    pub timer_interval: Option<f64>,
}

pub struct Task {
    pub id: u64,
    pub ident: Ident,
    pub name: String,
    pub state: std::sync::atomic::AtomicU8,
    pub spec: TaskSpec,
    pub done: Arc<std::sync::atomic::AtomicBool>,
    pub park_seq: AtomicU64,
}

impl Task {
    pub fn state_of(&self) -> TaskState {
        match self.state.load(Ordering::Relaxed) {
            1 => TaskState::Running,
            2 => TaskState::Done,
            _ => TaskState::Unstarted,
        }
    }
}

static NEXT_TASK: AtomicU64 = AtomicU64::new(1);

// ---------------------------------------------------------------------------
// Fiber protocol types
// ---------------------------------------------------------------------------

/// Input handed INTO the task fiber on (re)entry (consumed by the closure on
/// the first resume; later resumes surface as the suspend() return inside
/// quantum_yield, which ignores it).
pub enum SchedulerIn {
    Start,
    Resume,
}

/// Output handed back OUT at a suspension.
pub enum SchedulerOut {
    /// Quantum expired: state stays Running{home}, fiber parked in READY.
    YieldScheduler,
}

type TaskYielder = corosensei::Yielder<SchedulerIn, SchedulerOut>;
pub type TaskFiber = Coroutine<SchedulerIn, SchedulerOut, (), PooledStack>;

/// task id -> yielder raw pointer, valid while the body is executing (the
/// pointer lives in the coroutine's initial frame; the Coroutine object is
/// owned by the home worker's READY deque for the whole suspension window).
/// Process-global because the stash is written inside the fiber body and
/// read at arbitrary interp depth on the same thread — a table keyed by
/// task id, exactly the generators.rs YIELDERS pattern.
static TASK_YIELDERS: OnceLock<Mutex<HashMap<u64, usize>>> = OnceLock::new();

fn yielders() -> &'static Mutex<HashMap<u64, usize>> {
    TASK_YIELDERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn task_yielder_ptr(task_id: u64) -> *const TaskYielder {
    let p = yielders()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&task_id)
        .copied()
        .unwrap_or(0);
    p as *const TaskYielder
}

fn stash_task_yielder(task_id: u64, y: &TaskYielder) {
    yielders()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(task_id, y as *const TaskYielder as usize);
}

fn unstash_task_yielder(task_id: u64) {
    yielders()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&task_id);
}

// ---------------------------------------------------------------------------
// Stack: reuse generators.rs PooledStack (size-classed thread-local free
// list, 8 MiB reservation / ~330 KB touched per fiber).
// ---------------------------------------------------------------------------

use crate::generators::{gen_stack_size, PooledStack};

// ---------------------------------------------------------------------------
// RunQueue + pool
// ---------------------------------------------------------------------------

pub struct RunQueue {
    inner: Mutex<VecDeque<Arc<Task>>>,
    pub available: Condvar,
}

impl RunQueue {
    fn new() -> Self {
        RunQueue {
            inner: Mutex::new(VecDeque::new()),
            available: Condvar::new(),
        }
    }
    fn push(&self, t: Arc<Task>) {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(t);
        self.available.notify_one();
    }
    fn pop(&self) -> Option<Arc<Task>> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
    }
    fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
    /// Steal HALF of a victim queue's unstarted tasks into OUR queue.
    fn steal_half_from(&self, victim: &RunQueue) -> usize {
        let mut v = victim.inner.lock().unwrap_or_else(|p| p.into_inner());
        let n = v.len();
        if n < 2 {
            return 0;
        }
        let take = n - n / 2;
        let mut mine = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        for _ in 0..take {
            if let Some(t) = v.pop_front() {
                mine.push_back(t);
            }
        }
        drop(mine);
        if take > 0 {
            victim.available.notify_one();
            self.available.notify_one();
        }
        take
    }
    fn drain_into(&self, dst: &RunQueue) -> usize {
        let mut v = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let n = v.len();
        for _ in 0..n {
            if let Some(t) = v.pop_front() {
                dst.push(t);
            }
        }
        n
    }
}

struct Pool {
    queues: Vec<RunQueue>,
    workers: usize,
}

static POOL: OnceLock<Pool> = OnceLock::new();
static WORKERS_P: AtomicUsize = AtomicUsize::new(0);
/// Compensation-lane accounting (2c.4): workers currently blocked in a
/// prim/native wait (ParkGuard alive) and temp workers alive.
static PARKED_WORKERS: AtomicUsize = AtomicUsize::new(0);
static TEMP_WORKERS: AtomicUsize = AtomicUsize::new(0);
static TEMP_CAP: AtomicUsize = AtomicUsize::new(0);
/// Admission backstop (2c.5): max simultaneously-STARTED fibers.
/// 0 = unlimited (default — measure first; the knob exists so the 10k gate
/// can go red→green without a code change).
static MAX_STARTED: AtomicUsize = AtomicUsize::new(0);
static STARTED_FIBERS: AtomicUsize = AtomicUsize::new(0);

fn temp_cap_default(p: usize) -> usize {
    std::env::var("APX_COMP_CAP")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or_else(|| p.saturating_mul(4).max(32))
}

fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let p = std::env::var("APX_GOMAXPROCS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v >= 1)
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            });
        let workers = p.max(1);
        WORKERS_P.store(workers, Ordering::Relaxed);
        TEMP_CAP.store(temp_cap_default(workers), Ordering::Relaxed);
        if let Ok(v) = std::env::var("APX_MAX_STARTED") {
            if let Ok(n) = v.parse::<usize>() {
                MAX_STARTED.store(n, Ordering::Relaxed);
            }
        }
        let queues: Vec<RunQueue> = (0..workers).map(|_| RunQueue::new()).collect();
        for idx in 0..workers {
            std::thread::Builder::new()
                .name(format!("ap-worker-{idx}"))
                .stack_size(64 << 20)
                .spawn(move || worker_main(idx, false))
                .expect("scheduler worker spawn");
        }
        sched_trace(&format!("pool up: P={workers}"));
        Pool { queues, workers }
    })
}

pub fn worker_count() -> usize {
    pool().workers
}

fn sched_trace(msg: &str) {
    if std::env::var("APX_SCHED_TRACE").is_ok() {
        eprintln!("[SCHED] {msg}");
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

fn worker_main(qidx: usize, temp: bool) {
    MY_QUEUE.with(|c| c.set(qidx));
    crate::gcsupport::install_worker_thread();
    if temp {
        TEMP_WORKERS.fetch_add(1, Ordering::Relaxed);
        sched_trace(&format!("temp worker up q{qidx}"));
    }
    let p = pool();
    loop {
        // 1. Our own suspended tasks first (round-robin fairness).
        let mine = READY.with(|r| r.borrow_mut().pop_front());
        if let Some((task, fiber)) = mine {
            run_slice(task, fiber, false);
            continue;
        }
        // 2. Unstarted tasks: our queue, then steal-half from peers.
        let mut task = p.queues[qidx].pop();
        if task.is_none() {
            for k in 1..p.workers {
                let victim = &p.queues[(qidx + k) % p.workers];
                if victim.len() >= 2 {
                    p.queues[qidx].steal_half_from(victim);
                }
            }
            task = p.queues[qidx].pop();
        }
        if let Some(task) = task {
            start_task(task);
            continue;
        }
        // 3. Idle: wait for a submission. Temp workers exit after ~1 s
        // idle, requeueing anything that raced in (tasks are unstarted and
        // freely movable — M1 constrains only STARTED tasks).
        let exit_at = temp.then(|| Instant::now() + Duration::from_secs(1));
        let mut g = p.queues[qidx]
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut exiting = false;
        loop {
            if let Some(t) = g.pop_front() {
                drop(g);
                start_task(t);
                break;
            }
            let left = match exit_at {
                Some(d) => d.saturating_duration_since(Instant::now()),
                None => Duration::from_millis(200),
            };
            if left.is_zero() {
                // Temp idle-exit: hand any raced-in tasks to worker 0.
                let leftover: Vec<Arc<Task>> = g.drain(..).collect();
                drop(g);
                for t in leftover {
                    p.queues[0].push(t);
                }
                TEMP_WORKERS.fetch_sub(1, Ordering::Relaxed);
                sched_trace(&format!("temp worker q{qidx} idle-exit"));
                crate::gcsupport::uninstall_worker_thread();
                exiting = true;
                break;
            }
            let (ng, _to) = p.queues[qidx]
                .available
                .wait_timeout(g, left)
                .unwrap_or_else(|e| e.into_inner());
            g = ng;
        }
        if exiting {
            return;
        }
    }
}

/// Create the fiber for an unstarted task and run its first slice.
fn start_task(task: Arc<Task>) {
    // Admission backstop (2c.5): bound simultaneously-started fibers. At
    // capacity, run our own suspended tasks while waiting (the started
    // count only falls when fibers finish), then admit.
    let cap = MAX_STARTED.load(Ordering::Relaxed);
    if cap > 0 {
        loop {
            let n = STARTED_FIBERS.load(Ordering::Acquire);
            if n < cap {
                if STARTED_FIBERS
                    .compare_exchange(n, n + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break;
                }
            } else {
                std::thread::sleep(Duration::from_micros(500));
                let mine = READY.with(|r| r.borrow_mut().pop_front());
                if let Some((t, f)) = mine {
                    run_slice(t, f, false);
                }
            }
        }
        sched_trace(&format!(
            "task={} started-admission {}/{}",
            task.id,
            STARTED_FIBERS.load(Ordering::Acquire),
            cap
        ));
    }
    task.state
        .store(TaskState::Running as u8, Ordering::Relaxed);
    let stack = match PooledStack::new(gen_stack_size()) {
        Ok(s) => s,
        Err(e) => {
            // Stack reservation failure: complete the registry protocol
            // loudly (done flip + joiner wake) like the old spawn-failure
            // rollback — a joiner must never hang on a task that never ran.
            eprintln!("[SCHED] task={} fiber stack failed: {e}", task.id);
            finalize_failed(task);
            return;
        }
    };
    let task_id = task.id;
    let ident = task.ident;
    let interp = task.spec.interp.clone();
    let target = task.spec.target.clone();
    let args = task.spec.args.clone();
    let kwargs = task.spec.kwargs.clone();
    let addr = task.spec.addr;
    let interval = task.spec.timer_interval;
    let fiber: TaskFiber =
        Coroutine::with_stack(stack, move |y: &TaskYielder, _in: SchedulerIn| {
            stash_task_yielder(task_id, y);
            set_mounted(task_id, ident);
            arm_budget();
            crate::threading::worker_bootstrap(interp, target, args, kwargs, addr, interval);
            disarm_budget();
            set_mounted(0, 0);
            unstash_task_yielder(task_id);
        });
    run_slice(task, fiber, true);
}

/// One scheduler slice: resume the fiber until it suspends (quantum) or
/// finishes. Panic-safe: corosensei propagates a body panic THROUGH resume
/// into the resumer — caught here so the WORKER survives and the registry
/// protocol completes (pre-scheduler, a panicking worker died with its done
/// flag never flipping and joiners hung forever).
fn run_slice(task: Arc<Task>, mut fiber: TaskFiber, first: bool) {
    arm_budget();
    let input = if first {
        SchedulerIn::Start
    } else {
        SchedulerIn::Resume
    };
    let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fiber.resume(input)));
    match resumed {
        Ok(CoroutineResult::Yield(SchedulerOut::YieldScheduler)) => {
            if fiber.done() {
                finalize_done(task, Some(fiber));
            } else {
                READY.with(|r| r.borrow_mut().push_back((task, fiber)));
            }
        }
        Ok(CoroutineResult::Return(())) => {
            finalize_done(task, Some(fiber));
        }
        Err(_panic) => {
            // The body panicked: the fiber is dead. Complete the registry
            // protocol the body would have run (done flip + wake joiners),
            // clean the worker-confined TLS, and keep the worker alive.
            eprintln!(
                "[SCHED] task={} panicked; completing registry protocol",
                task.id
            );
            disarm_budget();
            set_mounted(0, 0);
            unstash_task_yielder(task.id);
            finalize_done(task, Some(fiber));
        }
    }
}

/// Complete a task: release its admission slot, flip done, wake joiners,
/// and move the spec payloads to the main-thread reclaim queue (E-safety:
/// Arc payload drops never race a peer's mutation from a worker — the
/// audited protocol; Task.spec would otherwise drop on the worker when the
/// last Arc<Task> reference dies here).
fn finalize_done(task: Arc<Task>, _fiber: Option<TaskFiber>) {
    if MAX_STARTED.load(Ordering::Relaxed) > 0 {
        STARTED_FIBERS.fetch_sub(1, Ordering::Release);
    }
    task.state.store(TaskState::Done as u8, Ordering::Relaxed);
    crate::threading::reclaim_worker_payload_pub((
        task.spec.interp.clone(),
        task.spec.target.clone(),
        task.spec.args.clone(),
        task.spec.kwargs.clone(),
        task.name.clone(),
    ));
    crate::threading::notify_join_waiters();
    sched_trace(&format!("task={} complete", task.id));
}

/// A task whose fiber never started (stack reservation failure): complete
/// the registry protocol the body would have run so joins cannot hang.
fn finalize_failed(task: Arc<Task>) {
    if MAX_STARTED.load(Ordering::Relaxed) > 0 {
        STARTED_FIBERS.fetch_sub(1, Ordering::Release);
    }
    task.state.store(TaskState::Done as u8, Ordering::Relaxed);
    crate::threading::mark_done_pub(task.spec.addr);
    crate::threading::reclaim_worker_payload_pub((
        task.spec.interp.clone(),
        task.spec.target.clone(),
        task.spec.args.clone(),
        task.spec.kwargs.clone(),
        task.name.clone(),
    ));
    crate::threading::notify_join_waiters();
}

// ---------------------------------------------------------------------------
// Submission
// ---------------------------------------------------------------------------

/// Build a Task and submit it to the pool (called from thread_start with the
/// registry slot already installed). Returns the task id.
pub fn submit_task(ident: i64, name: String, spec: TaskSpec) -> u64 {
    let id = NEXT_TASK.fetch_add(1, Ordering::Relaxed);
    let task = Arc::new(Task {
        id,
        ident,
        name,
        state: std::sync::atomic::AtomicU8::new(TaskState::Unstarted as u8),
        spec,
        done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        park_seq: AtomicU64::new(0),
    });
    let p = pool();
    // Round-robin placement at birth; steal-half rebalances when a worker
    // drains its queue. Unstarted tasks are the ONLY stealable state (M1).
    let q = (id as usize) % p.workers;
    p.queues[q].push(task);
    sched_trace(&format!("task={id} submitted to q{q}"));
    id
}

// ---------------------------------------------------------------------------
// Compensation lane (2c.4): ParkGuard
// ---------------------------------------------------------------------------

/// Take at every prim/native blocking wait inside a task fiber. When ALL
/// pool workers are parked and unstarted tasks remain queued, spawn
/// temporary compensation workers (bounded by APX_COMP_CAP, default P×4) so
/// rendezvous shapes with parties > P cannot deadlock (T113 class). Drop
/// releases the parked count — Drop guarantees the count stays balanced on
/// timeout/error/notify paths alike.
pub struct ParkGuard {
    active: bool,
}

impl ParkGuard {
    pub fn new() -> ParkGuard {
        PARKED_WORKERS.fetch_add(1, Ordering::AcqRel);
        let parked = PARKED_WORKERS.load(Ordering::Acquire);
        let p = WORKERS_P.load(Ordering::Relaxed).max(1);
        let pending: usize = {
            let pool = pool();
            pool.queues.iter().map(|q| q.len()).sum()
        };
        if parked >= p && pending > 0 {
            let cap = TEMP_CAP.load(Ordering::Relaxed);
            let cur = TEMP_WORKERS.load(Ordering::Relaxed);
            if cur < cap {
                // Saturate toward the pending count: no point spawning more
                // temps than there are runnable tasks.
                let want = cur + pending.min(cap - cur);
                for i in cur..want {
                    let q = i % p;
                    let spawned = std::thread::Builder::new()
                        .name(format!("ap-temp-{i}"))
                        .stack_size(64 << 20)
                        .spawn(move || worker_main(q, true));
                    if spawned.is_err() {
                        break;
                    }
                }
                sched_trace(&format!(
                    "compensation: parked={parked}/{p} pending={pending} temps→{want}"
                ));
            }
        }
        ParkGuard { active: true }
    }
}

impl Default for ParkGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ParkGuard {
    fn drop(&mut self) {
        if self.active {
            PARKED_WORKERS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

// ---------------------------------------------------------------------------
// Park/unpark registration (2c.1): explicit per-prim waiter records.
//
// The lost-wakeup protocol (impl doc §4.4): the parker registers
// (task_id, park_seq) under the CALLER's prim state Mutex — the same mutex
// the waker's state mutation takes — then waits. The waker mutates state,
// drains the registered waiters under that same Mutex (no parker can slip
// between flip and drain), and the drained records carry the park_seq
// epoch so a stale resume from a previous park on the same key is
// recognizable. Lock order is globally prim Mutex -> PARK_TABLE (never
// reversed).
// ---------------------------------------------------------------------------

pub type ParkKeyTuple = (u64, u64);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ParkKey {
    Lock(usize),
    RLock(usize),
    Cond(usize),
    Event(usize),
    Sem(usize),
    Barrier(usize),
    QueueGet(usize),
    QueuePut(usize),
    QueueJoin(usize),
    /// join_wait: keyed (obj_addr, serial) — the double-start-safe identity.
    Join(usize, i64),
    TimerCancel(usize, i64),
    DaemonDrain,
}

static PARK_TABLE: OnceLock<Mutex<HashMap<ParkKey, Vec<ParkKeyTuple>>>> = OnceLock::new();

fn park_table() -> &'static Mutex<HashMap<ParkKey, Vec<ParkKeyTuple>>> {
    PARK_TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register the mounted task (or, for main-thread callers, a synthetic 0
/// task id) on `key`. Caller MUST hold the prim's own state Mutex.
/// Returns the park_seq epoch for this registration.
pub fn park_register(key: ParkKey) -> u64 {
    let task_id = current_task_id();
    let seq = NEXT_PARK_SEQ.fetch_add(1, Ordering::Relaxed);
    park_table()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(key)
        .or_default()
        .push((task_id, seq));
    sched_trace(&format!("task={task_id} park-register {key:?} seq={seq}"));
    seq
}

static NEXT_PARK_SEQ: AtomicU64 = AtomicU64::new(1);

/// Drain the waiters on `key` — called by the WAKER after its prim-state
/// mutation, under the SAME prim Mutex.
pub fn park_drain(key: &ParkKey) -> Vec<ParkKeyTuple> {
    let drained = park_table()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(key)
        .unwrap_or_default();
    for (tid, _) in &drained {
        sched_trace(&format!("task={tid} unparked via {key:?}"));
    }
    drained
}

/// Unregister after a spurious/timeout wake (the parker re-locks, re-checks
/// the predicate, and must not linger in the table).
pub fn park_unregister(key: &ParkKey, task_id: u64, seq: u64) {
    let mut t = park_table().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(v) = t.get_mut(key) {
        v.retain(|&(tid, s)| !(tid == task_id && s == seq));
    }
}

/// Re-insert drained records beyond an exact-count notify budget (notify(k)
/// keeps the surplus registered — impl doc §4.4 notify(k) exact-count).
pub fn park_refix(key: &ParkKey, rec: ParkKeyTuple) {
    park_table()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(*key)
        .or_default()
        .push(rec);
}

/// Registered-parker count for a key (n_waiting-shape diagnostics).
pub fn park_len(key: &ParkKey) -> usize {
    park_table()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(key)
        .map(|v| v.len())
        .unwrap_or(0)
}

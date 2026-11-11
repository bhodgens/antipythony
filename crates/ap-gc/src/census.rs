//! census — the mutator thread census (leaf 02.09b P6).
//!
//! SPLIT-02.09.md decision 3: "What MUST be added: a mutator thread census —
//! the collector must see every live thread's root-bearing thread_locals and
//! (once JIT frames exist under threads) every thread's rootscan capture
//! point; today both are main-thread-only. This is 02.09-b's core edit."
//!
//! The collector keeps ONE registry (process-global since the P6 conversion)
//! plus the subtract-refs invariant. The census is what lets the collecting
//! thread walk OTHER mutators' root-bearing state:
//!
//! - Each registered mutator carries an `active` flag (pruned at thread
//!   exit) and a `capture_slot` mirror of that thread's rootscan capture
//!   point (the thread writes its own slot at the JIT re-entry seam —
//!   ap-run jit_hook — the collector reads every slot).
//! - With no live JIT frame on a thread, its slot reads 0 — the same
//!   degenerate behavior the per-thread read had (gcmod's walker skips a
//!   zero fp). The conservative floor (subtract-refs) covers the rest:
//!   a mutator's stack references exceed internal edge counts and seed the
//!   walk regardless of census coverage.
//!
//! Soundness note (decision 3's posture): a false-RETAINED verdict is
//! benign (retry at the next threshold); a false-COLLECTED verdict needs a
//! decrement racing the scan. The scan aborts on interior-borrow contention
//! (SCAN_CONTENDED, process-global now) and the registry family lock makes
//! insert/sweep atomic per entry — the abort-on-contention direction keeps
//! false-COLLECTED unreachable for the borrow-visible surfaces.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub(crate) struct MutatorEntry {
    /// ap-run's current_os_tid at registration (diagnostics + idempotency).
    #[allow(dead_code)] // diagnostics; iter_active skips inactive entries
    pub thread_id: u64,
    /// False when the thread has exited: pruned on the next census walk.
    pub active: Arc<AtomicBool>,
    /// Mirror of this thread's rootscan capture point (fp of the JIT frame
    /// that re-entered the host; 0 = no live JIT frame). Written by the
    /// OWNING thread only; read by the collector.
    pub capture_slot: Arc<AtomicUsize>,
}

fn mutators() -> &'static Mutex<Vec<MutatorEntry>> {
    static MUTATORS: std::sync::OnceLock<Mutex<Vec<MutatorEntry>>> = std::sync::OnceLock::new();
    MUTATORS.get_or_init(|| Mutex::new(Vec::new()))
}

thread_local! {
    /// This thread's census entry handle (set at register(); the slot is
    /// also kept here so the per-thread write path is one Arc deref).
    static MY_SLOT: std::cell::RefCell<Option<Arc<AtomicUsize>>> =
        const { std::cell::RefCell::new(None) };
    static MY_ACTIVE: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
}

/// Register THIS thread as a live mutator. Idempotent per OS thread (the
/// active flag flips back true on re-entry). Returns the thread's capture
/// slot so the JIT seam can install its mirror writer.
pub fn register() {
    let slot = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicBool::new(true));
    MY_SLOT.with(|s| *s.borrow_mut() = Some(slot.clone()));
    MY_ACTIVE.with(|a| *a.borrow_mut() = Some(active.clone()));
    let mut m = mutators().lock().unwrap_or_else(|p| p.into_inner());
    m.push(MutatorEntry {
        thread_id: 0, // stamped by the caller via set_thread_id (ap-run owns tids)
        active,
        capture_slot: slot,
    });
}

/// Stamp the OS tid (called after register() by ap-run, which owns the
/// tid space; kept separate so ap-gc needs no libc dependency).
pub fn set_thread_id(tid: u64) {
    let mut m = mutators().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(last) = m.last_mut() {
        last.thread_id = tid;
    }
}

/// Mark THIS thread's census entry inactive (worker exit). The entry is
/// physically pruned by the next census walk; the flag makes it invisible
/// immediately.
pub fn unregister() {
    MY_ACTIVE.with(|a| {
        if let Some(flag) = a.borrow().as_ref() {
            flag.store(false, Ordering::Release);
        }
        *a.borrow_mut() = None;
    });
    MY_SLOT.with(|s| *s.borrow_mut() = None);
}

/// THIS thread's capture slot (for the jit_hook mirror writer); None when
/// the thread never registered (the main thread before run_module wiring —
/// the per-thread CAPTURE_POINT read still covers it).
pub fn my_capture_slot() -> Option<Arc<AtomicUsize>> {
    MY_SLOT.with(|s| s.borrow().clone())
}

/// The collector's walk list: capture points of every ACTIVE registered
/// mutator. Entries whose active flag has flipped are pruned here (the
/// collecting thread holds the mutex; worker registration blocks for the
/// snapshot's duration only).
pub fn active_capture_points() -> Vec<usize> {
    let mut m = mutators().lock().unwrap_or_else(|p| p.into_inner());
    m.retain(|e| e.active.load(Ordering::Acquire));
    m.iter()
        .map(|e| e.capture_slot.load(Ordering::Acquire))
        .collect()
}

/// Census size diagnostics (GCSTATS).
pub fn live_mutators() -> usize {
    let mut m = mutators().lock().unwrap_or_else(|p| p.into_inner());
    m.retain(|e| e.active.load(Ordering::Acquire));
    m.len()
}

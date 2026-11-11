//! tierup — the background tier-up driver (leaf 02.05e).
//!
//! Replaces 02.05c's synchronous tier-up at the call boundary: the calling
//! thread now only SNAPSHOTS feedback (on its own thread, before enqueue)
//! and ENQUEUES a compile request; a single background compiler thread does
//! the expensive re-lowering + infer + plan + typed emission and publishes
//! the finished fast-path entry, which a later call boundary installs.
//!
//! Discipline (adapted from the pon tierup.rs "Background compile threading
//! model" per docs/pon-study-tier1.md; pon's pin/snapshot/CAS model is
//! explicitly the right shape for no-GIL, and our GIL v1 does NOT serialize
//! the compiler thread — it only guards execution — so a background
//! compiler is sound today and forward-compatible with 02.09):
//!
//! - **Snapshot-at-enqueue**: feedback cells are snapshotted on the
//!   enqueueing (mutator) thread by the CALLER (jit_hook); the compiler
//!   thread never reads live cells — it compiles from the frozen snapshot.
//! - **Pin-until-retire**: the compiler thread owns every finished
//!   compilation (blob + entry pointer) for its whole lifetime; shutdown
//!   JOINS the compiler before the pins drop. Mapped code survives even
//!   that drop: cranelift-jit 0.134's SystemMemoryProvider leaks mapped
//!   code on drop BY DESIGN ("leak memory to guarantee validity of
//!   function pointers"), so handed-out entry pointers remain valid for
//!   the process lifetime regardless of drop timing. Nothing in this
//!   codebase ever calls `free_memory`.
//! - **Install by Release store + QUEUED→TIER1 CAS**: the compiler
//!   publishes the completion (Release); a mutator observing QUEUED
//!   completes the install through exactly-one CAS QUEUED→TIER1. Racing
//!   mutators lose the CAS and run tier-0 one extra call — correct either
//!   way (byte-identical by the transparency contract).
//! - **Failure → DISABLED**: a failed (or region-less) compile marks the
//!   function DISABLED and it is never retried (pon
//!   failed-tier-up-disables discipline; 02.05c already shipped this on
//!   the sync path — same rule, new owner).
//! - **Shutdown joins the compiler** before any retained module state
//!   drops (see pin-until-retire above).
//!
//! State machine (the SPLIT-02.05 Risk-4 shared machine; it lives HERE —
//! this leaf is the owner. The 02.05d merge seam is documented below):
//!
//! ```text
//! TIER0 ──enqueue──> QUEUED ──install ok──> TIER1
//!                        │
//!                        ├─ compile failed / no typed region ──> DISABLED
//!                        └─ (02.05d: side-exit epoch backoff ──> DEFERRED)
//! ```
//!
//! MERGE SEAM (02.05d): this leaf defines `TierUpState` INDEPENDENTLY of
//! the sibling deopt.rs epoch state (which may not exist on this branch
//! yet). Per the split's recommendation, whichever leaf lands SECOND
//! reconciles both into one shared module — the obvious shape: deopt.rs
//! imports this enum, adds `DEFERRED` transitions from TIER1 driven by its
//! `h_deopt_note` epoch counters, and thrash-DISABLED collapses onto the
//! same DISABLED state this driver already produces for compile failures.
//! Nothing here reads deopt state, so both merge orders work.
//!
//! Debug surface: `APX_SYNC_TIERUP=1` forces jit_hook to keep the 02.05c
//! synchronous compile-at-the-call-boundary shape (pon's PON_SYNC_TIERUP
//! analogue). The driver still tracks the state machine in that mode, but
//! the compile is inline — the full corpus must pass identically in both
//! modes (spec gate).
//!
//! Non-Send objects never cross the channel: `Tier1Func`/`JitFunc` own
//! `JITModule` blobs and stay on their owning thread. The completion
//! payload ([`FastPathHandle`]) is plain data — the twin half of the pair
//! is the mutator's OWN baseline `JitEntry::jit` (the deterministic Risk-5
//! lowering guarantees the compiler's clone of the cfg has the identical
//! slot layout, so a status-2 hand-off to the mutator's baseline is
//! semantically exact).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

// ===========================================================================
// State machine
// ===========================================================================

/// The per-function tier state (SPLIT-02.05 Risk-4 shared machine).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierUpState {
    /// Tier-0 only; no compile in flight.
    Tier0,
    /// Enqueued; the background compiler may be working on it.
    Queued,
    /// Tier-1 installed and serving calls.
    Tier1,
    /// Reserved for 02.05d's thrash backoff (tier-1 compiled, side-exits
    /// exceeded the epoch threshold; run tier-0 for `threshold << epoch`
    /// calls, then re-queue). This leaf never PRODUCES Deferred — the
    /// state exists so the CAS discipline and the stats surface already
    /// understand the full machine (see the module-level merge seam).
    Deferred,
    /// A failed (or region-less) compile, or a lost channel — never
    /// retried.
    Disabled,
}

impl TierUpState {
    /// The u8 wire form stored in the per-function atomic slots (public:
    /// the state machine's observable encoding — 02.05d's deferred
    /// transitions write through the same slots).
    pub fn to_u8(self) -> u8 {
        match self {
            TierUpState::Tier0 => 0,
            TierUpState::Queued => 1,
            TierUpState::Tier1 => 2,
            TierUpState::Deferred => 3,
            TierUpState::Disabled => 4,
        }
    }

    /// Decode the wire form (public: the state machine's observable
    /// encoding; out-of-range values degrade to Disabled — never "active").
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => TierUpState::Tier0,
            1 => TierUpState::Queued,
            2 => TierUpState::Tier1,
            3 => TierUpState::Deferred,
            _ => TierUpState::Disabled,
        }
    }
}

// ===========================================================================
// Completion payload (the only thing crossing threads)
// ===========================================================================

/// A finished fast path, shipped from the compiler thread to a mutator.
/// Plain data: the blob stays pinned on the compiler thread (see the
/// module docs — pin-until-retire + leak-on-drop provider make the pointer
/// valid for the process lifetime).
#[derive(Debug, Clone, Copy)]
pub struct FastPathHandle {
    /// `extern "C" fn(*mut ExecCtx) -> i32` fast-path entry.
    pub fast_ptr: *const u8,
    /// Typed-region count (diagnostics / the install stats line).
    pub regions: usize,
}

// SAFETY: `fast_ptr` points into JIT memory that is never freed
// (`free_memory` is never called anywhere; the cranelift-jit default
// memory provider leaks mapped code on drop by design) and is immutable
// after finalize_definitions. Nothing is mutated through it.
unsafe impl Send for FastPathHandle {}

/// Run an async-compiled fast path with the mutator's own baseline as the
/// cold twin. Status contract identical to [`crate::clif::Tier1Func::call`]
/// (0 = done, 2 = guard failure at region entry → zero observable progress
/// → run the twin from the top). The twin here is the JitEntry's ORIGINAL
/// baseline — the deterministic cfg lowering (Risk-5) makes its slot
/// layout identical to the compiler's clone, so the hand-off is exact.
///
/// # Safety
///
/// `fast_ptr` must be a live finalized fast-path entry (a
/// [`FastPathHandle`] the driver handed out) and `ctx` exclusively
/// borrowed for the call duration — same contract as
/// [`crate::clif::Tier1Func::call`].
pub unsafe fn call_fast(
    fast_ptr: *const u8,
    twin: &crate::clif::JitFunc,
    ctx: &mut crate::runtime::ExecCtx,
) -> Result<(), ap_obj::PyException> {
    let f: extern "C" fn(*mut crate::runtime::ExecCtx) -> i32 = std::mem::transmute(fast_ptr);
    let status = f(ctx as *mut crate::runtime::ExecCtx);
    // Diagnostic-only, env-gated (02.05e union fix): the unconditional
    // eprintln here fired PER CALL — 40k lines for a single race-stress
    // probe, filling the 64KB stderr pipe and deadlocking any harness that
    // spawns the interpreter with piped stderr before draining it (the
    // tierup_race_stress suite died at its own 300s deadline this way).
    if std::env::var("APX_CALLDBG").is_ok() {
        eprintln!("[CALLFAST-DBG] status={status}");
    }
    match status {
        0 => Ok(()),
        2 => twin.call(ctx),
        _ => match ctx.pending_exc.take() {
            Some(e) => Err(e),
            None => Err(ap_obj::PyException::new(
                "SystemError",
                "tier-1 fast path failed without parking an exception",
            )),
        },
    }
}

// ===========================================================================
// Compile request
// ===========================================================================

/// A compile request (mutator → compiler thread). Owns everything the
/// compiler needs: the lowered cfg (moved — the deterministic lowering
/// means the mutator's copy, if any, is redundant) and the FROZEN feedback
/// snapshot (taken on the mutator thread before enqueue; the compiler
/// never reads live cells).
struct CompileRequest {
    fn_id: usize,
    fn_name: String,
    func: crate::cfg::Func,
    snapshots: Vec<crate::feedback::CellSnapshot>,
}

/// One pinned finished compilation on the compiler thread.
struct PinnedFastPath {
    /// Kept for the debug/stats story (the handle handed out is the same
    /// address); the map entry itself is the pin.
    #[allow(dead_code)]
    ptr: *const u8,
    /// The fast path's owning blob — retained on the compiler thread until
    /// shutdown (pin-until-retire; the mapped code survives the drop via
    /// the provider's leak-on-drop design).
    _blob: cranelift_jit::JITModule,
}

// SAFETY: the blob is never freed and the pointer is immutable after
// finalize; the map is local to the compiler thread (no cross-thread
// access at all — pointers are handed out by value).
unsafe impl Send for PinnedFastPath {}

/// Compiler-thread completion record: the handle for installs, or a
/// disable marker (compile failed / no typed region).
type Completion = Result<FastPathHandle, ()>;

/// The per-driver completion hand-off (compiler → mutator). Per-INSTANCE,
/// not process-global: concurrent drivers (parallel test binaries, the
/// unit suite) must never see each other's completions.
#[derive(Default)]
struct Completions {
    inner: Mutex<HashMap<usize, Completion>>,
    ready: AtomicU64,
}

impl Completions {
    fn publish(&self, fn_id: usize, c: Completion) {
        let mut q = self.inner.lock().expect("tierup completions lock");
        q.insert(fn_id, c);
        drop(q);
        self.ready.fetch_add(1, Ordering::Release);
    }

    fn take(&self, fn_id: usize) -> Option<Completion> {
        let mut q = self.inner.lock().expect("tierup completions lock");
        q.remove(&fn_id)
    }

    fn ready_count(&self) -> u64 {
        self.ready.load(Ordering::Acquire)
    }
}

// ===========================================================================
// The driver
// ===========================================================================

/// The tier-up driver: one background compiler thread + the shared state
/// table + the completion hand-off. One instance per process (the global),
/// or per-test instances for the unit suite.
pub struct TierUpDriver {
    /// Request channel sender; taken by shutdown() (closing the channel
    /// ends the compiler's recv loop).
    tx: Mutex<Option<Sender<CompileRequest>>>,
    /// Compiler-thread join handle; shutdown() joins before returning
    /// (pon shutdown discipline: join before dropping retained modules —
    /// the pins drop inside the compiler loop when it exits, and the join
    /// happens-before that drop is complete... precisely: the loop's local
    /// pin map drops as the thread function returns; join() returns after
    /// that, so no retained state outlives an unjoined thread).
    compiler: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Per-function tier states (fn_id → state wire form). Every access
    /// takes this lock; slots are updated in place through the map.
    states: Mutex<HashMap<usize, AtomicU8>>,
    /// Retained (pinned) compilation count — pin-until-retire accounting.
    retained: AtomicU64,
    /// APX_SYNC_TIERUP=1 (resolved once; jit_hook branches on it to keep
    /// the 02.05c inline-compile shape; the driver's async machinery is
    /// unused in that mode).
    sync_mode: bool,
    /// Compiler → mutator completion hand-off (per-instance).
    completions: Arc<Completions>,
}

impl TierUpDriver {
    /// Construct + start the driver (one background compiler thread,
    /// unless APX_SYNC_TIERUP=1 — sync mode needs no thread).
    pub fn start() -> Arc<TierUpDriver> {
        let sync_mode = std::env::var("APX_SYNC_TIERUP")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        TierUpDriver::start_with(sync_mode)
    }

    /// Force-construct the driver in sync mode WITHOUT the env var (test
    /// seam: parallel tests share the process env, so an env-var-driven
    /// sync test would flip unrelated drivers; jit_hook keeps using
    /// `start()`, which reads APX_SYNC_TIERUP once at init).
    pub fn start_sync() -> Arc<TierUpDriver> {
        TierUpDriver::start_with(true)
    }

    fn start_with(sync_mode: bool) -> Arc<TierUpDriver> {
        let (tx, rx) = std::sync::mpsc::channel::<CompileRequest>();
        let completions = Arc::new(Completions::default());
        let compiler = if sync_mode {
            None
        } else {
            let completions = completions.clone();
            Some(
                std::thread::Builder::new()
                    .name("apjit-tierup".to_string())
                    .spawn(move || compiler_loop(rx, completions))
                    .expect("spawn tier-up compiler thread"),
            )
        };
        Arc::new(TierUpDriver {
            tx: Mutex::new(if sync_mode { None } else { Some(tx) }),
            compiler: Mutex::new(compiler),
            states: Mutex::new(HashMap::new()),
            retained: AtomicU64::new(0),
            sync_mode,
            completions,
        })
    }

    /// Whether this driver runs in APX_SYNC_TIERUP debug mode.
    pub fn sync_mode(&self) -> bool {
        self.sync_mode
    }

    /// Read a function's tier state (probe/stats surface).
    pub fn state_of(&self, fn_id: usize) -> TierUpState {
        let map = self.states.lock().expect("tierup states lock");
        map.get(&fn_id)
            .map(|s| TierUpState::from_u8(s.load(Ordering::Acquire)))
            .unwrap_or(TierUpState::Tier0)
    }

    fn set_state(&self, fn_id: usize, st: TierUpState, ord: Ordering) {
        let mut map = self.states.lock().expect("tierup states lock");
        let slot = map
            .entry(fn_id)
            .or_insert_with(|| AtomicU8::new(TierUpState::Tier0.to_u8()));
        slot.store(st.to_u8(), ord);
    }

    /// Count of retained (pinned) compilations.
    pub fn retained_count(&self) -> u64 {
        self.retained.load(Ordering::Relaxed)
    }

    /// Compile completions published but not yet installed (loop-tolerant
    /// test/probe surface: wait on THIS, not queued_count — QUEUED only
    /// clears when a mutator polls the install).
    pub fn completions_ready(&self) -> u64 {
        self.completions.ready_count()
    }

    /// Functions currently QUEUED (probe surface; loop-tolerant tests use
    /// this instead of sleeping fixed amounts).
    pub fn queued_count(&self) -> usize {
        let map = self.states.lock().expect("tierup states lock");
        map.values()
            .filter(|s| s.load(Ordering::Acquire) == TierUpState::Queued.to_u8())
            .count()
    }

    /// Enqueue a tier-1 compile for `fn_id`.
    ///
    /// Exactly-once: only the thread whose CAS moves Tier0→Queued sends
    /// the request; later callers (and racing threads) observe a non-Tier0
    /// state and return without enqueuing. A lost channel (compiler
    /// already shut down) marks DISABLED — fail safe to tier-0 forever,
    /// never a retry loop.
    ///
    /// The caller must have snapshotted the cells on ITS OWN thread and
    /// applied the hotness gate — this method trusts both (single owner of
    /// live cells: the mutator).
    pub fn request(
        &self,
        fn_id: usize,
        fn_name: &str,
        func: crate::cfg::Func,
        snapshots: Vec<crate::feedback::CellSnapshot>,
    ) -> bool {
        // Exactly-once enqueue (the QUEUED transition IS the claim).
        {
            let mut map = self.states.lock().expect("tierup states lock");
            let slot = map
                .entry(fn_id)
                .or_insert_with(|| AtomicU8::new(TierUpState::Tier0.to_u8()));
            let claimed = slot.compare_exchange(
                TierUpState::Tier0.to_u8(),
                TierUpState::Queued.to_u8(),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            if claimed.is_err() {
                return false; // queued / tier1 / disabled already
            }
        }
        let req = CompileRequest {
            fn_id,
            fn_name: fn_name.to_string(),
            func,
            snapshots,
        };
        let sent = match self.tx.lock().expect("tierup tx lock").as_ref() {
            Some(tx) => tx.send(req).is_ok(),
            None => false, // sync mode never routes through here; or shut down
        };
        if !sent {
            // Compiler gone: fail safe to DISABLED (tier-0 forever, never
            // retried) — the mutator keeps byte-identical tier-0 results.
            self.set_state(fn_id, TierUpState::Disabled, Ordering::Release);
            return false;
        }
        true
    }

    /// Poll for a finished compile and INSTALL it for `fn_id`.
    ///
    /// The install is the CAS: QUEUED→TIER1 happens exactly once; a racing
    /// mutator loses the CAS, gets None, and runs tier-0 one extra call.
    /// A disable-marker completion moves QUEUED→DISABLED (never retried).
    /// Returns the fast-path handle when THIS caller won the install.
    pub fn poll(&self, fn_id: usize) -> Option<FastPathHandle> {
        if self.completions.ready_count() == 0 {
            return None; // fast path: nothing has EVER completed
        }
        if self.state_of(fn_id) != TierUpState::Queued {
            return None;
        }
        let done = self.completions.take(fn_id);
        match done {
            Some(Ok(handle)) => {
                // THE install CAS (Release: the mutator's later execution
                // of the handed-out pointer must observe the finalized
                // code — finalized before the completion was published).
                let mut map = self.states.lock().expect("tierup states lock");
                let slot = map
                    .entry(fn_id)
                    .or_insert_with(|| AtomicU8::new(TierUpState::Tier0.to_u8()));
                let won = slot
                    .compare_exchange(
                        TierUpState::Queued.to_u8(),
                        TierUpState::Tier1.to_u8(),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok();
                drop(map);
                if won {
                    self.retained.fetch_add(1, Ordering::Relaxed);
                    Some(handle)
                } else {
                    None
                }
            }
            Some(Err(())) => {
                // Failed-tier-up-disables (pon discipline): compile error
                // OR no typed region — never retried.
                self.set_state(fn_id, TierUpState::Disabled, Ordering::Release);
                None
            }
            None => None, // not our completion (or already consumed)
        }
    }

    /// Bookkeeping for the APX_SYNC_TIERUP=1 inline path (jit_hook
    /// compiles synchronously exactly as 02.05c did; the driver still
    /// owns the state machine so stats and the failed-disable rule have a
    /// single home in both modes).
    pub fn note_installed(&self, fn_id: usize) {
        self.retained.fetch_add(1, Ordering::Relaxed);
        self.set_state(fn_id, TierUpState::Tier1, Ordering::Release);
    }

    /// Mark DISABLED from the caller side (sync-mode compile failure /
    /// region-less body — the 02.05c `tier1_failed` rule, mirrored into
    /// the shared state machine).
    pub fn mark_disabled(&self, fn_id: usize) {
        self.set_state(fn_id, TierUpState::Disabled, Ordering::Release);
    }

    /// Stats snapshot: `[tier0, queued, tier1, deferred, disabled]` over
    /// all tracked functions.
    pub fn state_counts(&self) -> [u64; 5] {
        let map = self.states.lock().expect("tierup states lock");
        let mut counts = [0u64; 5];
        for s in map.values() {
            let v = s.load(Ordering::Acquire);
            counts[v.min(4) as usize] += 1;
        }
        counts
    }

    /// Shutdown: close the request channel, then JOIN the compiler before
    /// returning (pon shutdown discipline — join before dropping retained
    /// modules; the compiler's pin map drops as its thread function
    /// returns, and join happens-after that).
    pub fn shutdown(&self) {
        self.tx.lock().expect("tierup tx lock").take();
        if let Some(h) = self.compiler.lock().expect("tierup compiler lock").take() {
            let _ = h.join();
        }
    }
}

impl Drop for TierUpDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ===========================================================================
// Compiler thread
// ===========================================================================

fn tier_debug() -> bool {
    std::env::var("APX_TIER_DEBUG")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn compiler_loop(rx: Receiver<CompileRequest>, completions: Arc<Completions>) {
    // Pin-until-retire: every finished compilation is retained HERE, on
    // the compiler thread, until the loop exits (shutdown join). Mapped
    // code stays valid even after this map drops — cranelift-jit 0.134's
    // SystemMemoryProvider leaks mapped code on drop BY DESIGN ("leak
    // memory to guarantee validity of function pointers"), and nothing in
    // this codebase calls free_memory. So handed-out entry pointers are
    // valid for the process lifetime in every drop order.
    let mut pinned: HashMap<usize, PinnedFastPath> = HashMap::new();
    while let Ok(req) = rx.recv() {
        let CompileRequest {
            fn_id,
            fn_name,
            func,
            snapshots,
        } = req;
        // The expensive half of the pipeline, ON THE COMPILER THREAD, from
        // the frozen snapshot: baseline twin + infer + plan + typed
        // emission. The twin half of the returned pair is deliberately
        // DROPPED (not freed — the mutator executes its OWN baseline as
        // the twin; see the module docs), and the fast-path blob is
        // pinned above.
        let outcome = {
            // 02.05d: the deopt thrash key IS the fn identity the driver
            // was requested under — the same def-pointer key jit_hook
            // gates tier-up and dispatch on.
            let fn_key = fn_id as u64;
            match crate::clif::compile_tier1(&func, &snapshots, fn_key) {
                Ok(Some(t1)) => {
                    // compile_tier1 already ran deopt::note_tier1_installed
                    // (Tier1, counters reset) for this fn_key.
                    let handle = FastPathHandle {
                        fast_ptr: t1.fast_ptr,
                        regions: t1.regions,
                    };
                    pinned.insert(
                        fn_id,
                        PinnedFastPath {
                            ptr: t1.fast_ptr,
                            _blob: t1._fast_blob,
                        },
                    );
                    let _ = t1.twin; // dropped, NOT freed (see above)
                    Ok(handle)
                }
                Ok(None) => Err(()), // no typed region: permanently boxed
                Err(e) => {
                    if tier_debug() {
                        eprintln!(
                            "[TIER1-ASYNC] fn {fn_name} tier-1 compile failed \
                             (disabled): {e:?}"
                        );
                    }
                    Err(())
                }
            }
        };
        if tier_debug() && outcome.is_ok() {
            if let Ok(h) = &outcome {
                eprintln!(
                    "[TIER1-ASYNC] fn {fn_name} compiled regions={} (awaiting install)",
                    h.regions
                );
            }
        }
        completions.publish(fn_id, outcome);
    }
    // Channel closed by shutdown(): pins drop here, mapped code survives
    // (leak-on-drop provider), thread exits, join() in shutdown() returns.
}

// ===========================================================================
// Process-global driver (ap-run's init seam)
// ===========================================================================

static GLOBAL: Mutex<Option<Arc<TierUpDriver>>> = Mutex::new(None);

/// The process driver, constructing it on first use.
pub fn global() -> Arc<TierUpDriver> {
    let mut g = GLOBAL.lock().expect("tierup global lock");
    g.get_or_insert_with(TierUpDriver::start).clone()
}

/// The process driver without forcing construction.
pub fn try_global() -> Option<Arc<TierUpDriver>> {
    GLOBAL.lock().expect("tierup global lock").clone()
}

/// Shutdown the process driver (join the compiler). Called from ap-run's
/// module-exit seam. Idempotent.
pub fn shutdown_global() {
    let d = GLOBAL.lock().expect("tierup global lock").take();
    if let Some(d) = d {
        d.shutdown();
    }
}

/// The `[TIERUP-STATS]` diagnostics line (tier-state histogram + retained
/// count) for an explicit driver — the report seam passes the global in
/// so a not-started driver prints nothing at the call site.
pub fn stats_line_for(d: &TierUpDriver) -> String {
    let c = d.state_counts();
    format!(
        "[TIERUP-STATS] tier0={} queued={} tier1={} deferred={} disabled={} retained={}",
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        d.retained_count()
    )
}

/// The `[TIERUP-STATS]` diagnostics line for the process driver (None →
/// empty; the caller prints nothing when the driver never started).
pub fn stats_line() -> Option<String> {
    try_global().map(|d| stats_line_for(&d))
}

//! 02.05d — deopt thrash backoff (pon `abi.rs:5354-5400` analogue).
//!
//! Adapted from pon (github.com/can1357/pon). Copyright Can Bölük,
//! MIT/Apache-2.0; adapted for antipythony under the granted dual license
//! with attribution.
//!
//! THE PROBLEM (PROGRESS-0205c open item): a tier-1 fast path that
//! side-exits on almost every call (the caller alternates int/str, the
//! "deliberate thrash" shape) costs the guard checks PLUS the twin plus
//! the deopt bookkeeping — strictly worse than never having tiered up.
//! pon's answer: count side-exits per function per epoch; when an epoch
//! reaches [`DEOPT_THRASH_THRESHOLD`] the function drops to
//! [`TierState::Deferred`] and the tier-up threshold doubles per epoch
//! (`threshold << epoch`, capped at shift 6); after [`DEOPT_PIN_EPOCH`]
//! epochs it is pinned [`TierState::Disabled`] permanently. 02.05c
//! already lands the side-exit edge (`h_deopt_note` on every guard /
//! exactness failure) — this module owns what happens AFTER the note.
//!
//! THE STATE MACHINE (SPLIT-02.05 §02.05d risk note: "states land here
//! or in 02.05e, pick one owner" — OWNER: deopt.rs; 02.05e's driver
//! consumes [`TierState`] and [`defer_threshold`] from here):
//!
//! ```text
//! Tier0 ──(hotness gate)──> Tier1 ──(side-exit flood)──> Deferred ──(8 epochs)──> Disabled
//!   ^                        ^             │  (tier-up re-fires at              │
//!   └── Disabled is FINAL; Deferred re-admits compile-only        threshold << epoch)┘
//! ```
//!
//! True state-materialization deopt stays OUT OF SCOPE (study problem
//! #3): exits re-box live values through the write-through slot traffic
//! (02.05c's `h_store_smi` invariant: all boxed state is already in
//! `ctx.slots`), so a cold-twin hand-off needs NO state vector — the
//! twin simply continues at the parked `twin_start` block.
//!
//! CONCURRENCY: the table is keyed by the function's def-pointer identity
//! and lives behind atomics — no locks, no GIL interaction (the 02.09
//! flip must not need a re-audit here: Relaxed orderings on
//! independent words; a torn or lost count can only cost one epoch of
//! delay, never a wrong answer — fail-OPEN by design). Entry is
//! `(u64 key -> DeoptState)`; the 02.05e driver shares the same table
//! via [`state_of`] / [`record_side_exit`].
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Side-exits within ONE epoch before the function drops to
/// [`TierState::Deferred`] (pon `DEOPT_THRASH_THRESHOLD`).
pub const DEOPT_THRASH_THRESHOLD: u64 = 64;

/// Deferred epochs before the DISABLED pin (pon `DEOPT_PIN_EPOCH`).
pub const DEOPT_PIN_EPOCH: u64 = 8;

/// Max shift for the deferred re-try threshold (`threshold << epoch`,
/// capped — pon's `min(epoch, 6)` shape).
const MAX_SHIFT: u32 = 6;

/// Tier states for one function (pon `object.rs:801-811` analogue).
/// OWNER per SPLIT-02.05 §02.05d: this enum lives HERE; the 02.05e
/// driver consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TierState {
    /// Boxed-only (baseline); tier-up eligible when hot.
    Tier0 = 0,
    /// Tier-1 installed (fast path + cold twin).
    Tier1 = 1,
    /// A compile request is in flight (02.05e's QUEUED; unused until
    /// the driver lands — the sync tier-up path never occupies it).
    Queued = 2,
    /// Thrash-detected: tier-1 stays installed (it is semantically
    /// free — guards just fail), but re-tier-up waits until the call
    /// count crosses [`defer_threshold`].
    Deferred = 3,
    /// Pinned OFF after [`DEOPT_PIN_EPOCH`] deferred epochs. FINAL.
    Disabled = 4,
}

impl TierState {
    pub fn from_u8(v: u8) -> Option<TierState> {
        Some(match v {
            0 => TierState::Tier0,
            1 => TierState::Tier1,
            2 => TierState::Queued,
            3 => TierState::Deferred,
            4 => TierState::Disabled,
            _ => return None,
        })
    }

    /// Whether a tier-up compile attempt may fire in this state
    /// (the jit_hook consults this BEFORE snapshotting/compiling).
    pub fn tier_up_eligible(self) -> bool {
        matches!(
            self,
            TierState::Tier0 | TierState::Tier1 | TierState::Queued
        )
    }
}

/// Packed per-function deopt word (one AtomicU64 per key):
/// low 8 bits = TierState discriminant; bits 8..40 = side-exit count in
/// the CURRENT epoch; bits 40..64 = epoch counter (saturating at 2^24 —
/// far beyond DEOPT_PIN_EPOCH's reach).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeoptWord(u64);

const STATE_MASK: u64 = 0xFF;
const COUNT_SHIFT: u32 = 8;
const COUNT_MASK: u64 = 0xFF_FFFF;
const EPOCH_SHIFT: u32 = 32;

impl DeoptWord {
    fn state(self) -> TierState {
        TierState::from_u8((self.0 & STATE_MASK) as u8).unwrap_or(TierState::Tier0)
    }
    fn count(self) -> u64 {
        (self.0 >> COUNT_SHIFT) & COUNT_MASK
    }
    fn epoch(self) -> u64 {
        self.0 >> EPOCH_SHIFT
    }
    fn pack(state: TierState, count: u64, epoch: u64) -> DeoptWord {
        DeoptWord(
            (state as u64)
                | ((count & COUNT_MASK) << COUNT_SHIFT)
                | (epoch.min(0xFF_FFFF) << EPOCH_SHIFT),
        )
    }
}

thread_local! {
    /// Per-function deopt words, keyed by the SAME def-pointer identity
    /// jit_hook uses for its JIT_CACHE (`Rc::as_ptr as usize`). One
    /// leaked AtomicU64 per known function; absent == Tier0/0/0.
    static DEOPT_TABLE: Mutex<HashMap<u64, &'static AtomicU64>> = Mutex::new(HashMap::new());
}

fn word_for(key: u64) -> &'static AtomicU64 {
    // Leak-once table of AtomicU64 cells: bounded by the number of
    // distinct function bodies that ever tier up (same bounded-per-fn
    // shape as the JitEntry cache itself).
    let cell: &'static AtomicU64 = DEOPT_TABLE.with(|t| {
        let mut map = t.lock().expect("deopt table poisoned");
        let existing: Option<&'static AtomicU64> = map.get(&key).copied();
        existing.unwrap_or_else(|| {
            let leaked: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(
                DeoptWord::pack(TierState::Tier0, 0, 0).0,
            )));
            map.insert(key, leaked);
            leaked
        })
    });
    cell
}

/// The function's current tier state (absent key = Tier0). The jit_hook
/// tier-up probe and the 02.05e driver read this.
pub fn state_of(key: u64) -> TierState {
    DeoptWord(word_for(key).load(Ordering::Relaxed)).state()
}

/// The per-function state word itself (02.05d fast-path dispatch gate):
/// the jit_hook caches this pointer at tier-1 install time so the
/// per-call Disabled check is ONE relaxed byte load — no map lookup on
/// the dispatch hot path. Low 8 bits are the TierState discriminant.
pub fn word_cell(key: u64) -> &'static AtomicU64 {
    word_for(key)
}

/// Decode just the state byte of a word (the dispatch gate's read).
pub fn state_byte_of(word: &AtomicU64) -> TierState {
    TierState::from_u8((word.load(Ordering::Relaxed) & STATE_MASK) as u8)
        .unwrap_or(TierState::Tier0)
}

/// Record that the tier-1 install succeeded for `key` (Tier0/Queued →
/// Tier1, counters reset). Called by the compile path.
pub fn note_tier1_installed(key: u64) {
    word_for(key).store(DeoptWord::pack(TierState::Tier1, 0, 0).0, Ordering::Relaxed);
}

/// The call-count threshold a Deferred function must reach before its
/// tier-up probe may fire again: `DEOPT_THRASH_THRESHOLD << epoch`,
/// capped at shift 6 (= 4096 calls at epoch 6+). Mirrors pon's backoff.
pub fn defer_threshold(epoch: u64) -> u64 {
    DEOPT_THRASH_THRESHOLD << epoch.min(MAX_SHIFT as u64)
}

/// ONE side-exit note from a tier-1 fast path (the `h_deopt_note`
/// body). Returns the post-note state for the caller to act on.
///
/// Transition rules (pon abi.rs analogue):
/// - Tier1: count toward the epoch. Crossing the threshold drops to
///   Deferred with a fresh epoch. When the FRESH epoch's number reaches
///   DEOPT_PIN_EPOCH the state pins Disabled (final).
/// - Already Deferred/Disabled: Disabled stays Disabled; Deferred
///   increments its epoch counter toward the pin (a thrashing function
///   that never re-tiers still reaches Disabled — the pin is a function
///   of EPOCHS OBSERVED, not compiles attempted).
/// - Tier0/Queued: a side-exit cannot occur in Tier0 (no fast path);
///   tolerated as a no-op for robustness (fail-open).
pub fn record_side_exit(key: u64) -> TierState {
    let word = word_for(key);
    SIDE_EXIT_NOTES.fetch_add(1, Ordering::Relaxed);
    loop {
        let cur = DeoptWord(word.load(Ordering::Relaxed));
        let (state, count, epoch) = (cur.state(), cur.count(), cur.epoch());
        let next = match state {
            TierState::Disabled => return TierState::Disabled,
            TierState::Tier0 | TierState::Queued => {
                // No fast path runs in these states; ignore (robustness).
                return state;
            }
            TierState::Tier1 | TierState::Deferred => {
                let count = count + 1;
                if count >= DEOPT_THRASH_THRESHOLD {
                    let epoch = epoch + 1;
                    if epoch >= DEOPT_PIN_EPOCH {
                        // Pin: DISABLED permanently (counters reset).
                        word.store(
                            DeoptWord::pack(TierState::Disabled, 0, epoch).0,
                            Ordering::Relaxed,
                        );
                        DISABLED_PINS.fetch_add(1, Ordering::Relaxed);
                        // Probe line (machine-parseable, same channel as
                        // [TIER1-STATS]): the pin is the leaf gate's
                        // observable. key = the def-pointer identity.
                        if crate::runtime::tier1_stats_enabled() {
                            eprintln!("[TIER1-STATS] pin key={key} epochs={epoch}");
                        }
                        return TierState::Disabled;
                    }
                    // Drop to Deferred for this epoch: the jit_hook runs
                    // the boxed baseline and re-admits the fast path only
                    // after defer_threshold(epoch) more calls (pon's
                    // entry-pointer restore + `threshold << epoch` retry).
                    word.store(
                        DeoptWord::pack(TierState::Deferred, 0, epoch).0,
                        Ordering::Relaxed,
                    );
                    DEFERRED_DROPS.fetch_add(1, Ordering::Relaxed);
                    return TierState::Deferred;
                }
                DeoptWord::pack(state, count, epoch)
            }
        };
        // Relaxed is sufficient: the word is advisory (fail-open) and
        // single-writer-per-call-site in practice; a lost update costs
        // one epoch of delay at worst.
        if word
            .compare_exchange_weak(cur.0, next.0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return state;
        }
        // Lost the race (nested recursion of the same fn on one thread
        // cannot race; 02.09 threads might): retry.
    }
}

/// Whether the jit_hook's tier-up probe may fire for `key` now.
/// `calls_since` is the caller's cheap progress signal (jit_hook uses
/// the per-call JIT_EXECS counter — monotonic, shared).
pub fn tier_up_allowed(key: u64, calls_since: u64) -> bool {
    let cur = DeoptWord(word_for(key).load(Ordering::Relaxed));
    match cur.state() {
        s if s.tier_up_eligible() => true,
        TierState::Deferred => calls_since >= defer_threshold(cur.epoch()),
        TierState::Disabled => false,
        _ => false,
    }
}

/// Tier-state + counters read for `key` (tests + probe surface):
/// `(state, epoch, side_exits_in_epoch)`.
pub fn deopt_stats(key: u64) -> (TierState, u64, u64) {
    let cur = DeoptWord(word_for(key).load(Ordering::Relaxed));
    (cur.state(), cur.epoch(), cur.count())
}

/// Process-wide thrash counters for the `[TIER1-STATS]` line:
/// `(deferred_drops, disabled_pins, side_exit_notes)`.
static DEFERRED_DROPS: AtomicU64 = AtomicU64::new(0);
static DISABLED_PINS: AtomicU64 = AtomicU64::new(0);
static SIDE_EXIT_NOTES: AtomicU64 = AtomicU64::new(0);

pub fn thrash_counters() -> (u64, u64, u64) {
    (
        DEFERRED_DROPS.load(Ordering::Relaxed),
        DISABLED_PINS.load(Ordering::Relaxed),
        SIDE_EXIT_NOTES.load(Ordering::Relaxed),
    )
}

/// Reset the per-function table + counters (tests only).
pub fn reset_for_test() {
    DEOPT_TABLE.with(|t| t.lock().expect("deopt table poisoned").clear());
    DEFERRED_DROPS.store(0, Ordering::Relaxed);
    DISABLED_PINS.store(0, Ordering::Relaxed);
    SIDE_EXIT_NOTES.store(0, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Tests: the state machine + backoff arithmetic (no JIT required).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The per-function keys are test-distinct, but `reset_for_test` and
    /// the process-wide thrash counters are SHARED — serialize the tests.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tier_up_allowed_defaults_true_and_disabled_is_final() {
        let _g = TEST_LOCK.lock().expect("test lock poisoned");
        reset_for_test();
        let k = 10_001;
        assert_eq!(state_of(k), TierState::Tier0);
        assert!(tier_up_allowed(k, 0));
        // Install (the runtime flow: compile_tier1 -> note_tier1_installed),
        // then drive the pin path: 8 epochs of 64 side-exits each; the
        // 512th note (8th epoch's 64th exit) IS the pin.
        note_tier1_installed(k);
        assert_eq!(state_of(k), TierState::Tier1);
        let total = DEOPT_PIN_EPOCH * DEOPT_THRASH_THRESHOLD;
        for n in 1..=total {
            let s = record_side_exit(k);
            if n == total {
                assert_eq!(s, TierState::Disabled, "pin lands on the last note");
            } else {
                assert_ne!(s, TierState::Disabled, "pin landed early at note {n}");
            }
            // Between epochs the state is Deferred (never straight Tier1).
            if n % DEOPT_THRASH_THRESHOLD == 0 && n != total {
                assert_eq!(state_of(k), TierState::Deferred);
            }
        }
        assert_eq!(state_of(k), TierState::Disabled);
        // Disabled is FINAL: further notes are no-ops, tier-up never fires.
        assert_eq!(record_side_exit(k), TierState::Disabled);
        assert!(!tier_up_allowed(k, u64::MAX));
        let (_drops, pins, _notes) = thrash_counters();
        assert_eq!(pins, 1, "exactly one DISABLED pin");
    }

    #[test]
    fn defer_backoff_doubles_and_is_capped_at_shift_6() {
        let _g = TEST_LOCK.lock().expect("test lock poisoned");
        assert_eq!(defer_threshold(0), 64);
        assert_eq!(defer_threshold(1), 128);
        assert_eq!(defer_threshold(3), 512);
        assert_eq!(defer_threshold(6), 64 << 6);
        assert_eq!(defer_threshold(7), 64 << 6, "shift caps at 6");
        assert_eq!(defer_threshold(1_000), 64 << 6);
    }

    #[test]
    fn deferred_state_gates_tier_up_by_call_count() {
        let _g = TEST_LOCK.lock().expect("test lock poisoned");
        reset_for_test();
        let k = 10_002;
        note_tier1_installed(k); // the runtime install flow
                                 // 63 exits: still Tier1.
        for _ in 0..(DEOPT_THRASH_THRESHOLD - 1) {
            record_side_exit(k);
        }
        assert_eq!(state_of(k), TierState::Tier1);
        // 64th exit: Deferred, epoch 1, counter reset.
        assert_eq!(record_side_exit(k), TierState::Deferred);
        let (state, epoch, count) = deopt_stats(k);
        assert_eq!(state, TierState::Deferred);
        assert_eq!(epoch, 1);
        assert_eq!(count, 0);
        // Not eligible below the backoff threshold...
        assert!(!tier_up_allowed(k, 0));
        assert!(!tier_up_allowed(k, defer_threshold(1) - 1));
        // ...eligible at it.
        assert!(tier_up_allowed(k, defer_threshold(1)));
    }

    #[test]
    fn side_exit_notes_counter_tracks_every_note() {
        let _g = TEST_LOCK.lock().expect("test lock poisoned");
        reset_for_test();
        let k = 10_003;
        note_tier1_installed(k);
        let notes_before = thrash_counters().2;
        for _ in 0..5 {
            record_side_exit(k);
        }
        assert_eq!(thrash_counters().2 - notes_before, 5);
    }

    #[test]
    fn tier0_notes_are_noops_and_retry_threshold_shape_holds() {
        let _g = TEST_LOCK.lock().expect("test lock poisoned");
        reset_for_test();
        let k = 10_004;
        // Tier0 (never installed): notes tolerated as no-ops (fail-open),
        // state unchanged, tier-up stays eligible.
        assert_eq!(record_side_exit(k), TierState::Tier0);
        assert_eq!(state_of(k), TierState::Tier0);
        assert!(tier_up_allowed(k, 0));
        // And the pin arithmetic: at epoch 7 (8th transition) it lands.
        note_tier1_installed(k);
        for _ in 0..(DEOPT_PIN_EPOCH * DEOPT_THRASH_THRESHOLD - 1) {
            record_side_exit(k);
        }
        assert_eq!(state_of(k), TierState::Deferred, "epoch 8 not yet crossed");
        assert_eq!(record_side_exit(k), TierState::Disabled, "pin lands here");
        // Deferred epoch snapshot readable (probes/tests).
        let (state, epoch, _count) = deopt_stats(k);
        assert_eq!(state, TierState::Disabled);
        assert_eq!(epoch, 8);
    }
}

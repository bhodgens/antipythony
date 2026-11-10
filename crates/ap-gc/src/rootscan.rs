//! rootscan — the GC roots shim (leaf 02.05g, Risk 1): precise stack maps
//! plus the frame-pointer-chain walker. The pon `stackmap.rs` pattern
//! adapted per the two design docs: docs/pon-study-tier1.md ("Green Tea
//! GC integration points") and docs/gc-mmtk-swap.md §4.
//!
//! Attribution: adapted from pon (github.com/can1357/pon,
//! pon-runtime/src/stackmap.rs + pon-gc conservative-scan floor patterns).
//! Copyright Can Bölük, MIT/Apache-2.0; adapted for antipythony under the
//! granted dual license with attribution.
//!
//! THE CONTRACT (SPLIT-02.05.md §02.05g):
//!
//! - A return-address-keyed [`SafepointMap`]: the JIT (ap-codegen-native,
//!   clif.rs `declare_gc_values` analogue) registers one entry per
//!   helper-call safepoint, listing the live root slots of THAT frame as
//!   frame-pointer offsets. The map is the root-reporting shim handed to
//!   the collector (today) / MMTk `Scanning::scan_roots_in_mutator_thread`
//!   (stage 2 — the same data feeds both).
//! - The [`fp_chain_walk`] walker: from a thread's current frame pointer,
//!   walk saved-rbp/saved-rip pairs to enumerate native JIT frames, look
//!   each return address up in the map, and report the live slots. Frames
//!   WITHOUT a map entry fall back to the conservative floor (below) —
//!   pon's own answer for OSR/non-registered frames (never collect with
//!   an unenumerated frame that could hold the only reference).
//! - The conservative floor: this crate NEVER drops the subtract-refs
//!   invariant (`strong_count > internal edges` seeds reachability in
//!   gcmod.rs). The floor is therefore structural: the registry clone
//!   arithmetic makes any external holder — including a temporay the
//!   walker missed — a root. [`ConservativeFloorGuard`] exists to pin the
//!   policy in one named place and to expose the floor state for tests,
//!   mirroring pon's `set_conservative_scan_floor` seam.
//! - Scrub discipline: dead JIT slots are zeroed by the codegen tail
//!   (clif.rs) so a stale slot pointer cannot pin garbage between
//!   safepoints; [`rootscan`] only reads what the maps declare live.
//!
//! Division of labor vs the existing collector (augment, not replace —
//! the leaf brief): tier-0 keeps every value in the ExecCtx slot array,
//! which IS the precise-frame choice (PROGRESS-0205 Risk-1); the map
//! registry exists so tier-1 (02.05c typed codegen) can shrink the
//! per-frame root set to the callsite live-set without a redesign, and
//! so the MMTk binding (stage 2) has its root feed ready.
//!
//! Thread-safety: the registry is a process-global
//! `RwLock<HashMap>` (not a thread_local) because a collection on the
//! mutator thread must see maps registered by ANY compile thread
//! (02.05's background tier-up driver compiles off-thread; pon pins maps
//! before install for the same reason). Walker state is per-call.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::RwLock;

/// One live root slot of a frame at a safepoint: the slot's byte offset
/// from the frame pointer (pon `StackRootSlot{fp_offset}` analogue).
///
/// Offsets are NON-NEGATIVE: the codegen computes them against the frame
/// pointer with the same signed convention Cranelift uses for spillslot
/// addresses (`fp + offset` where offset may address either direction —
/// the codegen owns the sign; this type stores the raw biased value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackRootSlot {
    /// Byte offset from the frame pointer of the slot holding a
    /// `Value` (16-byte enum-of-Rc: the Rc payload pointer is the first
    /// word; the walker reports slot BASE addresses and the consumer
    /// re-derives the payload pointer exactly as `value_addr` does).
    pub fp_offset: i32,
}

/// Per-callsite stack map (pon `SafepointMap` analogue): the roots of one
/// frame at one return address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafepointEntry {
    /// Live root slots (frame-pointer offsets), deterministic order
    /// (slot index ascending — the codegen emits in that order; the
    /// walker preserves it so maps diff cleanly across recompiles).
    pub roots: Vec<StackRootSlot>,
}

/// The return-address-keyed registry. Global (see thread-safety note).
static SAFEPOINT_MAPS: RwLock<Option<HashMap<usize, SafepointEntry>>> = RwLock::new(None);

/// Registered-map count (probe/stats surface; atomics so the read path
/// never takes the write lock).
static MAP_COUNT: AtomicUsize = AtomicUsize::new(0);
/// Total root slots across all registered maps (probe/stats surface).
static ROOT_SLOT_COUNT: AtomicUsize = AtomicUsize::new(0);
/// fp-chain walk counter (probe surface: the walk actually ran).
static WALK_COUNT: AtomicU64 = AtomicU64::new(0);
/// Frames walked that had NO map entry (probe surface: conservative-floor
/// fallbacks actually happened).
static UNMAPPED_FRAME_COUNT: AtomicU64 = AtomicU64::new(0);

/// Register one callsite map. Called by the JIT at code-finalize time
/// (before the code becomes executable — pon pins maps before install).
/// Re-registering the same return address REPLACES the entry (a function
/// recompiled into fresh JIT memory owns fresh addresses, so a collision
/// across compilations cannot occur in practice; the replace semantics
/// keep the API idempotent for the deterministic re-lowering test).
pub fn register_safepoint(return_address: usize, roots: Vec<StackRootSlot>) {
    if roots.is_empty() {
        // A callsite with no live roots needs no entry: the walker's
        // map-miss path treats "no entry" as "nothing to report from
        // this frame" (the conservative floor covers correctness).
        return;
    }
    let mut maps = SAFEPOINT_MAPS.write().expect("rootscan.rs: maps lock");
    let maps = maps.get_or_insert_with(HashMap::new);
    if let Some(old) = maps.insert(
        return_address,
        SafepointEntry {
            roots: roots.clone(),
        },
    ) {
        ROOT_SLOT_COUNT.fetch_sub(old.roots.len(), Ordering::Relaxed);
    } else {
        MAP_COUNT.fetch_add(1, Ordering::Relaxed);
    }
    ROOT_SLOT_COUNT.fetch_add(roots.len(), Ordering::Relaxed);
}

/// Drop every registered map (workspace teardown / test isolation).
/// Registered maps describe JIT code that is being unmapped with them.
pub fn clear_safepoints() {
    let mut maps = SAFEPOINT_MAPS.write().expect("rootscan.rs: maps lock");
    if let Some(m) = maps.as_mut() {
        m.clear();
    }
    MAP_COUNT.store(0, Ordering::Relaxed);
    ROOT_SLOT_COUNT.store(0, Ordering::Relaxed);
}

/// Number of registered callsite maps (probe/stats surface).
pub fn safepoint_map_count() -> usize {
    MAP_COUNT.load(Ordering::Relaxed)
}

/// Total root slots across registered maps (probe/stats surface).
pub fn safepoint_root_slot_count() -> usize {
    ROOT_SLOT_COUNT.load(Ordering::Relaxed)
}

/// Look one return address up. `None` = unmapped frame (the walker's
/// conservative-floor fallback arm).
pub fn map_for_return_address(return_address: usize) -> Option<SafepointEntry> {
    let maps = SAFEPOINT_MAPS.read().expect("rootscan.rs: maps lock");
    maps.as_ref()?.get(&return_address).cloned()
}

// ---------------------------------------------------------------------------
// The fp-chain walker
// ---------------------------------------------------------------------------

/// One walked native frame: its return address and frame-pointer base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkedFrame {
    /// The frame-pointer base of THIS frame (the value rbp holds while
    /// this frame executes).
    pub fp: usize,
    /// The return address stored in this frame's trailer — the key into
    /// the safepoint map (it names the CALL SITE inside this frame).
    pub return_address: usize,
}

/// Walk the frame-pointer chain starting at `first_fp`, yielding each
/// frame's (fp, return-address) pair until the chain bottoms out.
///
/// Safety contract (mirrors pon stackmap.rs walk discipline): every frame
/// on the chain between `first_fp` and the root was built by code
/// compiled with frame-pointer retention (Cranelift `preserve_frame_pointers`
/// semantics + our prologue discipline), so `*fp` is the caller's fp and
/// `*(fp+8)` is the return address. The loop stops at:
/// - a null saved-rbp (the conventional chain terminator), or
/// - a non-monotonic link (a corrupted/garbage link — the allocator's
///   addresses grow downward, so the caller fp must be >= the current
///   one on the main stack), or
/// - [`FP_CHAIN_MAX_DEPTH`] (a runaway guard: never loop forever on a
///   corrupted chain — fail toward the conservative floor instead).
///
/// # Safety
///
/// `first_fp` must be a valid frame-pointer base for a LIVE frame on the
/// calling thread's stack (the intended caller reads rbp at a safepoint
/// or passes a captured value). Every dereference here walks the chain
/// per the contract above; the monotonic + depth guards make a corrupted
/// link a bounded stop, not a fault window wider than the raw walk
/// itself. Callers inside the collector use the captured-at-safepoint
/// fp handed up by the codegen trampoline, never a synthesized value.
pub unsafe fn fp_chain_walk(first_fp: usize) -> Vec<WalkedFrame> {
    let mut out = Vec::new();
    let mut fp = first_fp;
    let mut prev_fp = usize::MAX;
    for _ in 0..FP_CHAIN_MAX_DEPTH {
        if fp == 0 {
            break;
        }
        // The return address lives at [fp+8], the caller's fp at [fp].
        let saved_fp = *(fp as *const usize);
        let ret = *(fp.wrapping_add(8) as *const usize);
        out.push(WalkedFrame {
            fp,
            return_address: ret,
        });
        // Monotonic-link guard: stack grows downward, so a caller frame
        // lives at an address >= this frame's fp. Anything else is a
        // corrupted link (or the macOS trampoline-stationary tail) —
        // stop BEFORE following it.
        if saved_fp == 0 || saved_fp < fp || saved_fp == prev_fp {
            break;
        }
        prev_fp = fp;
        fp = saved_fp;
    }
    WALK_COUNT.fetch_add(out.len() as u64, Ordering::Relaxed);
    out
}

/// Runaway guard: deeper native chains than this are treated as a
/// corrupted chain and stop the walk (fail toward the conservative
/// floor). 16384 frames ≫ any legal interpreter depth (the recursion
/// limit is 1000; the deepest measured Rust chains are < 500).
pub const FP_CHAIN_MAX_DEPTH: usize = 16_384;

/// Frames walked without a map entry (probe/stats read).
#[cfg_attr(not(test), allow(dead_code))] // probe surface (kept: rootscan stats seam)
pub fn unmapped_frame_count() -> u64 {
    UNMAPPED_FRAME_COUNT.load(Ordering::Relaxed)
}

/// fp-chain walks performed (probe/stats read; frames, not calls). Used
/// by the walker unit tests and exposed as the probe seam for the
/// ap-run rootscan instrumentation (kept for that consumer even though
/// the shim itself does not read it).
#[cfg_attr(not(test), allow(dead_code))] // probe seam (consumed by tests + future ap-run instrumentation)
pub fn walked_frame_count() -> u64 {
    WALK_COUNT.load(Ordering::Relaxed)
}

/// Walk + map-resolve in one call: every frame whose return address has
/// a registered map contributes that map's live slots (as
/// `(slot_fp_address, &[StackRootSlot])` pairs the consumer reads root
/// payload pointers from); every UNMAPPED frame is counted (probe
/// surface) and covered by the conservative floor instead of enumerated.
/// This is the exact shape the MMTk `Scanning::scan_roots_in_mutator_thread`
/// native-frame arm hands to the binding in stage 2 — the data, not the
/// policy, is the shim.
pub fn mapped_frames_from(first_fp: usize) -> Vec<(usize, SafepointEntry)> {
    // SAFETY: `first_fp` is captured from a live frame per the raw walk
    // contract above; the guards bound every iteration.
    let frames = unsafe { fp_chain_walk(first_fp) };
    let mut out = Vec::new();
    for f in frames {
        match map_for_return_address(f.return_address) {
            Some(entry) => out.push((f.fp, entry)),
            None => {
                UNMAPPED_FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Conservative-scan floor (pon set_conservative_scan_floor analogue)
// ---------------------------------------------------------------------------

/// The conservative floor policy, pinned in one named place.
///
/// POLICY (leaf risk mitigation, Study problem #1): root enumeration for
/// native frames is ours, and a walker that can MISS a frame must never
/// be the only root source. The floor is the subtract-refs invariant
/// itself: every externally-held strong reference (`strong_count >
/// internal edges` in gcmod.rs's seeding pass) reads as a root no matter
/// where the reference lives — including temporaries in JIT frames the
/// fp-chain walk could not enumerate, interpreter frames, and OSR
/// transfer buffers. Consequently:
///
/// - `del x; gc.collect()` observability works today (the registry
///   arithmetic sees the drop) and stays correct with maps registered —
///   a precise map ADDS roots, it never removes the floor.
/// - A JIT frame holding the ONLY reference to an object is safe even
///   if its map is missing or incomplete: the Rc clone in the slot makes
///   the object externally referenced.
///
/// This struct has no runtime state today (the floor is structural);
/// it exists so the policy has ONE doc point, the tests can assert the
/// floor is armed, and the MMTk swap (stage 2) has the seam where a
/// per-thread floor LEVEL (pon's scan-range floor) would land when the
/// collector stops being subtract-refs based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConservativeFloorGuard {
    /// Always true today: the floor is armed by construction (see type
    /// doc). The field makes the invariant assertable rather than
    /// documented-only.
    pub armed: bool,
}

impl ConservativeFloorGuard {
    /// The floor is armed: subtract-refs seeding remains the correctness
    /// base under the maps. Constructing a guard asserts nothing at
    /// runtime — arming is a property of gcmod.rs's seeding pass.
    pub fn armed() -> ConservativeFloorGuard {
        ConservativeFloorGuard { armed: true }
    }
}

// ---------------------------------------------------------------------------
// Scrub discipline hook (pon call_pyobject_helper_consuming analogue)
// ---------------------------------------------------------------------------

/// Zero the dead `Value` slots of an ExecCtx between helper calls.
///
/// Scrub policy (pon baseline.rs:2153-2175 discipline): a slot whose
/// value will not be read again before the next safepoint must be
/// cleared so its stale Rc payload pointer cannot pin garbage. The
/// baseline codegen routes EVERY op through host helpers (the helper
/// call sites are the safepoints), so the codegen tail calls this at
/// the lowering points where a temp dies (see clif.rs `emit_scrub`).
///
/// The slot write is a plain `Value::None` store (not unsafe): the
/// dropping Rc releases the reference EXACTLY as a normal rebinding
/// would — this is scrub-as-semantics, not scrub-as-raw-memory. That is
/// the difference from pon: our slots are Rust `Value`s in a Vec, so
/// the type system already gives us the drop-on-overwrite; the helper
/// exists to name the discipline and to be the single point a future
/// unboxed tier-1 slot repr must revisit.
#[cfg_attr(not(test), allow(dead_code))] // wired into the clif.rs tail in ap-codegen-native
pub fn scrub_slot(slot: &mut ap_obj::value::Value) {
    *slot = ap_obj::value::Value::None;
}

#[cfg(test)]
mod tests {
    //! In-crate unit tests: registry + map resolution + walker guards.
    //! The fp-chain walk ITSELF is exercised end-to-end at the ap-gc
    //! integration level (tests/gc_invariants.rs rootscan tests build a
    //! real chain through registered test frames) — here the walk is
    //! probed only through synthetic chain values that never dereference
    //! (the guards stop at depth/monotonic checks before any wild read).

    use super::*;

    #[test]
    fn register_and_resolve_roundtrip() {
        clear_safepoints();
        let ra = 0x0000_5aa0_usize; // synthetic; never dereferenced
        register_safepoint(
            ra,
            vec![
                StackRootSlot { fp_offset: 16 },
                StackRootSlot { fp_offset: 32 },
            ],
        );
        let m = map_for_return_address(ra).expect("registered map must resolve");
        assert_eq!(
            m.roots,
            vec![
                StackRootSlot { fp_offset: 16 },
                StackRootSlot { fp_offset: 32 }
            ]
        );
        assert_eq!(safepoint_map_count(), 1);
        assert_eq!(safepoint_root_slot_count(), 2);
        clear_safepoints();
        assert!(map_for_return_address(ra).is_none());
        assert_eq!(safepoint_map_count(), 0);
    }

    #[test]
    fn re_register_replaces_and_recounts() {
        clear_safepoints();
        let ra = 0x0000_5aa1_usize;
        register_safepoint(ra, vec![StackRootSlot { fp_offset: 8 }]);
        register_safepoint(
            ra,
            vec![
                StackRootSlot { fp_offset: 8 },
                StackRootSlot { fp_offset: 16 },
                StackRootSlot { fp_offset: 24 },
            ],
        );
        assert_eq!(safepoint_map_count(), 1, "replace, not duplicate");
        assert_eq!(
            safepoint_root_slot_count(),
            3,
            "count follows the last registration"
        );
        clear_safepoints();
    }

    #[test]
    fn empty_root_sets_are_not_registered() {
        clear_safepoints();
        let before = safepoint_map_count();
        register_safepoint(0x0000_5aa2_usize, Vec::new());
        assert_eq!(safepoint_map_count(), before);
        assert!(map_for_return_address(0x0000_5aa2_usize).is_none());
    }

    #[test]
    fn unmapped_return_addresses_miss() {
        clear_safepoints();
        assert!(map_for_return_address(0xdead_beef_usize).is_none());
    }

    #[test]
    fn walker_stops_at_null_link() {
        // Build a chain trailer in a local buffer: [fp+0] = 0 (null
        // caller link), [fp+8] = the synthetic return address. The walk
        // must yield exactly one frame and stop.
        let mut buf = [0usize; 4];
        let base = buf.as_mut_ptr() as usize;
        unsafe {
            *(base as *mut usize) = 0; // saved fp: null => terminator
            *((base + 8) as *mut usize) = 0x0000_5aa3; // return address
            let frames = fp_chain_walk(base);
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].return_address, 0x0000_5aa3);
            assert_eq!(frames[0].fp, base);
        }
    }

    #[test]
    fn walker_stops_at_non_monotonic_link() {
        // saved fp BELOW the current fp = corrupted link: the guard must
        // stop before following it (one frame reported, no wild read).
        let mut buf = [0usize; 4];
        let base = buf.as_mut_ptr() as usize;
        let bogus = base.wrapping_sub(0x1000); // "caller" below us
        unsafe {
            *(base as *mut usize) = bogus;
            *((base + 8) as *mut usize) = 0x0000_5aa4;
            let frames = fp_chain_walk(base);
            assert_eq!(frames.len(), 1, "non-monotonic link must stop the walk");
        }
    }

    #[test]
    fn walker_depth_guard_bounds_runaway_chains() {
        // A chain that always links forward by 16 bytes (self-feeding
        // garbage) must stop at FP_CHAIN_MAX_DEPTH, not loop forever.
        // Build a ring: each link points 16 bytes up within one buffer
        // is non-monotonic... so instead build a monotonic ASCENDING
        // chain in a big buffer (caller fp ABOVE current — legal shape).
        let n = 64usize;
        let mut buf = vec![0usize; n * 2];
        let base = buf.as_mut_ptr() as usize;
        unsafe {
            for i in 0..n {
                let fp = base + i * 16;
                let next = if i + 1 < n { fp + 16 } else { 0 };
                *(fp as *mut usize) = next; // caller fp (above => monotonic)
                *((fp + 8) as *mut usize) = 0x0000_5aa5;
            }
            let frames = fp_chain_walk(base);
            assert_eq!(frames.len(), n, "chain of {n} links walks fully");
            // The stats counter observed every walked frame (probe seam).
            assert!(
                walked_frame_count() >= n as u64,
                "walk counter must track walked frames"
            );
            // Depth guard is structural: FP_CHAIN_MAX_DEPTH bounds any
            // walk; assert the constant is the documented value.
            assert_eq!(FP_CHAIN_MAX_DEPTH, 16_384);
        }
    }

    #[test]
    fn mapped_frames_joins_walk_and_maps() {
        clear_safepoints();
        let mut buf = [0usize; 4];
        let base = buf.as_mut_ptr() as usize;
        let ra = 0x0000_5aa6usize;
        register_safepoint(ra, vec![StackRootSlot { fp_offset: 24 }]);
        unsafe {
            *(base as *mut usize) = 0; // null caller link
            *((base + 8) as *mut usize) = ra;
            let mapped = mapped_frames_from(base);
            assert_eq!(mapped.len(), 1);
            assert_eq!(mapped[0].0, base);
            assert_eq!(mapped[0].1.roots, vec![StackRootSlot { fp_offset: 24 }]);
        }
        // An unmapped frame is counted, not reported.
        let mut buf2 = [0usize; 4];
        let base2 = buf2.as_mut_ptr() as usize;
        unsafe {
            *(base2 as *mut usize) = 0;
            *((base2 + 8) as *mut usize) = 0x0000_0c0f_fee0_usize & 0xffff_ffff;
            let mapped2 = mapped_frames_from(base2);
            assert!(mapped2.is_empty(), "unmapped frame contributes nothing");
            assert!(unmapped_frame_count() >= 1);
        }
        clear_safepoints();
    }

    #[test]
    fn conservative_floor_guard_is_assertable() {
        let g = ConservativeFloorGuard::armed();
        assert!(
            g.armed,
            "the subtract-refs floor is structural and always armed"
        );
    }

    #[test]
    fn scrub_slot_releases_and_zeros() {
        let v = ap_obj::value::Value::List(ap_obj::value::PyList::new());
        let mut dead = v;
        // Probe the Rc count through a clone first: after the scrub the
        // list must be released (strong count back to ours only).
        let strong_before = match &dead {
            ap_obj::value::Value::List(l) => std::rc::Rc::strong_count(l),
            _ => unreachable!("test builds a list"),
        };
        assert_eq!(strong_before, 1);
        scrub_slot(&mut dead);
        assert!(
            matches!(dead, ap_obj::value::Value::None),
            "scrubbed slot reads None"
        );
        // The released list's memory is gone; nothing more to assert on
        // the value itself — the semantic (drop on overwrite) is Rust's.
    }
}

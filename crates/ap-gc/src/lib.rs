//! ap-gc — the antipythony cycle collector (leaf 02.04).
//!
//! STAGE 1 (this leaf): a CPython-shaped Arc-cycle collector over the
//! interpreter's object graph — allocation registry, subtract-refs
//! reachability, once-only finalizers, weakref liveness, and the state for
//! the `gc` module surface ap-run builds. `Gc<T>` stays `Arc<T>`; the MMTk
//! swap is designed in docs/gc-mmtk-swap.md (stage 2).
//!
//! STAGE 2 (future): the MMTk binding + heap policies behind trait Heap
//! land here, replacing the registry with MMTk root reporting.

mod census;
mod gcmod;
mod rootscan;
mod weakbridge;

pub use census::{
    active_capture_points, live_mutators, my_capture_slot, register, set_thread_id, unregister,
};

/// 02.09b: the worker-registration drain seam. ap-run installs a drain fn
/// at bridge-install time (draining its per-worker forward channels into
/// the process registry); every collect calls this FIRST so a worker's
/// allocations are visible to the pass scope. Without an installer (pure
/// ap-gc tests) this is a no-op.
// Issue #23 real fix (inventory category c): was AtomicUsize + usize->fn
// reinterpretation; OnceLock stores the typed fn directly, so the call
// path is fully safe. Publication semantics are unchanged (OnceLock is
// atomic internally); the first install wins and every in-repo caller
// installs the same fn (ap-run::gcsupport::install — single call site),
// so no overwriting behavior is lost.
static BRIDGE_DRAIN: std::sync::OnceLock<fn() -> usize> = std::sync::OnceLock::new();

/// Install the drain fn (ap-run::gcsupport, once at run entry).
pub fn set_bridge_drain(f: fn() -> usize) {
    let _ = BRIDGE_DRAIN.set(f);
}

fn gc_bridge_drain() -> usize {
    match BRIDGE_DRAIN.get() {
        Some(f) => f(),
        None => 0,
    }
}

pub use gcmod::{
    add_root_dict, add_root_module_ns, callbacks_set_list, callbacks_snapshot, clear_roots,
    collect, collect_gen, disable, enable, flush_test, freeze, gc_register, gc_tracked_len,
    get_count, get_object_by_addr, get_objects, get_threshold, is_enabled, is_tracked,
    reset_for_test, set_threshold, stats, unfreeze, weakref_add, weakref_alive, weakref_count_for,
    weakref_entry_count, weakref_is_dead, weakref_next_seq, weakref_prune_fired, weakref_take_dead,
    weakref_tokens_for, GcStats, WeakEntry,
};

/// Leaf 02.05g: the GC roots shim — safepoint-map registry + fp-chain
/// walker (the MMTk root feed for native JIT frames).
pub use rootscan::{
    caller_frame_pointer, capture_point, clear_capture_point, clear_safepoints, fp_chain_walk,
    map_for_return_address, mapped_frames_from, register_safepoint, safepoint_map_count,
    safepoint_root_slot_count, set_capture_point, ConservativeFloorGuard, StackRootSlot,
    WalkedFrame, FP_CHAIN_MAX_DEPTH,
};

/// The bridge module path (ap-run installs via weakbridge::set_bridge).
pub use weakbridge::set_bridge;

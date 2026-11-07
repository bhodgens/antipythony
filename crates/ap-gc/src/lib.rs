//! ap-gc — the antipythony cycle collector (leaf 02.04).
//!
//! STAGE 1 (this leaf): a CPython-shaped Rc-cycle collector over the
//! interpreter's object graph — allocation registry, subtract-refs
//! reachability, once-only finalizers, weakref liveness, and the state for
//! the `gc` module surface ap-run builds. `Gc<T>` stays `Rc<T>`; the MMTk
//! swap is designed in docs/gc-mmtk-swap.md (stage 2).
//!
//! STAGE 2 (future): the MMTk binding + heap policies behind trait Heap
//! land here, replacing the registry with MMTk root reporting.

mod gcmod;
mod weakbridge;

pub use gcmod::{
    add_root_dict, add_root_module_ns, callbacks_set_list, callbacks_snapshot, clear_roots,
    collect, collect_gen, disable, enable, freeze, gc_register, gc_tracked_len, get_count,
    get_object_by_addr, get_objects, get_threshold, is_enabled, is_tracked, set_threshold, stats,
    unfreeze, weakref_add, weakref_alive, weakref_count_for, weakref_entry_count, weakref_is_dead,
    weakref_next_seq, weakref_prune_fired, weakref_take_dead, weakref_tokens_for, GcStats,
    WeakEntry,
};

/// The bridge module path (ap-run installs via weakbridge::set_bridge).
pub use weakbridge::set_bridge;

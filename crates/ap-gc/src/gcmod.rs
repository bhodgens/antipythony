//! ap-gc — the cycle collector (leaf 02.04 STAGE 1).
//!
//! STAGE 1 keeps `Gc<T> = Rc<T>` (the MMTk swap is STAGE 2,
//! docs/gc-mmtk-swap.md) and adds a CPython-shaped CYCLE COLLECTOR:
//!
//! - A global REGISTRY of every GC-tracked allocation (`Value` clones keyed
//!   by payload address; `ap-obj::gc_gate::gc_register` feeds it at each
//!   container/object allocation site).
//! - `collect()` uses CPython's SUBTRACT-REFS invariant (gcmodule.c
//!   `subtract_refs`+`move_unreachable` collapsed into one pass): an object
//!   whose Rc strong count exceeds the number of inbound registry-graph
//!   edges has an external referee (root dict, live frame, native stack
//!   temporary) and is reachable; the rest are garbage. This needs NO root
//!   scanning of the Rust stack — every mutator reference outside the
//!   object graph simply shows up as `strong_count > internal_edges`,
//!   which is why the algorithm is safe for an Rc-hosting interpreter.
//! - Unreachable acyclic garbage dies by the plain Rc cascade once the
//!   registry clone drops; true cycles are broken because every member's
//!   registry clone was one of its inbound edges and all are dropped in
//!   the same pass.
//! - Finalizers (`__del__`) fire exactly once per object via the ap-run
//!   bridge at sweep (C-20/C-21); weakref callbacks fire once when the
//!   target dies (C-22); gc.callbacks observe start/stop (gc006).
//!
//! NOT in stage 1: generations (threshold t1/t2 stored for round-trip
//! only), moving collection, thread handshakes (single-threaded v0).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::Value;

pub(crate) use crate::weakbridge;

/// One tracked allocation. The registry holds a `Value` clone — one strong
/// count above the mutator's references — exactly the role of CPython's gc
/// header membership.
pub(crate) struct Tracked {
    pub value: Value,
}

thread_local! {
    /// The allocation registry: address -> tracked handle. Address keys are
    /// Rc payload pointers, stable for the object's life (the registry
    /// clone keeps the count >= 1, so addresses are never recycled while
    /// tracked — C-21's no-slot-reuse-re-fire requirement).
    static REGISTRY: RefCell<HashMap<usize, Tracked>> = RefCell::new(HashMap::new());

    /// gc.set_threshold/get_threshold (t0 = objects between auto-collections;
    /// t1/t2 stored for round-trip only — stage 1 has no generation model).
    static THRESHOLDS: std::cell::Cell<(usize, usize, usize)> =
        const { std::cell::Cell::new((700, 10, 10)) };

    /// Allocations since the last collection (gc.get_count()[0]).
    static ALLOC_SINCE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };

    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };

    /// gc.callbacks observers (callables; fired with (phase, info-dict)).
    pub(crate) static CALLBACKS: RefCell<Vec<Value>> = RefCell::new(Vec::new());

    /// gc.freeze/gc.unfreeze: addresses excluded from sweeping.
    static FROZEN: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());

    /// Weakrefs: the ap-run weakref surface records (target-addr, token,
    /// callback) here; the collector owns liveness and dead-notifications.
    pub(crate) static WEAKREFS: RefCell<Vec<WeakEntry>> = RefCell::new(Vec::new());

    /// Addresses whose finalizer already ran: never fire again (C-21).
    /// (Non-resurrected ones die at the sweep that ran them; resurrected
    /// ones stay tracked and die silently at a later sweep.)
    static FINALIZED_ADDRS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());

    /// Addresses judged unreachable in the CURRENT collection (swept at the
    /// retain below). sweep_weakrefs needs this because it runs before the
    /// retain: registry membership alone can't see this cycle's deaths.
    static GARBAGE_NOW: RefCell<Vec<usize>> = RefCell::new(Vec::new());

    /// Instrumentation totals (GCSTATS env / stress report).
    static STATS: std::cell::Cell<GcStats> = const { std::cell::Cell::new(GcStats::EMPTY) };

    /// Registration sequence (creation order for finalizer ordering, gc002).
    static SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SEQ_OF: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
    /// F2: monotonically increasing weakref-registration id (per thread).
    static WEAK_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Root dicts (module globals, builtins) wired by ap-run at entry.
    static ROOT_DICTS: RefCell<Vec<Gc<PyDict>>> = RefCell::new(Vec::new());
    /// Root module namespaces.
    static ROOT_MODULE_NS: RefCell<Vec<std::rc::Rc<PyDict>>> = RefCell::new(Vec::new());
}

/// A registered weak reference: the target address it watches, the token
/// `Value` handed to Python, and the optional callback. `dead` flips at
/// collect; ap-run fires callbacks and answers `ref()` calls from liveness.
pub struct WeakEntry {
    pub target: usize,
    pub token: Value,
    pub callback: Option<Value>,
    pub dead: bool,
    pub fired: bool,
    /// OOM-wave-1 (F2): identity of this registration = (registration seq,
    /// registration thread) — weakref_count_for/weakref_tokens_for index by
    /// creation order per target, and ap-run's WEAK_TOKENS carry the same
    /// seq so dead pairs can be pruned on both sides. Stable even while
    /// earlier entries are swap-removed out of the Vec.
    pub seq: u64,
}

#[derive(Clone, Copy, Default)]
pub struct GcStats {
    pub collections: u64,
    pub objects_reclaimed: u64,
    pub cycles_collected: u64,
}

impl GcStats {
    const EMPTY: GcStats = GcStats {
        collections: 0,
        objects_reclaimed: 0,
        cycles_collected: 0,
    };
    fn record(&mut self, reclaimed: u64, cycles: u64) {
        self.collections += 1;
        self.objects_reclaimed += reclaimed;
        self.cycles_collected += cycles;
    }
}

// ---------------------------------------------------------------------------
// Registration (ap-obj allocation sites -> ap_obj::gc_gate)
// ---------------------------------------------------------------------------

/// Register a freshly allocated container/object. Keyed by payload address;
/// duplicate registrations of a live object are no-ops. Registration is
/// suppressed while the collector is running finalizers (objects born in a
/// `__del__` are seen by the NEXT pass).
pub fn gc_register(v: &Value) {
    if weakbridge::suppress_registration() {
        return;
    }
    let key = value_addr(v);
    if key == 0 {
        return;
    }
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        if !r.contains_key(&key) {
            let n = SEQ.get() + 1;
            SEQ.set(n);
            SEQ_OF.with(|s| s.borrow_mut().insert(key, n));
            r.insert(key, Tracked { value: v.clone() });
            // New object -> young. The next gen-0 pass scans it (and
            // graduates it); full passes re-seed YOUNG from the registry.
            YOUNG.with(|y| y.borrow_mut().insert(key));
        }
    });
    // Automatic collection at the generation-0 threshold (tunable via
    // gc.set_threshold / APX_GC_THRESHOLD env read by ap-run at entry).
    let n = ALLOC_SINCE.get() + 1;
    ALLOC_SINCE.set(n);
    let (t0, t1, t2) = THRESHOLDS.get();
    if ENABLED.get() && t0 > 0 && n >= t0 {
        ALLOC_SINCE.set(0);
        // Automatic threshold fires are GENERATIONAL with a CASCADE (the
        // CPython design the pre-2026-11-05 code was missing in both
        // directions):
        // - plain full-heap scan every t0 allocations was O(n²) over
        //   steady-allocation workloads (3M-object probe never finished);
        // - gen-0-ONLY scans (first fix) let objects that die OLD keep
        //   their registry Value clones forever — the residual leak.
        // CPython's ladder: every t1 gen-0 passes fire a gen-1 pass; every
        // t2 gen-1 passes fire a full gen-2 pass. THRESHOLDS already
        // carries (t0, t1, t2) = (700, 10, 10).
        let g0 = GEN0_COUNT.get();
        let g1 = GEN1_COUNT.get();
        let reg_len = REGISTRY.with(|r| r.borrow().len());
        let last_full = LAST_FULL_LEN.get();
        // Growth-triggered major GC: registry doubled since the last full
        // pass (floor 100k entries so small heaps keep the cheap ladder).
        // Amortized O(1): a doubling epoch pays one full scan, the same
        // bound real-time collectors use.
        let growth_major = reg_len > 100_000 && reg_len > last_full.saturating_mul(2);
        if growth_major || g1 >= t2.max(1) {
            GEN1_COUNT.set(0);
            GEN0_COUNT.set(0);
            LAST_FULL_LEN.set(reg_len);
            collect_gen(2); // full: reclaims old garbage + re-seeds YOUNG
        } else if g0 >= t1.max(1) {
            GEN1_COUNT.set(g1 + 1);
            GEN0_COUNT.set(0);
            collect_gen(1); // gen-1: scans young + mid-age (see MID set)
        } else {
            GEN0_COUNT.set(g0 + 1);
            collect_gen(0); // gen-0: young only
        }
    }
}

pub fn gc_tracked_len() -> usize {
    REGISTRY.with(|r| r.borrow().len())
}

fn value_addr(v: &Value) -> usize {
    match v {
        Value::Str(s) => rc_addr(s),
        Value::Bytes(b) => rc_addr(b),
        Value::List(l) => rc_addr(l),
        Value::Tuple(t) => rc_addr(t),
        Value::Dict(d) => rc_addr(d),
        Value::Set(s) | Value::Frozen(s) => rc_addr(s),
        Value::Obj(o) => rc_addr(o),
        _ => 0,
    }
}

fn rc_addr<T>(r: &Gc<T>) -> usize {
    std::rc::Rc::as_ptr(r) as *const u8 as usize
}

fn strong_count<T>(r: &Gc<T>) -> usize {
    std::rc::Rc::strong_count(r)
}

// ---------------------------------------------------------------------------
// Roots
// ---------------------------------------------------------------------------

/// Register a root dict (module globals, builtins). ap-run calls at entry.
pub fn add_root_dict(d: &Gc<PyDict>) {
    ROOT_DICTS.with(|r| {
        let mut r = r.borrow_mut();
        if !r.iter().any(|x| rc_addr(x) == rc_addr(d)) {
            r.push(d.clone());
        }
    });
}

/// Register a module namespace root.
pub fn add_root_module_ns(ns: &std::rc::Rc<PyDict>) {
    ROOT_MODULE_NS.with(|r| {
        let mut r = r.borrow_mut();
        let ptr = std::rc::Rc::as_ptr(ns) as *const u8 as usize;
        if !r
            .iter()
            .any(|x| std::rc::Rc::as_ptr(x) as *const u8 as usize == ptr)
        {
            r.push(ns.clone());
        }
    });
}

pub fn clear_roots() {
    ROOT_DICTS.with(|r| r.borrow_mut().clear());
    ROOT_MODULE_NS.with(|r| r.borrow_mut().clear());
}

// ---------------------------------------------------------------------------
// The collector
// ---------------------------------------------------------------------------

/// Outbound edges of a value: every registry-tracked child it holds.
fn edges_of(v: &Value, registry: &HashMap<usize, Tracked>, out: &mut Vec<usize>) {
    fn tracked_addr(val: &Value, registry: &HashMap<usize, Tracked>) -> Option<usize> {
        let a = value_addr(val);
        if a != 0 && registry.contains_key(&a) {
            Some(a)
        } else {
            None
        }
    }
    match v {
        Value::List(l) => {
            for item in l.items().iter() {
                if let Some(a) = tracked_addr(item, registry) {
                    out.push(a);
                }
            }
        }
        Value::Tuple(t) => {
            for item in t.0.iter() {
                if let Some(a) = tracked_addr(item, registry) {
                    out.push(a);
                }
            }
        }
        Value::Dict(d) => {
            for (k, val) in d.items() {
                if let Some(a) = tracked_addr(&k, registry) {
                    out.push(a);
                }
                if let Some(a) = tracked_addr(&val, registry) {
                    out.push(a);
                }
            }
        }
        Value::Set(s) | Value::Frozen(s) => {
            for e in s.iter() {
                if let Some(a) = tracked_addr(&e, registry) {
                    out.push(a);
                }
            }
        }
        Value::Obj(o) => {
            // attrs dict (identity per object) + payload values.
            // Contention pre-check: if ANY field is borrowed elsewhere
            // (heap mid-mutation), abort this scan cycle via the flag.
            if o.attrs.try_borrow().is_err()
                || o.func.try_borrow().is_err()
                || o.self_value.try_borrow().is_err()
                || o.cause.try_borrow().is_err()
                || o.context.try_borrow().is_err()
                || o.dunders.try_borrow().is_err()
            {
                SCAN_CONTENDED.with(|c| c.set(true));
                return;
            }
            // try_borrow: GC can trigger while another borrow is live
            // (e.g. class-body exec mid-__setitem__); skip contended objects
            // (conservative - they are reachable from the live frame anyway).
            let Ok(attrs_guard) = o.attrs.try_borrow() else {
                SCAN_CONTENDED.with(|c| c.set(true));
                return;
            };
            let a = rc_addr(&attrs_guard);
            if registry.contains_key(&a) {
                out.push(a);
            }
            let attrs = attrs_guard.clone();
            for (k, v) in attrs.items() {
                if let Some(a) = tracked_addr(&k, registry) {
                    out.push(a);
                }
                if let Some(a) = tracked_addr(&v, registry) {
                    out.push(a);
                }
            }
            // try_borrow: GC can trigger while another borrow is live
            // (class-body exec mid-__setitem__); contended => skip (the
            // object is reachable from a live frame anyway).
            if let Ok(f_guard) = o.func.try_borrow() {
                if let Some(f) = f_guard.as_ref() {
                    if let Some(a) = tracked_addr(f, registry) {
                        out.push(a);
                    }
                }
            }
            if let Ok(s_guard) = o.self_value.try_borrow() {
                if let Some(s) = s_guard.as_ref() {
                    if let Some(a) = tracked_addr(s, registry) {
                        out.push(a);
                    }
                }
            }
            if let Ok(c_guard) = o.cause.try_borrow() {
                if let Some(c) = c_guard.as_ref() {
                    if let Some(a) = tracked_addr(c, registry) {
                        out.push(a);
                    }
                }
            }
            if let Ok(cx_guard) = o.context.try_borrow() {
                if let Some(c) = cx_guard.as_ref() {
                    if let Some(a) = tracked_addr(c, registry) {
                        out.push(a);
                    }
                }
            }
            // UserClass method table (dunders dict).
            if let Ok(dunders) = o.dunders.try_borrow() {
                for (k, v) in dunders.items() {
                    if let Some(a) = tracked_addr(&k, registry) {
                        out.push(a);
                    }
                    if let Some(a) = tracked_addr(&v, registry) {
                        out.push(a);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Count inbound internal edges per registry entry (the graph's fan-in).
/// Scoped mode (`scope = Some(young)`): only young objects are counted as
/// graph nodes — edges INTO them are counted, edges into old objects are
/// dropped. Old->young edges therefore read as EXTERNAL references on the
/// young object (its strong count exceeds its internal edge count), which
/// seeds it reachable: a young object an old object still holds must not
/// be swept by the gen-0 pass (CPython's identical deferred rule).
fn count_internal_refs_scoped(
    registry: &HashMap<usize, Tracked>,
    counts: &mut HashMap<usize, usize>,
    scope: Option<&HashSet<usize>>,
) {
    counts.clear();
    // Init counts ONLY for scope addresses: iterating registry.keys() with
    // a per-key probe was the third O(heap) per-pass cost. Old objects need
    // no count entry — scan_edges drops their edges (counts.get_mut None),
    // which is the scoped semantics.
    match scope {
        None => {
            for addr in registry.keys() {
                counts.insert(*addr, 0);
            }
        }
        Some(s) => {
            for addr in s.iter() {
                counts.insert(*addr, 0);
            }
        }
    }
    // Iterate the SCOPE addresses (gen-0), not the registry: the scope is
    // proportional to the young set, while registry.values() iteration is
    // proportional to the whole heap (the O(n) per-pass cost that kept the
    // gen-0 pass quadratic).
    match scope {
        None => {
            for t in registry.values() {
                scan_edges(&t.value, registry, counts);
            }
        }
        Some(s) => {
            for addr in s.iter() {
                if let Some(t) = registry.get(addr) {
                    scan_edges(&t.value, registry, counts);
                }
            }
        }
    }
}

/// Edge-count one tracked object's children into `counts`.
fn scan_edges(v: &Value, registry: &HashMap<usize, Tracked>, counts: &mut HashMap<usize, usize>) {
    let mut kids = Vec::new();
    edges_of(v, registry, &mut kids);
    for k in kids {
        if let Some(c) = counts.get_mut(&k) {
            *c += 1;
        }
    }
}

/// Addr of a tracked value without touching the registry map itself.
fn value_addr_of_tracked(t: &Tracked) -> usize {
    value_addr(&t.value)
}

/// Unscoped wrappers (full-heap scan): unit tests and any legacy callers.
fn count_internal_refs(registry: &HashMap<usize, Tracked>, counts: &mut HashMap<usize, usize>) {
    count_internal_refs_scoped(registry, counts, None)
}

fn mark_reachable(
    registry: &HashMap<usize, Tracked>,
    counts: &HashMap<usize, usize>,
) -> HashSet<usize> {
    mark_reachable_scoped(registry, counts, None)
}

/// Reachable set: walk from the roots through registry edges. An entry with
/// `strong_count > internal_edges` is externally referenced — seed the walk
/// from those (CPython's move_unreachable frontier) AND from the explicit
/// root dicts/module namespaces (belt and braces: roots may hold objects
/// through non-registry channels like module_ns Rc clones).
/// Scoped mode (`scope = Some(young)`): seeds and walk cover only young
/// objects. An old->young edge is invisible to `counts`, so the young
/// object's strong count exceeds its (lower) internal count and it seeds —
/// the CPython deferred-reclamation rule without scanning old objects.
fn mark_reachable_scoped(
    registry: &HashMap<usize, Tracked>,
    counts: &HashMap<usize, usize>,
    scope: Option<&HashSet<usize>>,
) -> HashSet<usize> {
    let in_scope = |addr: usize| scope.map_or(true, |s| s.contains(&addr));
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: Vec<usize> = Vec::new();

    // Externally-referenced seeds (subtract-refs invariant). The registry
    // clone itself holds one strong count, so an entry is externally
    // referenced iff sc - 1 (registry) > internal (graph edges): something
    // outside the tracked graph (root dict, live frame, native stack
    // temporary) holds a reference. Scoped mode: iterate SCOPE addrs only.
    match scope {
        None => {
            for (addr, t) in registry.iter() {
                let sc = strong_count_of(&t.value);
                let internal = counts.get(addr).copied().unwrap_or(0);
                if sc > internal + 1 {
                    queue.push(*addr);
                }
            }
        }
        Some(s) => {
            for addr in s.iter() {
                let Some(t) = registry.get(addr) else {
                    continue;
                };
                let sc = strong_count_of(&t.value);
                let internal = counts.get(addr).copied().unwrap_or(0);
                if sc > internal + 1 {
                    queue.push(*addr);
                }
            }
        }
    }

    // Explicit roots (globals dicts, module namespaces, sys.modules).
    let mut root_vals: Vec<Value> = Vec::new();
    ROOT_DICTS.with(|r| {
        for d in r.borrow().iter() {
            for (k, v) in d.items() {
                root_vals.push(k);
                root_vals.push(v);
            }
        }
    });
    ROOT_MODULE_NS.with(|r| {
        for ns in r.borrow().iter() {
            for (k, v) in ns.items() {
                root_vals.push(k);
                root_vals.push(v);
            }
        }
    });
    if let Some(mods) = weakbridge::sys_modules_values() {
        root_vals.extend(mods);
    }
    if let Some(frames) = weakbridge::frame_values_roots() {
        root_vals.extend(frames);
    }
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::GcDump) {
        let mut root_src = 0;
        for v in &root_vals {
            let a = value_addr(v);
            if a != 0 && registry.contains_key(&a) {
                root_src += 1;
            }
        }
        let sc_seeds = registry
            .iter()
            .filter(|(addr, t)| {
                let sc = strong_count_of(&t.value);
                let internal = counts.get(*addr).copied().unwrap_or(0);
                sc > internal
            })
            .count();
        eprintln!(
            "GC ROOTS: root_hits={root_src} sc_seeds={sc_seeds} sys_modules={}",
            weakbridge::sys_modules_values()
                .map(|v| v.len())
                .unwrap_or(0)
        );
    }
    for v in &root_vals {
        let a = value_addr(v);
        if a != 0 && registry.contains_key(&a) && in_scope(a) {
            queue.push(a);
        }
    }

    while let Some(addr) = queue.pop() {
        if !visited.insert(addr) {
            continue;
        }
        if let Some(t) = registry.get(&addr) {
            let mut kids = Vec::new();
            edges_of(&t.value, registry, &mut kids);
            for k in kids {
                if !visited.contains(&k) && in_scope(k) {
                    queue.push(k);
                }
            }
        }
    }
    visited
}

fn strong_count_of(v: &Value) -> usize {
    match v {
        Value::Str(s) => strong_count(s),
        Value::Bytes(b) => strong_count(b),
        Value::List(l) => strong_count(l),
        Value::Tuple(t) => strong_count(t),
        Value::Dict(d) => strong_count(d),
        Value::Set(s) | Value::Frozen(s) => strong_count(s),
        Value::Obj(o) => strong_count(o),
        _ => 1,
    }
}

thread_local! {
    /// Set when a scan hit a contended borrow (heap mid-mutation). collect()
    /// checks and aborts the cycle rather than collecting with a partial view.
    static SCAN_CONTENDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Objects registered since the last gen-0 (automatic) pass. Gen-0
    /// collections scan ONLY these; full passes scan the whole registry.
    /// Maintained by gc_register (insert) and collect_gen (drain on gen-0,
    /// refill from the registry on full passes).
    static YOUNG: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// Mid-age objects: promoted by gen-0 passes, NOT yet scanned by a
    /// gen-1 pass. A gen-1 pass scans YOUNG ∪ MID and clears MID (its
    /// survivors are old until the next full pass). Maintained by
    /// collect_gen only.
    static MID: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// Completed gen-0 passes since the last gen-1 pass (cascade counter,
    /// CPython gc.generations[0].count). At t1, a gen-1 pass fires.
    static GEN0_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Completed gen-1 (or higher) passes since the last full pass
    /// (gc.generations[1].count). At t2, a full gen-2 pass fires.
    static GEN1_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Registry length at the last growth-triggered or ladder full pass.
    /// When the registry DOUBLES since then (and is large), the next
    /// auto-fire escalates to a full pass regardless of the count ladder:
    /// a heap-growth-triggered major GC (browser-style). This bounds
    /// memory under slow-leak workloads without scanning on every fire.
    static LAST_FULL_LEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One FULL collection (explicit gc.collect()). Returns the number of
/// unreachable objects reclaimed (CPython's gc.collect() return: OBJECTS,
/// not cycles — gc011 pins `found >= 10` on a 10-node cycle).
pub fn collect() -> usize {
    collect_gen(2)
}

/// collect(generation): the generation rides the callbacks info dict
/// (gc006: collect(0)/collect(1) report the REQUESTED generation).
///
/// Gen-0 semantics (2026-11-05 O(n²) fix): when `generation == 0`, the
/// scan is scoped to the objects registered SINCE the last gen-0 pass
/// (YOUNG set) — the corpus pins identical OUTPUT (same objects die:
/// young garbage is either unreachable young cycles or members of older
/// garbage nobody touches), but the walk no longer touches the whole
/// registry, so N allocations cost O(N) total instead of O(N²). Full
/// passes (generation >= 1) scan everything, exactly as before.
pub fn collect_gen(generation: usize) -> usize {
    let _guard = weakbridge::CollectGuard::new();

    // 1. Internal-edge counts + reachable set. The registry borrow is held
    //    inside each step (no registration while the CollectGuard is up,
    //    so the snapshot is steady). Gen-0: the scan functions are scoped
    //    to the YOUNG set — counts/mark/sweep stay proportional to the
    //    young set, not the heap. The registry is borrowed, NEVER cloned:
    //    snapshot clones would inflate every strong_count by one and
    //    poison the sc > internal + 1 external-reference seeding.
    let scope: Option<HashSet<usize>> = match generation {
        0 => Some(YOUNG.with(|y| y.borrow().clone())),
        1 => {
            // Gen-1: young + mid-age. Old objects (already scanned by a
            // gen-1 or full pass) stay out of scope; the next FULL pass
            // catches garbage among them.
            let mut s = MID.with(|m| m.borrow().clone());
            let young: Vec<usize> = YOUNG.with(|y| y.borrow().iter().copied().collect());
            s.extend(young);
            Some(s)
        }
        _ => None,
    };
    let mut counts: HashMap<usize, usize> = HashMap::new();
    REGISTRY.with(|r| count_internal_refs_scoped(&r.borrow(), &mut counts, scope.as_ref()));
    let reachable = REGISTRY.with(|r| mark_reachable_scoped(&r.borrow(), &counts, scope.as_ref()));
    if SCAN_CONTENDED.with(|c| c.get()) {
        SCAN_CONTENDED.with(|c| c.set(false));
        return 0; // heap mid-mutation; retry at next threshold
    }

    // 2. Sweep list: drop registry handles for unreachable, unfrozen
    //    objects. Acyclic garbage dies by the Rc cascade when the LAST
    //    clone (the registry's) drops in the retain below; cycles break
    //    because all member clones drop together. We collect the garbage
    //    Values first so finalizers can still see their objects.
    let frozen = FROZEN.with(|f| f.borrow().clone());
    // gc002 pins CPython's creation-order finalizer firing: sort garbage by
    // registration sequence (the plain registry HashMap iteration order is
    // randomly seeded per process and does NOT correlate with creation).
    // Gen-0 scope: only YOUNG objects are candidates; unreachable OLD
    // objects are untouched this pass (they keep their registry handles).
    let mut garbage: Vec<(usize, u64, Value)> = Vec::new();
    // Iterate the SCOPE (young addrs), looking each up in the registry —
    // same rule as the counts loop. Iterating the full registry here was
    // the second quadratic remnant (O(heap) per gen-0 pass).
    let garbage_scope: Box<dyn Iterator<Item = usize>> = match scope.as_ref() {
        None => {
            Box::new(REGISTRY.with(|r| r.borrow().keys().copied().collect::<Vec<_>>().into_iter()))
        }
        Some(s) => Box::new(s.iter().copied()),
    };
    // Collect the scoped (addr, seq) pairs first, THEN clone the garbage
    // Values — one registry borrow, no Tracked clone.
    let mut garbage_addrs: Vec<(usize, u64)> = Vec::new();
    REGISTRY.with(|r| {
        let r = r.borrow();
        for addr in garbage_scope {
            if r.get(&addr).is_none() {
                continue;
            }
            if reachable.contains(&addr) || frozen.contains(&addr) {
                continue;
            }
            let seq = SEQ_OF.with(|s| s.borrow().get(&addr).copied().unwrap_or(0));
            garbage_addrs.push((addr, seq));
        }
    });
    let mut garbage: Vec<(usize, u64, Value)> = garbage_addrs
        .into_iter()
        .filter_map(|(addr, seq)| {
            REGISTRY.with(|r| r.borrow().get(&addr).map(|t| (addr, seq, t.value.clone())))
        })
        .collect();
    garbage.sort_by_key(|(_, seq, _)| *seq);
    let garbage: Vec<(usize, Value)> = garbage.into_iter().map(|(addr, _, v)| (addr, v)).collect();
    GARBAGE_NOW.with(|g| {
        let mut g = g.borrow_mut();
        g.clear();
        g.extend(garbage.iter().map(|(addr, _)| *addr));
    });
    if ap_obj::debug_flags::debug_flag(ap_obj::debug_flags::ApxFlag::GcDump) {
        REGISTRY.with(|r| {
            let r = r.borrow();
            eprintln!(
                "GC DUMP: registry={} reachable={} garbage={}",
                r.len(),
                reachable.len(),
                garbage.len()
            );
            for (addr, t) in r.iter().take(40) {
                eprintln!(
                    "  {addr:x} sc={} internal={} reachable={}",
                    strong_count_of(&t.value),
                    counts.get(addr).copied().unwrap_or(0),
                    reachable.contains(addr)
                );
            }
        });
    }

    // 3. Finalizers (__del__), creation order, exactly once (C-20/21).
    //    gc.callbacks see ("start", info) before, ("stop", info) after.
    //    Resurrection detection (C-21/gc008): an object is only kept
    //    tracked when its finalizer made it REACHABLE again (strong count
    //    rose during the call — someone stored it). Objects whose count is
    //    unchanged drop normally; the once-only guard lives in
    //    FINALIZED_ADDRS so a later sweep never re-fires them.
    run_callbacks_gen("start", garbage.len(), generation);
    let mut truly_resurrected: HashSet<usize> = HashSet::new();
    let already = FINALIZED_ADDRS.with(|r| r.borrow().clone());
    for (addr, v) in &garbage {
        if has_del(v) && !already.contains(addr) {
            let before = strong_count_of(v);
            weakbridge::call_del_finalizer(v);
            let after = strong_count_of(v);
            if after > before {
                truly_resurrected.insert(*addr);
            }
            FINALIZED_ADDRS.with(|r| r.borrow_mut().insert(*addr));
        }
    }
    run_callbacks_gen("stop", garbage.len(), generation);

    // 4. Weakrefs: flip dead targets, notify ap-run (callbacks fire there).
    REGISTRY.with(|r| sweep_weakrefs(&r.borrow(), &truly_resurrected));

    // 5. Stats.
    let reclaimed = garbage.len() as u64;
    let cycles = count_cycles(&garbage);
    STATS.with(|s| {
        let mut st = s.get();
        st.record(reclaimed, cycles);
        s.set(st);
    });

    // 6. Drop the registry clones: this releases acyclic garbage via the Rc
    //    cascade and breaks cycles (all members lose their registry handle
    //    in the same retain pass). Note: `garbage` still holds clones, so
    //    actual destruction lands when this function returns.
    //    Gen-0: objects OUTSIDE the young set were not scanned — always
    //    retain them (dropping them would free live old objects).
    //    CRITICAL for gen-0 complexity: retain() iterates the WHOLE map
    //    (O(heap) per pass — a quadratic remnant measured at 3M objects).
    //    A gen-0 pass can only remove the scoped garbage, so delete those
    //    addrs explicitly (O(garbage)); retain is reserved for full passes.
    let gen0 = generation == 0;
    if gen0 {
        REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            for (addr, _) in &garbage {
                r.remove(addr);
            }
        });
        SEQ_OF.with(|s| {
            let mut s = s.borrow_mut();
            for (addr, _) in &garbage {
                s.remove(addr);
            }
        });
        FINALIZED_ADDRS.with(|r| {
            let mut r = r.borrow_mut();
            for (addr, _) in &garbage {
                r.remove(addr);
            }
        });
    } else {
        REGISTRY.with(|r| {
            r.borrow_mut().retain(|addr, _| {
                reachable.contains(addr)
                    || frozen.contains(addr)
                    || truly_resurrected.contains(addr)
            });
        });
        SEQ_OF.with(|s| {
            s.borrow_mut()
                .retain(|a, _| reachable.contains(a) || truly_resurrected.contains(a))
        });
        // C-21 once-only is per-OBJECT, not per-address: swept addresses
        // return to the allocator and get reused by new objects (Rc allocs
        // recycle slots). Leaving a dead addr in FINALIZED_ADDRS poisons the
        // successor object — its __del__ would be skipped forever (observed:
        // with threshold 50, blocks of instances created after each
        // auto-collect never fired). Purge entries whose addr is no longer
        // tracked.
        FINALIZED_ADDRS.with(|r| {
            r.borrow_mut()
                .retain(|a| reachable.contains(a) || truly_resurrected.contains(a))
        });
    }

    // gc016: get_count()[0] reads 0 right after a collection.
    ALLOC_SINCE.with(|c| c.set(0));
    // Generation bookkeeping: gen-0 promotes its scanned set to MID (they
    // survive gen-0 now, die by gen-1 or full); gen-1 clears its scanned
    // MID entries (those are scanned-old until the next full pass); a full
    // pass re-seeds YOUNG/MID from the live registry so the NEXT gen-0 has
    // a conservative frontier (CPython moves survivors upward and full-
    // collects them eventually; the cascade above guarantees that).
    match generation {
        0 => {
            let scoped: Vec<usize> = scope
                .as_ref()
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            YOUNG.with(|y| {
                let mut y = y.borrow_mut();
                for addr in &scoped {
                    y.remove(addr);
                }
            });
            MID.with(|m| {
                let mut m = m.borrow_mut();
                m.extend(
                    scoped
                        .into_iter()
                        .filter(|a| REGISTRY.with(|r| r.borrow().contains_key(a))),
                );
            });
        }
        1 => {
            // Everything scanned this pass (scope = MID ∪ YOUNG) becomes
            // scanned-old; drop it from both frontier sets.
            let scoped: Vec<usize> = scope
                .as_ref()
                .map(|s| s.iter().copied().collect())
                .unwrap_or_default();
            YOUNG.with(|y| {
                let mut y = y.borrow_mut();
                for addr in &scoped {
                    y.remove(addr);
                }
            });
            MID.with(|m| {
                let mut m = m.borrow_mut();
                for addr in &scoped {
                    m.remove(addr);
                }
            });
        }
        _ => {
            YOUNG.with(|y| {
                let mut y = y.borrow_mut();
                y.clear();
                let keys: Vec<usize> = REGISTRY.with(|r| r.borrow().keys().copied().collect());
                y.extend(keys);
            });
            MID.with(|m| m.borrow_mut().clear());
        }
    }
    garbage.len()
}

fn has_del(v: &Value) -> bool {
    match v {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .type_ref
            .borrow()
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|t| t.lookup("__del__").is_some())
            .unwrap_or(false),
        _ => false,
    }
}

fn run_callbacks(phase: &str, collected: usize) {
    run_callbacks_gen(phase, collected, 2)
}

fn run_callbacks_gen(phase: &str, collected: usize, generation: usize) {
    let cbs = callbacks_snapshot();
    if cbs.is_empty() {
        return;
    }
    let info = PyDict::new();
    let _ = info.set(
        Value::Str(ap_obj::value::PyString::new("collected")),
        Value::Int(ap_obj::Int::from_i64(collected as i64)),
    );
    let _ = info.set(
        Value::Str(ap_obj::value::PyString::new("generation")),
        Value::Int(ap_obj::Int::from_i64(generation as i64)),
    );
    let _ = info.set(
        Value::Str(ap_obj::value::PyString::new("uncollectable")),
        Value::Int(ap_obj::Int::from_i64(0)),
    );
    let info = Value::Dict(info);
    let phase_v = Value::Str(ap_obj::value::PyString::new(phase));
    for cb in cbs {
        let _ = weakbridge::call_callback(&cb, &[phase_v.clone(), info.clone()]);
    }
}

fn sweep_weakrefs(registry: &HashMap<usize, Tracked>, truly_resurrected: &HashSet<usize>) {
    // The garbage list is authoritative for "died this cycle": sweep_weakrefs
    // runs BEFORE the registry retain (step 6), so membership alone can't see
    // this cycle's deaths — a ref dropped right before collect() would only
    // flip dead on the NEXT collect (observed: weakref callbacks landed one
    // collect late, gc004/gc005/gc008/gc015 red).
    let died_now: Vec<usize> = GARBAGE_NOW.with(|g| g.borrow().clone());
    WEAKREFS.with(|w| {
        let mut w = w.borrow_mut();
        for e in w.iter_mut() {
            if !e.dead
                && (died_now.contains(&e.target) || !registry.contains_key(&e.target))
                // C-22/gc008: a __del__ that resurrected its object put it
                // back in the reachable set — the weakref must NOT fire.
                && !truly_resurrected.contains(&e.target)
            {
                e.dead = true;
            }
        }
    });
    weakbridge::fire_dead_weakrefs();
}

/// Cycle census for GCSTATS: garbage members still holding strong counts > 1
/// at sweep time (they were kept alive by cycle peers, not plain refcounts).
fn count_cycles(garbage: &[(usize, Value)]) -> u64 {
    garbage
        .iter()
        .filter(|(_, v)| strong_count_of(v) > 1)
        .count() as u64
}

// ---------------------------------------------------------------------------
// gc module surface (state; ap-run builds the Python-visible module)
// ---------------------------------------------------------------------------

pub fn get_threshold() -> (usize, usize, usize) {
    THRESHOLDS.get()
}

pub fn set_threshold(t0: usize, t1: usize, t2: usize) {
    THRESHOLDS.set((t0.max(0), t1, t2));
}

pub fn get_count() -> (usize, usize, usize) {
    (ALLOC_SINCE.get(), 0, 0)
}

pub fn is_enabled() -> bool {
    ENABLED.get()
}

pub fn disable() {
    ENABLED.set(false);
}

pub fn enable() {
    ENABLED.set(true);
}

pub fn freeze() {
    let live = REGISTRY.with(|r| r.borrow().keys().copied().collect::<HashSet<usize>>());
    FROZEN.with(|f| *f.borrow_mut() = live);
}

pub fn unfreeze() {
    FROZEN.with(|f| f.borrow_mut().clear());
}

pub fn stats() -> GcStats {
    STATS.get()
}

/// gc.get_objects() — every tracked object (gc013 filters by type).
pub fn get_objects() -> Vec<Value> {
    REGISTRY.with(|r| r.borrow().values().map(|t| t.value.clone()).collect())
}

/// gc.is_tracked(v): registered containers True, scalars False. Tuples
/// containing only untracked leaves report False; a tuple holding a tracked
/// container is itself tracked (CPython pins tuple-list True, gc007).
pub fn is_tracked(v: &Value) -> bool {
    if let Value::Tuple(t) = v {
        // Tracked iff any element is tracked (CPython tracks tuples that
        // can participate in cycles — those holding containers).
        return t.0.iter().any(is_tracked);
    }
    let a = value_addr(v);
    a != 0 && REGISTRY.with(|r| r.borrow().contains_key(&a))
}

/// Weakref liveness for the ap-run weakref surface.
pub fn weakref_alive(target_addr: usize) -> bool {
    REGISTRY.with(|r| r.borrow().contains_key(&target_addr))
}

/// Number of weakref entries watching `target_addr` (dead ones included —
/// CPython's getweakrefcount counts the object's weakref list, which keeps
/// dead refs until their callbacks clear them).
pub fn weakref_count_for(target_addr: usize) -> usize {
    WEAKREFS.with(|w| {
        w.borrow()
            .iter()
            .filter(|e| e.target == target_addr)
            .count()
    })
}

/// The ref tokens watching `target_addr` (weakref.getweakrefs projection).
pub fn weakref_tokens_for(target_addr: usize) -> Vec<Value> {
    WEAKREFS.with(|w| {
        w.borrow()
            .iter()
            .filter(|e| e.target == target_addr)
            .map(|e| e.token.clone())
            .collect()
    })
}

/// Register a weakref entry (ap-run owns token/callback semantics).
pub fn weakref_add(e: WeakEntry) {
    WEAKREFS.with(|w| w.borrow_mut().push(e));
}

/// Current weakref entry count (ap-run indexes entries by creation order).
pub fn weakref_entry_count() -> usize {
    WEAKREFS.with(|w| w.borrow().len())
}

/// F2: the next weakref-registration seq (per-thread monotonic; ap-run's
/// WEAK_TOKENS store the same value so the pair can be pruned together).
pub fn weakref_next_seq() -> u64 {
    WEAK_SEQ.with(|n| {
        let id = n.get() + 1;
        n.set(id);
        id
    })
}

/// F2: prune (a) entries whose target is gone AND whose callback already
/// fired — the ref object keeps its dead state via weakref_is_dead reads on
/// WEAK_TOKENS side; once fired, CPython drops the entry, and (b) entries
/// whose token was explicitly dropped by ap-run (dead && fired && pruned).
/// Called from fire_dead_weakrefs after the drain. The linear pass is
/// bounded by the live-entry count, not cumulative registrations.
pub fn weakref_prune_fired(seq_list: &[u64]) {
    if seq_list.is_empty() {
        return;
    }
    use std::collections::HashSet;
    let gone: HashSet<u64> = seq_list.iter().copied().collect();
    WEAKREFS.with(|w| {
        w.borrow_mut().retain(|e| !gone.contains(&e.seq));
    });
}

/// Take (target_addr, callback, seq) pairs of dead-but-unfired entries;
/// marks them fired. ap-run invokes the callbacks (it owns call semantics)
/// and prunes BOTH sides of each drained pair by seq (weakref_prune_fired
/// + its own WEAK_TOKENS) — no-callback entries included: CPython drops a
/// dead ref immediately when no callback exists.
pub fn weakref_take_dead() -> Vec<(usize, Option<Value>)> {
    WEAKREFS.with(|w| {
        let mut w = w.borrow_mut();
        let mut out = Vec::new();
        for e in w.iter_mut() {
            if e.dead && !e.fired {
                e.fired = true;
                out.push((e.target, e.callback.clone()));
            }
        }
        out
    })
}

/// Is this weakref (by target address + registration order) dead?
pub fn weakref_is_dead(target_addr: usize, nth: usize) -> Option<bool> {
    WEAKREFS.with(|w| {
        let mut n = 0usize;
        for e in w.borrow().iter() {
            if e.target == target_addr {
                if n == nth {
                    return Some(e.dead);
                }
                n += 1;
            }
        }
        None
    })
}

/// Remove every dead weakref whose target address is gone (housekeeping).
pub fn weakref_prune() {
    WEAKREFS.with(|w| {
        w.borrow_mut().retain(|e| {
            if e.dead {
                // Keep fired entries that still have live tokens in Python;
                // ap-run drops tokens explicitly. Keep all: tokens are
                // user-visible objects. Only dedupe repeated dead entries.
                return true;
            }
            true
        });
    });
}

/// Append a gc.callbacks observer.
pub fn callback_add(v: Value) {
    CALLBACKS.with(|c| c.borrow_mut().push(v));
}

/// Snapshot of the callbacks list. When a Python-visible callbacks LIST is
/// registered (the `gc.callbacks` module attribute), it IS the source of
/// truth — appends/clears from user code are seen at collect time.
pub fn callbacks_snapshot() -> Vec<Value> {
    if let Some(list) = CALLBACKS_LIST.with(|c| c.borrow().clone()) {
        return list.items().to_vec();
    }
    CALLBACKS.with(|c| c.borrow().clone())
}

thread_local! {
    /// The Python-visible `gc.callbacks` list (registered by the module
    /// builder; appends/clears mutate it directly).
    static CALLBACKS_LIST: RefCell<Option<Gc<ap_obj::value::PyList>>> =
        const { RefCell::new(None) };
}

/// Register the Python-visible gc.callbacks list (module builder).
pub fn callbacks_set_list(l: &Gc<ap_obj::value::PyList>) {
    CALLBACKS_LIST.with(|c| *c.borrow_mut() = Some(l.clone()));
}

#[cfg(test)]
mod unit_tests {
    //! Leaf 01.03 (robustness-ci): in-crate unit tests of collector internals
    //! not reachable from the public surface. Each test runs on its own
    //! libtest thread, so the thread_local registry state is fresh.
    //!
    //! INVARIANT MAP (clause refs: docs/compat-contract.md C-20..C-23,
    //! docs/spec/06-observable-behavior.md S-06-17 #1; timing divergence
    //! D1 pinned only via EXPLICIT collect() per C-23's allowance):
    //! - subtract-refs seeding: the registry clone is exactly +1, so
    //!   `sc > internal + 1` <=> an external holder exists (C-22 "never a
    //!   false collection").
    //! - contention-abort: a scan that hits a contended borrow MUST abort
    //!   the whole cycle and return 0 objects (project rule: NEVER sweep a
    //!   partial view — historical incident: skipping contended objects
    //!   let a partial sweep kill live objects).
    //! - gc007 tuple rule, gc011 object-count return, gc016 post-collect
    //!   counter reset (corpus case ids in tests/corpus/gc).

    use super::*;
    use ap_obj::dict::PySet;
    use ap_obj::value::PyList;

    fn list_val() -> Value {
        Value::List(PyList::new())
    }

    #[test]
    fn value_addr_zero_for_scalars_nonzero_for_containers() {
        // INVARIANT: untracked scalars have no registry address (0 sentinel
        // keeps them out of the graph entirely — gc_gate doc). NOT spec-
        // pinned (internal identity detail; D1-sensitive id() values are
        // corpus-prohibited per S-06-17 #4, so nothing may observe these).
        assert_eq!(value_addr(&Value::None), 0);
        assert_eq!(value_addr(&Value::Bool(true)), 0);
        assert_eq!(value_addr(&Value::Int(ap_obj::Int::from_i64(42))), 0);
        assert_eq!(value_addr(&Value::Float(1.0)), 0);
        assert_eq!(value_addr(&Value::Ellipsis), 0);
        assert_ne!(value_addr(&list_val()), 0);
        assert_ne!(value_addr(&Value::Dict(PyDict::new())), 0);
        assert_ne!(value_addr(&Value::Set(PySet::new())), 0);
        // Distinct LIVE allocations get distinct addresses (registry keying).
        // Both values must stay alive while their addresses are compared —
        // freed Rc slots are recycled.
        let keep_a = list_val();
        let keep_b = list_val();
        assert_ne!(value_addr(&keep_a), value_addr(&keep_b));
    }

    #[test]
    fn strong_count_of_scalars_is_one() {
        // INVARIANT: scalars report 1 so subtract-refs arithmetic never
        // treats them as externally seeded (they are never in the
        // registry anyway; this pins the defensive arm).
        assert_eq!(strong_count_of(&Value::None), 1);
        assert_eq!(strong_count_of(&Value::Int(ap_obj::Int::from_i64(1))), 1);
    }

    fn local_registry(vals: &[Value]) -> HashMap<usize, Tracked> {
        let mut m = HashMap::new();
        for v in vals {
            m.insert(value_addr(v), Tracked { value: v.clone() });
        }
        m
    }

    #[test]
    fn edges_of_filters_untracked_children_and_covers_all_arms() {
        // INVARIANT: an edge exists only toward a REGISTRY member; scalars
        // and unregistered containers contribute no fan-in (CPython's
        // gc_visit only follows tracked children). gc007: tuple members
        // contribute edges. Not spec-pinned beyond the corpus slices.
        let tracked_child = list_val();
        let untracked_child = list_val(); // deliberately NOT in the map
        let registry = local_registry(&[tracked_child.clone()]);

        let mut out = Vec::new();
        let list = PyList::from_vec(vec![
            tracked_child.clone(),
            untracked_child.clone(),
            Value::Int(ap_obj::Int::from_i64(7)),
        ]);
        edges_of(&Value::List(list), &registry, &mut out);
        assert_eq!(out, vec![value_addr(&tracked_child)]);

        // Dict arm: tracked VALUES are edges (the untracked Str key is not).
        let d = PyDict::new();
        let val = list_val();
        let reg2 = local_registry(&[val.clone()]);
        d.set(Value::Str(ap_obj::PyString::new("k")), val.clone())
            .unwrap();
        let mut out2 = Vec::new();
        edges_of(&Value::Dict(d), &reg2, &mut out2);
        assert_eq!(out2, vec![value_addr(&val)]);

        // Tuple arm (gc007 fix 380f28e): members are edges.
        let kid = list_val();
        let reg3 = local_registry(&[kid.clone()]);
        let tup = Value::Tuple(Gc::new(ap_obj::Tuple(vec![
            Value::Int(ap_obj::Int::from_i64(1)),
            kid.clone(),
        ])));
        let mut out3 = Vec::new();
        edges_of(&tup, &reg3, &mut out3);
        assert_eq!(out3, vec![value_addr(&kid)]);
    }

    #[test]
    fn subtract_refs_seed_reachable_and_cycle_is_not() {
        // INVARIANT (core C-22 "never a false collection"): an entry with
        // an external holder (sc > internal + 1) seeds the reachable walk;
        // a pure cycle with only registry + intra-graph refs does NOT, so
        // it is collected. gc011-family behavior.
        //
        // Case A: registry clone + one external holder => sc = 2,
        // internal = 0 => seeded reachable (false-collection guard).
        let holder = list_val();
        let registry = local_registry(&[holder.clone()]);
        let mut counts = HashMap::new();
        count_internal_refs(&registry, &mut counts);
        let reach = mark_reachable(&registry, &counts);
        assert!(reach.contains(&value_addr(&holder)));

        // Case B: a true cycle, no external handle. Each list is held by
        // the registry clone + its cycle partner => sc = 2, internal = 1:
        // NOT seeded, and the walk reaches nothing. All test-thread handles
        // MUST be dropped first — any surviving clone is an "external
        // holder" by definition and would legitimately seed the walk.
        let a = PyList::new();
        let b = PyList::new();
        a.append(Value::List(b.clone()));
        b.append(Value::List(a.clone()));
        let va = Value::List(a.clone());
        let vb = Value::List(b.clone());
        let addr_a = value_addr(&va);
        let addr_b = value_addr(&vb);
        let reg_b = local_registry(&[va.clone(), vb.clone()]);
        drop((a, b, va, vb));
        let mut counts2 = HashMap::new();
        count_internal_refs(&reg_b, &mut counts2);
        assert_eq!(*counts2.get(&addr_a).unwrap(), 1);
        assert_eq!(*counts2.get(&addr_b).unwrap(), 1);
        let reach2 = mark_reachable(&reg_b, &counts2);
        assert!(reach2.is_empty(), "a pure cycle must be unreachable");

        // Cycle PEERS are counted by the census (count_cycles: garbage
        // entry with sc > 1): a cycle member is still held by its partner
        // at sweep time, while acyclic garbage dying by the plain Rc
        // cascade has ONLY its garbage-entry clone (sc == 1). Entries must
        // be MOVED into the garbage vec, not cloned — an extra test-side
        // clone inflates sc exactly the way a cycle partner does. gc011
        // pins collect() returning member OBJECTS; the census distinguishes
        // cycle-held from refcount-held garbage (GCSTATS instrumentation).
        let lone = PyList::new();
        let lone_v = Value::List(lone.clone());
        drop(lone); // lone_v is the ONLY clone
        assert_eq!(strong_count_of(&lone_v), 1);
        let plain = vec![(value_addr(&lone_v), lone_v)];
        assert_eq!(
            count_cycles(&plain),
            0,
            "acyclic garbage held only by its dying entry clone is sc==1: not a cycle peer"
        );
        let x = PyList::new();
        let y = PyList::new();
        x.append(Value::List(y.clone()));
        y.append(Value::List(x.clone()));
        let xv = Value::List(x.clone());
        let yv = Value::List(y.clone());
        drop(x);
        drop(y);
        let garbage = vec![(value_addr(&xv), xv), (value_addr(&yv), yv)];
        assert_eq!(
            count_cycles(&garbage),
            2,
            "both peers still held by a cycle partner (sc > 1) count as cycle members"
        );
    }

    #[test]
    fn contention_abort_never_sweeps_and_clears_flag() {
        // INVARIANT (PROJECT RULE, never weaken): a scan that hits a
        // borrowed-mid-mutation object sets SCAN_CONTENDED and collect()
        // ABORTS the entire cycle — returns 0, sweeps NOTHING, and clears
        // the flag so the next collection runs normally. A partial view
        // that skipped the contended object would let its (unseen) inbound
        // edges go uncounted and sweep live objects — the historical
        // incident this gate exists for. Not a spec clause (S-06-17 bans
        // GC timing from corpus); this is the internal correctness rule
        // behind D1's "never a false collection" (C-22).
        set_threshold(0, 10, 10);
        let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "contended");
        let ov = Value::Obj(obj.clone());
        gc_register(&ov);
        let victim = list_val();
        gc_register(&victim.clone()); // registry clone only... plus victim!
                                      // Keep ONLY the registry holding the victim so it would die if the
                                      // pass ran: move our handle into a temp that we drop below.
        let victim_addr = value_addr(&victim);
        drop(victim);

        // Mutate `obj` mid-scan: hold its attrs borrow across collect().
        let held = obj.attrs.borrow_mut();
        assert_eq!(collect(), 0, "contended scan must abort, not skip");
        assert!(
            REGISTRY.with(|r| r.borrow().contains_key(&victim_addr)),
            "abort must NOT sweep: the victim stays tracked (no partial-view kill)"
        );
        assert_eq!(
            SCAN_CONTENDED.with(|c| c.get()),
            false,
            "flag reset after abort"
        );
        drop(held);

        // Release the contention: the same pass now runs and sweeps the
        // victim (registry-only => sc 1, internal 0 => not seeded).
        let swept = collect();
        assert!(swept >= 1);
        assert!(!REGISTRY.with(|r| r.borrow().contains_key(&victim_addr)));
    }

    #[test]
    fn gc016_alloc_since_resets_after_collection() {
        // INVARIANT (corpus gc016 / C-23-safe): gc.get_count()[0] reads 0
        // right after a collection; registrations increment it monotonically
        // between collections.
        set_threshold(0, 10, 10);
        collect();
        assert_eq!(get_count().0, 0);
        gc_register(&list_val());
        gc_register(&list_val());
        assert_eq!(get_count().0, 2);
        collect();
        assert_eq!(get_count().0, 0);
    }

    #[test]
    fn auto_collect_fires_at_threshold_and_zero_disables() {
        // INVARIANT (gc.set_threshold semantics, CPython parity): t0 > 0
        // triggers a collection when ALLOC_SINCE reaches it; t0 == 0
        // disables auto-collection; gc.disable() gates it regardless.
        // Divergence-safe: only stats deltas observed, never timing (C-23).
        // NOTE: registration must go through gc_register EXPLICITLY — the
        // ap-obj allocation hook (gc_gate) is not installed in unit context.
        let base_collections = stats().collections;
        set_threshold(3, 10, 10);
        enable();
        // Hold refs so nothing dies; we observe the PASS happening, not
        // what it collects.
        let mut held = Vec::new();
        for _ in 0..3 {
            let v = list_val();
            held.push(v.clone());
            gc_register(&v);
        }
        assert_eq!(
            stats().collections,
            base_collections + 1,
            "third registration at t0=3 must auto-collect exactly once"
        );
        assert_eq!(get_count().0, 0);
        set_threshold(0, 10, 10);
        let d = list_val();
        gc_register(&d);
        assert_eq!(
            stats().collections,
            base_collections + 1,
            "t0=0 never auto-collects"
        );
        // gc.disable()/enable() gate the auto-collect even at t0 > 0.
        set_threshold(1, 10, 10);
        disable();
        assert!(!is_enabled());
        let e = list_val();
        gc_register(&e);
        assert_eq!(
            stats().collections,
            base_collections + 1,
            "disabled => no auto-collect"
        );
        enable();
        let f = list_val();
        gc_register(&f);
        assert_eq!(
            stats().collections,
            base_collections + 2,
            "re-enabled => auto-collect resumes"
        );
    }

    #[test]
    fn frozen_objects_survive_collection_until_unfreeze() {
        // INVARIANT (gc.freeze/unfreeze): frozen addresses are excluded from
        // sweeping even when unreachable; unfreeze restores collection.
        // Corpus: none (freeze is off the v1 diff surface); internal pin for
        // the MMTk swap (docs/gc-mmtk-swap.md keeps the FROZEN seam).
        set_threshold(0, 10, 10);
        unfreeze();
        let v = list_val();
        let addr = value_addr(&v);
        gc_register(&v.clone());
        drop(v);
        freeze();
        assert_eq!(collect(), 0, "everything live is frozen => nothing swept");
        assert!(REGISTRY.with(|r| r.borrow().contains_key(&addr)));
        unfreeze();
        assert!(collect() >= 1);
        assert!(!REGISTRY.with(|r| r.borrow().contains_key(&addr)));
    }

    #[test]
    fn registration_is_suppressed_while_collect_guard_is_up() {
        // INVARIANT (weakbridge doc): objects born inside a finalizer are
        // NOT registered mid-sweep ("would corrupt the pass"). The guard
        // flips suppress_registration; gc_register honors it.
        set_threshold(0, 10, 10);
        let v = list_val();
        let addr = value_addr(&v);
        {
            let _g = weakbridge::CollectGuard::new();
            assert!(weakbridge::suppress_registration());
            gc_register(&v.clone());
            assert!(
                !REGISTRY.with(|r| r.borrow().contains_key(&addr)),
                "mid-collect registration must be dropped"
            );
        }
        assert!(!weakbridge::suppress_registration());
        gc_register(&v.clone());
        assert!(REGISTRY.with(|r| r.borrow().contains_key(&addr)));
    }

    #[test]
    fn stats_census_counts_objects_and_cycle_peers() {
        // INVARIANT (gc011: collect returns OBJECTS; GCSTATS cycles counts
        // garbage peers still held by cycle partners at sweep time — a
        // census, not a cycle count; MMTk-swap-sensitive naming risk
        // flagged in PROGRESS gap list).
        set_threshold(0, 10, 10);
        let before = stats();
        let a = PyList::new();
        let b = PyList::new();
        a.append(Value::List(b.clone()));
        b.append(Value::List(a.clone()));
        gc_register(&Value::List(a.clone()));
        gc_register(&Value::List(b.clone()));
        drop(a);
        drop(b);
        let n = collect();
        assert_eq!(n, 2, "10-node-style cycle returns member OBJECTS (gc011)");
        let now = stats();
        assert_eq!(now.collections - before.collections, 1);
        assert_eq!(now.objects_reclaimed - before.objects_reclaimed, 2);
        assert_eq!(now.cycles_collected - before.cycles_collected, 2);
    }
}

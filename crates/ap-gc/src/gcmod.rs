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

    /// Instrumentation totals (GCSTATS env / stress report).
    static STATS: std::cell::Cell<GcStats> = const { std::cell::Cell::new(GcStats::EMPTY) };

    /// Registration sequence (creation order for finalizer ordering, gc002).
    static SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SEQ_OF: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());

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
        }
    });
    // Automatic collection at the generation-0 threshold (tunable via
    // gc.set_threshold / APX_GC_THRESHOLD env read by ap-run at entry).
    let n = ALLOC_SINCE.get() + 1;
    ALLOC_SINCE.set(n);
    let (t0, _, _) = THRESHOLDS.get();
    if ENABLED.get() && t0 > 0 && n >= t0 {
        ALLOC_SINCE.set(0);
        collect();
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
fn count_internal_refs(registry: &HashMap<usize, Tracked>, counts: &mut HashMap<usize, usize>) {
    counts.clear();
    for addr in registry.keys() {
        counts.insert(*addr, 0);
    }
    for t in registry.values() {
        let mut kids = Vec::new();
        edges_of(&t.value, registry, &mut kids);
        for k in kids {
            if let Some(c) = counts.get_mut(&k) {
                *c += 1;
            }
        }
    }
}

/// Reachable set: walk from the roots through registry edges. An entry with
/// `strong_count > internal_edges` is externally referenced — seed the walk
/// from those (CPython's move_unreachable frontier) AND from the explicit
/// root dicts/module namespaces (belt and braces: roots may hold objects
/// through non-registry channels like module_ns Rc clones).
fn mark_reachable(
    registry: &HashMap<usize, Tracked>,
    counts: &HashMap<usize, usize>,
) -> HashSet<usize> {
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: Vec<usize> = Vec::new();

    // Externally-referenced seeds (subtract-refs invariant). The registry
    // clone itself holds one strong count, so an entry is externally
    // referenced iff sc - 1 (registry) > internal (graph edges): something
    // outside the tracked graph (root dict, live frame, native stack
    // temporary) holds a reference.
    for (addr, t) in registry.iter() {
        let sc = strong_count_of(&t.value);
        let internal = counts.get(addr).copied().unwrap_or(0);
        if sc > internal + 1 {
            queue.push(*addr);
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
    if std::env::var("APX_GC_DUMP").is_ok() {
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
        if a != 0 && registry.contains_key(&a) {
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
                if !visited.contains(&k) {
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
}

/// One full collection. Returns the number of unreachable objects reclaimed
/// (CPython's gc.collect() return: OBJECTS, not cycles — gc011 pins
/// `found >= 10` on a 10-node cycle).
pub fn collect() -> usize {
    collect_gen(2)
}

/// collect(generation): the generation rides the callbacks info dict
/// (gc006: collect(0)/collect(1) report the REQUESTED generation).
pub fn collect_gen(generation: usize) -> usize {
    let _guard = weakbridge::CollectGuard::new();

    // 1. Internal-edge counts + reachable set. The registry borrow is held
    //    inside each step (no registration while the CollectGuard is up,
    //    so the snapshot is steady).
    let mut counts: HashMap<usize, usize> = HashMap::new();
    REGISTRY.with(|r| count_internal_refs(&r.borrow(), &mut counts));
    let reachable = REGISTRY.with(|r| mark_reachable(&r.borrow(), &counts));
    if SCAN_CONTENDED.with(|c| c.get()) {
        SCAN_CONTENDED.with(|c| c.set(false));
        return 0; // heap mid-mutation; retry at next threshold
    }

    // 2. Sweep list: drop registry handles for unreachable, unfrozen
    //    objects. Acyclic garbage dies by the Rc cascade when the LAST
    //    clone (the registry's) drops in the retain below; cycles break
    //    because all member clones drop together. We collect the garbage
    //    Values first so finalizers can still see their objects.
    let mut garbage: Vec<(usize, Value)> = Vec::new();
    let frozen = FROZEN.with(|f| f.borrow().clone());
    REGISTRY.with(|r| {
        let r = r.borrow();
        for (addr, t) in r.iter() {
            if reachable.contains(addr) || frozen.contains(addr) {
                continue;
            }
            garbage.push((*addr, t.value.clone()));
        }
    });
    if std::env::var("APX_GC_DUMP").is_ok() {
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
    REGISTRY.with(|r| sweep_weakrefs(&r.borrow()));

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
    REGISTRY.with(|r| {
        r.borrow_mut().retain(|addr, _| {
            reachable.contains(addr) || frozen.contains(addr) || truly_resurrected.contains(addr)
        });
    });
    SEQ_OF.with(|s| {
        s.borrow_mut()
            .retain(|a, _| reachable.contains(a) || truly_resurrected.contains(a))
    });

    // gc016: get_count()[0] reads 0 right after a collection.
    ALLOC_SINCE.with(|c| c.set(0));
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

fn sweep_weakrefs(registry: &HashMap<usize, Tracked>) {
    WEAKREFS.with(|w| {
        let mut w = w.borrow_mut();
        for e in w.iter_mut() {
            if !e.dead && !registry.contains_key(&e.target) {
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
/// report False (gc007 samples them only after a collect, which proves
/// them untracked; the pre-collect window is unpinned per the corpus note).
pub fn is_tracked(v: &Value) -> bool {
    if matches!(v, Value::Tuple(_)) {
        return false;
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

/// Take (target_addr, callback) pairs of dead-but-unfired entries; marks
/// them fired. ap-run invokes the callbacks (it owns call semantics).
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

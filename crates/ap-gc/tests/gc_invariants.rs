//! Leaf 01.03 (robustness-ci): in-crate integration tests for ap-gc —
//! allocation accounting, root keepalive, weakref-bridge ordering, and the
//! abort-on-contention rule, exercised through the crate's PUBLIC surface
//! with a test-only interpreter bridge (ap-run owns the real one).
//!
//! Clause map (docs/compat-contract.md C-20..C-23, docs/spec/
//! 06-observable-behavior.md S-06-17 #1): GC/weakref FINALIZER *timing* is
//! the standing divergence D1 and corpus-prohibited (S-06-17 #1); every
//! timing-sensitive assertion here is pinned to an EXPLICIT collect(), the
//! one anchor point C-20/C-23 declare deterministic. The never-false-
//! collection half of C-22 and the never-skip/abort-on-contention project
//! rule are pinned unconditionally.
//!
//! libtest runs each #[test] on a fresh thread and ap-gc's state is
//! thread_local — every test starts with an empty registry.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use ap_gc::{
    add_root_dict, add_root_module_ns, clear_roots, collect, disable, enable, gc_register,
    gc_tracked_len, get_count, get_objects, get_threshold, is_enabled, is_tracked, set_threshold,
    stats, weakref_add, weakref_alive, weakref_count_for, weakref_entry_count, weakref_is_dead,
    weakref_take_dead, WeakEntry,
};
use ap_obj::classes::{TypeFlags, TypeObject};
use ap_obj::dict::PyDict;
use ap_obj::value::{PyList, PyString, Value};
use ap_obj::{ObjKind, PyObject};

fn list_val() -> Value {
    Value::List(PyList::new())
}

fn addr_of(v: &Value) -> usize {
    match v {
        Value::List(l) => Arc::as_ptr(l) as *const u8 as usize,
        Value::Dict(d) => Arc::as_ptr(d) as *const u8 as usize,
        Value::Obj(o) => Arc::as_ptr(o) as *const u8 as usize,
        _ => panic!("not a tracked arm"),
    }
}

fn tracked_set_contains(addr: usize) -> bool {
    get_objects().iter().any(|v| addr_of(v) == addr)
}

// Test-side bridge state. The bridge fns are plain `fn` pointers, so they
// observe only thread_local state (the collector itself is single-threaded
// v0 — the same contract ap-run's gcsupport relies on).
thread_local! {
    static LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static DEL_COUNT: Cell<usize> = const { Cell::new(0) };
    /// When true, call_del clones the dying object back to life
    /// (resurrection, C-21/gc008 case).
    static RESURRECT_MODE: Cell<bool> = const { Cell::new(false) };
    static RESURRECT: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// The weakref target the ordering probe watches at fire-time.
    static PROBE_TARGET: Cell<usize> = const { Cell::new(0) };
    static DEAD_AT_FIRE: RefCell<Option<bool>> = const { RefCell::new(None) };
    static ALIVE_AT_FIRE: RefCell<Option<bool>> = const { RefCell::new(None) };
    /// Keeps TypeObject payloads alive so instance type_ref upgrades work.
    static STICKY_TYPES: RefCell<Vec<Arc<TypeObject>>> = const { RefCell::new(Vec::new()) };
}

fn sys_modules() -> Vec<Value> {
    Vec::new()
}
fn frame_values() -> Vec<Value> {
    Vec::new()
}
fn call_del(v: &Value) {
    DEL_COUNT.with(|c| c.set(c.get() + 1));
    LOG.with(|l| l.borrow_mut().push("del"));
    if RESURRECT_MODE.with(|m| m.get()) {
        RESURRECT.with(|r| r.borrow_mut().push(v.clone()));
    }
}
fn call_callback(_cb: &Value, _args: &[Value]) -> Result<Value, String> {
    Ok(Value::None)
}
fn fire_weakrefs() {
    LOG.with(|l| l.borrow_mut().push("fire"));
    // Ordering probe: capture what the weakref surface + registry look like
    // at the moment the collector notifies ap-run (step 4). Both reads are
    // re-entrant borrows — collect holds only a shared REGISTRY borrow here
    // (the deadlock this pins against is a retained-mut-borrow across the
    // bridge call).
    let target = PROBE_TARGET.with(|t| t.get());
    if target != 0 {
        let dead = weakref_is_dead(target, 0) == Some(true);
        let alive = weakref_alive(target);
        DEAD_AT_FIRE.with(|d| *d.borrow_mut() = Some(dead));
        ALIVE_AT_FIRE.with(|a| *a.borrow_mut() = Some(alive));
    }
}

/// Install the allocation hook + bridge (ap-run does this at entry via
/// gcsupport.rs:1505-1506). Threshold 0 => no auto-collect surprises.
///
/// 02.09b: the collector state is PROCESS-global now, so these tests (which
/// previously relied on thread_local registry isolation between libtest
/// threads) must run ONE AT A TIME against the shared registry — same
/// assertions, serialized execution (never weakening).
fn install_bridge() -> std::sync::MutexGuard<'static, ()> {
    static TEST_SER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = match TEST_SER.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    ap_obj::gc_gate::set_hook(ap_gc::gc_register);
    ap_gc::set_bridge(
        sys_modules,
        frame_values,
        call_del,
        call_callback,
        fire_weakrefs,
    );
    ap_gc::reset_for_test();
    set_threshold(0, 10, 10);
    clear_log();
    guard
}

fn clear_log() {
    LOG.with(|l| l.borrow_mut().clear());
    DEL_COUNT.with(|c| c.set(0));
    RESURRECT_MODE.with(|m| m.set(false));
    RESURRECT.with(|r| r.borrow_mut().clear());
    PROBE_TARGET.with(|t| t.set(0));
    DEAD_AT_FIRE.with(|d| *d.borrow_mut() = None);
    ALIVE_AT_FIRE.with(|a| *a.borrow_mut() = None);
}

/// A UserInstance whose class carries `__del__` (the has_del() predicate in
/// gcmod.rs: ObjKind::UserInstance + type_ref upgrade + MRO lookup).
fn make_del_instance(name: &str) -> Value {
    let cls_dict = PyDict::new();
    let del_fn = PyObject::new(ObjKind::BuiltinFn, "__del__");
    cls_dict
        .set(Value::Str(PyString::new("__del__")), Value::Obj(del_fn))
        .expect("gc_invariants.rs:128");
    let to = TypeObject::create(
        name.to_string(),
        name.to_string(),
        vec![],
        cls_dict,
        TypeFlags::default(),
        None,
    )
    .expect("gc_invariants.rs:131");
    STICKY_TYPES.with(|s| s.borrow_mut().push(to.clone()));
    let inst = PyObject::new(ObjKind::UserInstance, name);
    *inst.type_ref.write().unwrap_or_else(|p| p.into_inner()) = Some(Arc::downgrade(&to));
    Value::Obj(inst)
}

// ---------------------------------------------------------------------------
// 1. Allocation accounting
// ---------------------------------------------------------------------------

#[test]
fn allocation_accounting_counts_tracks_and_reclaims() {
    // INVARIANT (allocation accounting): every container registered through
    // the gc_gate hook lands in the registry exactly once (re-register is a
    // no-op); scalars are never tracked; a tuple of scalars is untracked
    // while a tuple holding a tracked container IS tracked (gc007);
    // gc.get_count()[0] counts registrations since the last collection and
    // reads 0 right after one (gc016); a sweep returns the exact count of
    // garbage OBJECTS (gc011 pins the unit) and shrinks the registry
    // identically. Timing-safe per C-23: only explicit-collect deltas
    // observed.
    let _bridge_ser = install_bridge();
    let base = gc_tracked_len();
    assert_eq!(base, 0, "fresh thread_local registry");
    assert_eq!(get_threshold().0, 0);

    let mut held = Vec::new();
    for _ in 0..3 {
        held.push(list_val()); // hook registers at the allocation site
    }
    assert_eq!(gc_tracked_len(), base + 3, "3 containers tracked");
    assert_eq!(get_count().0, 3, "alloc-since counts registrations");
    gc_register(&held[0]); // duplicate registration must not double-count
    assert_eq!(gc_tracked_len(), base + 3);
    assert_eq!(get_objects().len(), gc_tracked_len());
    for v in &held {
        assert!(is_tracked(v));
    }

    // Scalars never track (gc_gate doc header).
    assert!(!is_tracked(&Value::Str(PyString::new("scalar"))));
    assert!(!is_tracked(&Value::Int(ap_obj::Int::from_i64(5))));
    assert!(!is_tracked(&Value::None));
    // gc007: tuple of scalars untracked; tuple holding a tracked container
    // IS tracked.
    let tup_scalars = Value::Tuple(Arc::new(ap_obj::Tuple(vec![
        Value::Int(ap_obj::Int::from_i64(1)),
        Value::Int(ap_obj::Int::from_i64(2)),
    ])));
    assert!(!is_tracked(&tup_scalars), "gc007 tuple-scalar rule");
    let tup_with_list = Value::Tuple(Arc::new(ap_obj::Tuple(vec![held[0].clone()])));
    assert!(is_tracked(&tup_with_list), "gc007 tuple-list rule");
    drop(tup_with_list); // its held[0] clone is an EXTERNAL ref: drop it
                         // before the sweep or held[0] legitimately lives.

    let before_passes = stats().collections;
    drop(held);
    let reclaimed = collect();
    assert_eq!(
        reclaimed, 3,
        "dropped holders => exactly 3 objects reclaimed (gc011 units: OBJECTS)"
    );
    assert_eq!(gc_tracked_len(), base, "registry shrank by the sweep");
    assert_eq!(get_count().0, 0, "gc016: alloc-since resets after collect");
    assert_eq!(stats().collections, before_passes + 1, "exactly one pass");
}

#[test]
fn collection_cycle_frees_a_true_reference_cycle() {
    // INVARIANT (the reason this crate exists — Arc alone leaks cycles): a
    // cycle with no external holder is collected and ALL members die in
    // one retain pass; while any node is externally held the whole cycle
    // survives the pass (never a false collection, C-22). gc011: a 10-node
    // cycle reports >= 10 OBJECTS; here a 5-node cycle reports exactly 5
    // and the cycle census counts the same 5 peers.
    let _bridge_ser = install_bridge();
    let base = gc_tracked_len();

    let nodes: Vec<Arc<PyList>> = (0..5).map(|_| PyList::new()).collect();
    for i in 0..5 {
        nodes[i].append(Value::List(nodes[(i + 1) % 5].clone()));
    }
    assert_eq!(gc_tracked_len(), base + 5, "hook tracked every node");

    // Cycle + external anchor: must survive.
    let anchor = Value::List(nodes[0].clone());
    assert_eq!(
        collect(),
        0,
        "externally anchored cycle must survive (C-22)"
    );
    assert_eq!(gc_tracked_len(), base + 5);
    drop(anchor);

    drop(nodes); // only registry clones + intra-cycle slots remain
    let before = stats();
    let reclaimed = collect();
    assert_eq!(
        reclaimed, 5,
        "pure cycle: all 5 members are objects reclaimed (gc011)"
    );
    assert_eq!(gc_tracked_len(), base, "cycle broken in one retain pass");
    assert_eq!(
        stats().cycles_collected - before.cycles_collected,
        5,
        "census: each peer still held by a cycle partner at sweep time"
    );
}

// ---------------------------------------------------------------------------
// 2. Root keepalive
// ---------------------------------------------------------------------------

#[test]
fn root_dict_keepalive_survives_collect() {
    // INVARIANT (C-22 "never a false collection", root half): an object
    // held ONLY by a registered root dict MUST survive every collection
    // while the root holds it — even when the subtract-refs seed condition
    // does not cover it (here sc=2, internal=1: root-dict slot + registry
    // clone, so the ROOT_DICTS direct walk is the only thing saving it;
    // belt-and-braces per mark_reachable's doc). It becomes collectable
    // exactly when the root releases it (explicit collect() => deterministic
    // per C-23's allowance).
    let _bridge_ser = install_bridge();

    let root = PyDict::new();
    add_root_dict(&root);
    let v = list_val();
    let addr = addr_of(&v);
    root.set(Value::Str(PyString::new("x")), v.clone())
        .expect("gc_invariants.rs:269");
    drop(v);

    assert_eq!(
        collect(),
        0,
        "root-held object must not be collected (C-22 never-false)"
    );
    assert!(
        tracked_set_contains(addr),
        "root keepalive survived the pass"
    );
    assert!(weakref_alive(addr));

    // Release from the root: now ONLY the registry clone holds it.
    root.set(Value::Str(PyString::new("x")), Value::None)
        .expect("gc_invariants.rs:284");
    let reclaimed = collect();
    assert!(reclaimed >= 1);
    assert!(!tracked_set_contains(addr), "unrooted object collected");
    assert!(
        tracked_set_contains(addr_of(&Value::Dict(root.clone()))),
        "the root dict itself stays (externally held)"
    );
    clear_roots();
}

#[test]
fn module_ns_root_keepalive_survives_collect() {
    // INVARIANT: module-namespace roots (add_root_module_ns) keep objects
    // alive exactly like root dicts. Both root channels are wired by ap-run
    // at entry and the MMTk swap replaces them with MMTk root reporting
    // (docs/gc-mmtk-swap.md §1) — this pins today's stage-1 behavior.
    let _bridge_ser = install_bridge();
    let ns: Arc<PyDict> = PyDict::new();
    add_root_module_ns(&ns);
    let v = list_val();
    let addr = addr_of(&v);
    ns.set(Value::Str(PyString::new("f")), v.clone())
        .expect("gc_invariants.rs:307");
    drop(v);

    assert_eq!(collect(), 0, "module-ns-held object survives");
    assert!(tracked_set_contains(addr));

    clear_roots();
    ns.set(Value::Str(PyString::new("f")), Value::None)
        .expect("gc_invariants.rs:314");
    collect();
    assert!(
        !tracked_set_contains(addr),
        "released from the namespace => collected on the next pass"
    );
}

// ---------------------------------------------------------------------------
// 3. Weakref bridge ordering
// ---------------------------------------------------------------------------

#[test]
fn weakref_never_false_collects_and_dies_in_the_same_pass_it_fires() {
    // INVARIANT (C-22 verify-paragraph, verbatim): strong ref + callback,
    // 5 forced collects => callback NOT fired, ref alive; drop the strong
    // ref, collect => ref dead + callback offered EXACTLY ONCE. Also pins
    // the GARBAGE_NOW ordering fix: the dead flip and the ap-run fire
    // notification land in the SAME collect that killed the target, not
    // one pass late (gc004/gc005/gc015 were red before that fix).
    let _bridge_ser = install_bridge();
    let target = list_val();
    let addr = addr_of(&target);

    weakref_add(WeakEntry {
        target: addr,
        token: Value::Int(ap_obj::Int::from_i64(0)),
        callback: Some(Value::Str(PyString::new("cb"))),
        dead: false,
        fired: false,
        seq: 0,
    });
    assert_eq!(weakref_entry_count(), 1);
    assert_eq!(weakref_count_for(addr), 1);

    // 5 forced collects while strongly held: never a false collection.
    for _ in 0..5 {
        assert_eq!(collect(), 0, "live target must not be swept (C-22)");
        assert!(weakref_alive(addr));
        assert_eq!(weakref_is_dead(addr, 0), Some(false));
        assert!(
            weakref_take_dead().is_empty(),
            "callback must not fire while the target is reachable (C-22)"
        );
    }

    drop(target);
    let reclaimed = collect();
    assert!(reclaimed >= 1, "the pass that kills the target...");
    assert_eq!(
        weakref_is_dead(addr, 0),
        Some(true),
        "...flips it dead in the SAME pass (GARBAGE_NOW ordering)"
    );
    let dead = weakref_take_dead();
    assert_eq!(dead.len(), 1, "notification offered exactly once");
    assert_eq!(dead[0].0, addr);
    assert!(
        weakref_take_dead().is_empty(),
        "take-once semantics (fired flag)"
    );
    assert!(!weakref_alive(addr));
}

#[test]
fn weakref_bridge_orders_del_then_fire_before_registry_retain() {
    // INVARIANT (bridge ORDERING — the stated global-state risk of
    // weakbridge.rs): inside one collect() the collector must run __del__
    // finalizers (step 3) BEFORE notifying weakref deaths (step 4,
    // fire_dead_weakrefs), and notify BEFORE dropping registry clones
    // (step 6 retain). Observed at fire-time: entry.dead == true (step 4
    // ran) yet weakref_alive == true (retain hasn't). C-22 timing is
    // D1-divergent outside this explicit-collect anchor; ordering after
    // collect() is the one C-23-allowed form.
    let _bridge_ser = install_bridge();
    let target = make_del_instance("Noisy");
    let addr = addr_of(&target);
    weakref_add(WeakEntry {
        target: addr,
        token: Value::Int(ap_obj::Int::from_i64(1)),
        callback: Some(Value::Str(PyString::new("cb"))),
        dead: false,
        fired: false,
        seq: 0,
    });
    PROBE_TARGET.with(|t| t.set(addr));
    drop(target);

    let reclaimed = collect();
    assert!(reclaimed >= 1);

    let order = LOG.with(|l| l.borrow().clone());
    assert_eq!(
        order,
        vec!["del", "fire"],
        "bridge call ORDER: finalizer seam (step 3) then weakref-fire seam (step 4)"
    );
    assert_eq!(
        DEL_COUNT.with(|c| c.get()),
        1,
        "__del__ ran exactly once (C-21)"
    );
    assert_eq!(
        DEAD_AT_FIRE.with(|d| *d.borrow()),
        Some(true),
        "dead flag already flipped when ap-run is notified (same pass)"
    );
    assert_eq!(
        ALIVE_AT_FIRE.with(|a| *a.borrow()),
        Some(true),
        "registry retain NOT yet run at fire-time (step 4 < step 6)"
    );
    // After the pass: gone from the registry, dead, offered once.
    assert!(!weakref_alive(addr));
    assert_eq!(weakref_take_dead().len(), 1);
}

#[test]
fn resurrecting_del_keeps_weakref_alive_and_never_refires() {
    // INVARIANT (C-21 + gc008 resurrection clause): a __del__ that makes
    // its object reachable again keeps it tracked, its weakref MUST NOT
    // flip dead (C-22: "callbacks MUST NOT fire for objects that are still
    // reachable"), and the finalizer MUST NOT run again when the object
    // later dies — once-only is per-OBJECT over the process lifetime.
    let _bridge_ser = install_bridge();
    RESURRECT_MODE.with(|m| m.set(true));
    let target = make_del_instance("Zombie");
    let addr = addr_of(&target);
    weakref_add(WeakEntry {
        target: addr,
        token: Value::Int(ap_obj::Int::from_i64(2)),
        callback: Some(Value::Str(PyString::new("cb"))),
        dead: false,
        fired: false,
        seq: 0,
    });
    drop(target);

    collect();
    assert_eq!(DEL_COUNT.with(|c| c.get()), 1, "first finalizer ran");
    assert!(
        tracked_set_contains(addr),
        "resurrected object stays in the registry (C-21/gc008)"
    );
    assert_eq!(
        weakref_is_dead(addr, 0),
        Some(false),
        "weakref must NOT fire for a resurrected target (C-22)"
    );
    assert!(weakref_take_dead().is_empty());
    assert!(weakref_alive(addr));

    // Let it die for real: drop the resurrection holder, collect again.
    RESURRECT_MODE.with(|m| m.set(false));
    RESURRECT.with(|r| r.borrow_mut().clear());
    let reclaimed = collect();
    assert!(reclaimed >= 1, "second death is collected");
    assert_eq!(
        DEL_COUNT.with(|c| c.get()),
        1,
        "C-21 once-only across resurrection: finalizer did NOT re-fire"
    );
    assert!(!weakref_alive(addr));
    assert_eq!(weakref_is_dead(addr, 0), Some(true));
    assert_eq!(
        weakref_take_dead().len(),
        1,
        "callback offered at the real death"
    );
}

// ---------------------------------------------------------------------------
// 4. Abort-on-contention (PROJECT RULE: abort, never skip)
// ---------------------------------------------------------------------------

#[test]
fn collect_aborts_on_mid_mutation_contention_never_skips() {
    // INVARIANT (PROJECT RULE, never weaken): if the scan hits an object
    // whose interior is borrowed mid-mutation, the WHOLE collection ABORTS:
    // collect() returns 0, sweeps NOTHING, does not reset the allocation
    // counter (so the retry comes soon), and clears the contention flag.
    // SKIPPING the contended object instead would undercount its outbound
    // edges and let the partial view sweep objects kept live only through
    // it — the historical incident behind this gate (gcmod.rs SCAN_CONTENDED
    // + count_internal_refs docs; protects C-22's never-false-collection
    // half). Not corpus-pinned: S-06-17 #1 bans GC timing from the diff.
    let _bridge_ser = install_bridge();

    let obj = PyObject::new(ObjKind::Opaque, "mutating");
    let obj_v = Value::Obj(obj.clone());

    // Victim: reachable ONLY via an edge held inside `obj`'s attrs dict.
    // A scan that skipped the contended obj would not see that edge and
    // would sweep the victim out from under the mutator.
    let victim = list_val();
    let victim_addr = addr_of(&victim);
    obj.attrs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set(Value::Str(PyString::new("keep")), victim.clone())
        .expect("gc_invariants.rs:510");
    drop(victim);

    let alloc_before = get_count().0;

    // Mutate `obj` while a collection runs (the real-world shape: class-
    // body exec holding attrs.borrow_mut() across an allocation past t0).
    let held_borrow = obj.attrs.lock().unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        collect(),
        0,
        "contended scan must ABORT the cycle (return 0), never sweep"
    );
    assert!(
        tracked_set_contains(victim_addr),
        "ABORT-not-SKIP: the victim survives the contended pass untouched"
    );
    assert!(tracked_set_contains(addr_of(&obj_v)));
    assert_eq!(
        get_count().0,
        alloc_before,
        "abort must NOT reset the allocation counter (retry soon)"
    );
    assert_eq!(
        DEL_COUNT.with(|c| c.get()),
        0,
        "no finalizers ran on the abort"
    );
    drop(held_borrow);

    // Contention released + external handles dropped: the full pass now
    // re-scans and finds the chain dead — nothing outside the registry
    // holds obj, and the victim is reachable only through the swept obj,
    // so all of it goes in the same retain pass. This is correct: nothing
    // externally live dies once the mutator borrow is gone.
    clear_log();
    drop(obj_v);
    drop(obj);
    let swept = collect();
    assert!(
        swept >= 1,
        "clean pass sweeps the detached obj chain (no external holder)"
    );
    assert!(
        !tracked_set_contains(victim_addr),
        "victim went with its only holder (obj) once contention cleared"
    );
    assert_eq!(
        DEL_COUNT.with(|c| c.get()),
        0,
        "no __del__ seam calls: the swept chain has no finalizer (Opaque)"
    );

    // The external-handle shape (what the abort actually protects): a
    // live mutator ref through a root dict must keep the chain forever.
    // NOTE: PyObject::new() also allocates a transient tracking-eligible
    // dunders PyDict that nothing keeps alive except its registry clone —
    // that orphan dies on the first pass. The never-false assertion is
    // therefore about OUR objects staying tracked, not the pass count.
    let obj2 = PyObject::new(ObjKind::Opaque, "rooted");
    let obj2_v = Value::Obj(obj2.clone());
    let root2 = PyDict::new();
    add_root_dict(&root2);
    let victim2 = list_val();
    let victim2_addr = addr_of(&victim2);
    obj2.attrs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set(Value::Str(PyString::new("keep")), victim2.clone())
        .expect("gc_invariants.rs:578");
    root2
        .set(Value::Str(PyString::new("obj")), obj2_v.clone())
        .expect("gc_invariants.rs:582");
    drop(victim2);
    collect(); // absorb the dunders orphan (see NOTE)
    assert_eq!(
        collect(),
        0,
        "externally-rooted chain survives every pass (C-22 never-false)"
    );
    assert!(
        tracked_set_contains(victim2_addr),
        "edge through obj2 counted: no false collection"
    );
    // Drop the external handles: the whole chain dies together.
    root2
        .set(Value::Str(PyString::new("obj")), Value::None)
        .expect("gc_invariants.rs:597");
    clear_roots();
    drop(obj2_v);
    drop(obj2);
    let reclaimed = collect();
    assert!(reclaimed >= 1, "post-release collection proceeds normally");
    assert!(!tracked_set_contains(victim2_addr));
    drop(root2);
}

#[test]
fn disable_gates_automatic_collection_only() {
    // INVARIANT (gc.disable semantics, CPython parity): with collection
    // disabled the t0 threshold never auto-collects, but an explicit
    // collect() still runs — C-20's "finalization is forced by an explicit
    // gc.collect()" stays true while disabled (the corpus relies on it).
    let _bridge_ser = install_bridge();
    set_threshold(2, 10, 10);
    disable();
    assert!(!is_enabled());
    let base = stats().collections;
    let a = list_val(); // allocations bump ALLOC_SINCE to t0
    let b = list_val();
    assert_eq!(
        stats().collections,
        base,
        "disabled => threshold must not auto-collect"
    );

    // Explicit collect still works while disabled.
    drop(a);
    drop(b);
    let n = collect();
    assert!(n >= 1, "explicit collect runs even while disabled");
    assert_eq!(stats().collections, base + 1, "counted as a pass");

    enable();
    assert!(is_enabled());
    let c = list_val(); // re-armed: crossing t0 again auto-collects
    let d = list_val();
    assert_eq!(
        stats().collections,
        base + 2,
        "re-enabled => threshold auto-collect resumes"
    );
    drop(c);
    drop(d);
}

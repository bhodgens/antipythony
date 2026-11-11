//! Leaf 02.05a — the JIT entry hook + tier-dispatch seam.
//!
//! This module owns everything 02.05a adds to ap-run so that interp.rs
//! keeps its minimal single-dispatch-arm diff (hottest-file rule): tier
//! mode resolution (run_module init seam + APX_TIER env fallback), the
//! HostCallbacks implementation over the real interpreter surfaces, the
//! per-function JIT compile cache, and `jit_call_body` — the lowered fast
//! path `call_function` routes tier-eligible function bodies through.
//!
//! Tier-0-native discipline (ap-codegen-native/runtime.rs): everything
//! stays BOXED. The JIT executes the same cfg::Func → clif lowering the
//! codegen tests pin; host callbacks route every Python-visible operation
//! through the exact interpreter surfaces the tree-walker uses
//! (eval_binop/eval_compare/lookup_name/attr_get/call_value/…), so
//! observable bytes are identical by construction — the transparency
//! differential (tests/corpus/tier1-transparency/, C-31) verifies it per
//! probe in both modes.
//!
//! Fail-closed: a function whose body the cfg lowering rejects
//! (CodegenError::Unsupported) falls back to the tree-walker; the reason
//! is cached per function so the lowering attempt happens once. The
//! fallback table is reported under APX_TIER_DEBUG=1 and recorded in the
//! leaf report — fallback-by-table is the accepted v1 shape per
//! SPLIT-02.05.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use ap_codegen_native::cfg;
use ap_codegen_native::clif::{compile_tier1, JitFunc};
use ap_codegen_native::runtime::{ExecCtx, HostCallbacks};
use ap_codegen_native::tierup;
use ap_obj::exc::PyException;
use ap_obj::value::Value;

use crate::frame::{CellSlot, FnEntry, Frame, Interpreter, Loc};
use crate::interp::attr_get_public;

// ===========================================================================
// Tier mode (the seam state)
// ===========================================================================

/// Which execution tier function bodies route through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TierMode {
    /// Tree-walker only (the historical executor; `--tier 0`).
    Tree,
    /// Function bodies route through the JIT baseline (tier-0-native).
    Jit,
    /// Adaptive: == Jit while no tier-1 exists (C-31: adaptive must be
    /// invisible today; the seam exists so tier-1 hooks in later without
    /// touching this dispatch again).
    Adaptive,
}

impl TierMode {
    /// Whether function bodies should attempt the JIT route.
    pub fn jit_enabled(self) -> bool {
        matches!(self, TierMode::Jit | TierMode::Adaptive)
    }
}

thread_local! {
    /// The process tier mode, set once by run_module (or env fallback).
    static TIER_MODE: std::cell::Cell<TierMode> =
        const { std::cell::Cell::new(TierMode::Tree) };
    /// Per-function compiled cache: def-pointer → compiled JIT entry
    /// (None = fallback table — the lowering attempt happens once per
    /// function).
    static JIT_CACHE: RefCell<HashMap<usize, Option<Rc<JitEntry>>>> =
        RefCell::new(HashMap::new());
}

/// One cached JIT compilation for a function.
struct JitEntry {
    jit: JitFunc,
    /// Total slot count the compiled code indexes (cfg num_locals; temps
    /// alias locals 1:1 in the current lowering — see the leaf report).
    num_slots: u32,
    /// 02.05c: the lowered cfg's per-slot name table (local_names) — the
    /// slot bridge needs it to sync body locals (params only before).
    local_names: Vec<String>,
    /// 02.05e: the installed tier-1 fast-path entry (None until the
    /// background driver's compile completes and a call boundary installs
    /// it). The twin for the call is THIS entry's own `jit` baseline (the
    /// deterministic Risk-5 lowering makes the compiler's cfg clone slot-
    /// identical). Execution contract: tierup::call_fast(fast_ptr, &jit, ctx).
    tier1_fast: std::cell::Cell<*const u8>,
    /// 02.05c/02.05e failed-tier-up-disables: a tier-1 compile error (or a
    /// region-less body) marks DISABLED in the driver's state machine and
    /// the tier-up probe never fires again for the body (pon's
    /// failed-tier-up-disables discipline).
    tier1_failed: std::cell::Cell<bool>,
    /// The lowered cfg, retained for the driver's enqueue path (the
    /// compiler compiles from a clone — deterministic lowering, Risk-5).
    cfg: cfg::Func,
    /// 02.05d thrash backoff: the function's deopt state word (deopt.rs
    /// table cell), cached at tier-1 install so the per-call dispatch
    /// gate is ONE relaxed load. None until a tier-1 exists (a boxed-only
    /// body has no fast path to gate).
    deopt_word: std::cell::RefCell<Option<&'static std::sync::atomic::AtomicU64>>,
}

/// Resolve the tier mode from an explicit flag, falling back to APX_TIER,
/// then Tree. Called once from run_module (the init seam).
pub fn init_tier(explicit: Option<&str>) {
    let mode = match explicit {
        Some(m) => parse_tier(m).unwrap_or(TierMode::Tree),
        None => std::env::var("APX_TIER")
            .ok()
            .and_then(|m| parse_tier(&m))
            .unwrap_or(TierMode::Tree),
    };
    TIER_MODE.with(|t| t.set(mode));
    // 02.05g (gate-run finding): the baseline JIT's subscript read was a
    // fail-closed stub — any JIT-routed body reading info["..."] /
    // seq[i] SystemError'd. Route it through the exact tree-walker
    // surface (methods::subscript_get), flattened to the (class, msg)
    // string form the codegen seam reconstructs Python exceptions from.
    // Install once per process; the fn pointer is pure (no interpreter
    // state captured — subscript_get is a free function over Values).
    ap_codegen_native::runtime::set_host_subscript_get(|o, k| {
        match crate::methods::subscript_get(o, k) {
            Ok(v) => Ok(v),
            Err(crate::Flow::Raise(ev)) => {
                // Identical rendering to flow_to_py + str_args: rebuild the
                // ExceptionValue and reuse its py_str (byte-identical to
                // the tree-walker's exception text).
                let class = intern_class(&ev.class);
                let probe = crate::exc_hier::ExceptionValue::new(class, ev.args.clone());
                Err(format!(
                    "{}: {}",
                    ev.class,
                    probe.py_str().unwrap_or_default()
                ))
            }
            Err(_) => Err("SystemError: non-raise Flow in JIT subscript".to_string()),
        }
    });
    // 02.05c: the membership seam (`a in b`) — same pattern; the JIT'd
    // h_compare routes CMP_IN/CMP_NOTIN through interp::contains.
    ap_codegen_native::runtime::set_host_contains(|o, k| match crate::interp::contains(o, k) {
        Ok(has) => Ok(has),
        Err(crate::Flow::Raise(ev)) => {
            let class = intern_class(&ev.class);
            let probe = crate::exc_hier::ExceptionValue::new(class, ev.args.clone());
            Err(format!(
                "{}: {}",
                ev.class,
                probe.py_str().unwrap_or_default()
            ))
        }
        Err(_) => Err("SystemError: non-raise Flow in JIT contains".to_string()),
    });
    // 02.05f: the boxed OSR registry seam — the tier-0 back-edge poll
    // probes THIS function (installed once per process; the lookup is
    // pure over the registry + the JIT cache, no interpreter state).
    ap_codegen_native::runtime::set_host_osr_poll(osr_registry_lookup);
}

fn parse_tier(m: &str) -> Option<TierMode> {
    match m {
        "0" => Some(TierMode::Tree),
        "jit" | "adaptive" => Some(TierMode::Adaptive),
        // 02.06: the AoT binary's mode — the dispatch arm consults the
        // registered manifest FIRST (aot_runtime::entry_for), falling
        // back to this same adaptive/JIT route (the tree-walker twin is
        // the last resort). Gate parity: the tier flag must never let a
        // silent "no JIT" mode masquerade as AoT.
        "aot" => Some(TierMode::Adaptive),
        _ => None,
    }
}

pub fn tier_mode() -> TierMode {
    TIER_MODE.with(|t| t.get())
}

// ===========================================================================
// 02.05f — the boxed OSR registry + dispatch arm.
//
// The OSR registry maps (fn_key, header) → entry pointer. The back-edge
// poll (compiled into tier-0 by clif.rs's OsrPoll arm) probes it through
// runtime::set_host_osr_poll's inverted seam (installed once in
// init_tier); on a hit, tier-0 builds the OsrTransferBuffer in a stack
// slot, calls the entry (OSR-BY-CALL — osr.rs), and abandons its frame.
//
// Compile routing: OSR entries are compiled through the SAME background
// driver (tierup::request_osr / poll_osr), keyed (fn_key, header), with
// the disable marker permanently blacklisting the header (failed-tier-up-
// disables). The eligible-header set is fixed per function at cache-insert
// time (osr::loop_headers — while-shape polls only; each resolved header
// passes the OSR_MAX_LIVE eligibility gate here, reported under
// APX_TIER_DEBUG when refused).
// ===========================================================================

thread_local! {
    /// (fn_key, header) → OSR-entry pointer (null = blacklisted: an
    /// ineligible live set or a failed compile — never retried).
    static OSR_ENTRIES: RefCell<HashMap<(u64, u32), *const u8>> =
        RefCell::new(HashMap::new());
    /// OSR compile requests in flight (or answered) — the exactly-once
    /// guard for driver.request_osr. Mirrors the OSR_ENTRIES keyspace:
    /// an answered-but-not-installed key stays here until poll_osr hands
    /// over the verdict.
    static OSR_REQUESTED: RefCell<std::collections::HashSet<(u64, u32)>> =
        RefCell::new(std::collections::HashSet::new());
}

/// OSR registry probe (the h_osr_poll host seam). `fn_key`/`header` come
/// from the compiled poll immediates. Steps:
/// 1. installed entry → return it;
/// 2. blacklisted / already-requested → null (poll fast path);
/// 3. header eligible (OSR_MAX_LIVE gate) → enqueue an OSR compile
///    through the background driver, remember the key as pending → null.
///
/// Only the process-global driver runs here; when the driver was never
/// started (tier-0 runs, unit tests) the registry stays empty.
fn osr_registry_lookup(fn_key: u64, header: u32) -> *const u8 {
    let installed = OSR_ENTRIES.with(|m| {
        *m.borrow()
            .get(&(fn_key, header))
            .unwrap_or(&std::ptr::null())
    });
    if !installed.is_null() {
        return installed;
    }
    let requested = OSR_REQUESTED.with(|s| s.borrow().contains(&(fn_key, header)));
    if requested {
        return std::ptr::null();
    }
    // Route the compile through the driver (process-global only). A
    // missing driver (tier-0 run / sync-mode unit context without a
    // started driver) means no OSR — the loop keeps running tier-0.
    let Some(driver) = tierup::try_global() else {
        return std::ptr::null();
    };
    // The JitEntry for this fn_key owns the lowered cfg (the cache is
    // never evicted, so the def-pointer key resolves for any function
    // that ever reached the JIT route — a miss here means the poll was
    // compiled under a key whose cache row vanished, impossible today).
    let Some(je) = JIT_CACHE.with(|c| {
        let cache = c.borrow();
        cache
            .iter()
            .find_map(|(k, v)| (*k as u64 == fn_key).then(|| v.clone()).flatten())
    }) else {
        return std::ptr::null();
    };
    // Eligibility gate (reported, never silent).
    let plan = ap_codegen_native::osr::osr_plan(&je.cfg, cfg::BlockId(header));
    let eligible = matches!(
        &plan,
        ap_codegen_native::osr::OsrEligibility::Eligible { .. }
    );
    if !eligible {
        if tier_debug() {
            if let ap_codegen_native::osr::OsrEligibility::Ineligible { reason, live_count } = &plan
            {
                eprintln!(
                    "[OSR] fn_key={fn_key} header={header} INELIGIBLE live_count={live_count}: {reason}"
                );
            }
        }
        // Mark blacklisted so the poll never re-checks.
        OSR_ENTRIES.with(|m| m.borrow_mut().insert((fn_key, header), std::ptr::null()));
        OSR_REQUESTED.with(|s| s.borrow_mut().insert((fn_key, header)));
        return std::ptr::null();
    }
    // The driver requires an Arc'd completion consumer; enqueue with a
    // fresh in-flight set seeded from our requested keys.
    let mut in_flight = OSR_REQUESTED.with(|s| s.borrow().clone());
    let sent = driver.request_osr(fn_key, &je.cfg.name, header, je.cfg.clone(), &mut in_flight);
    OSR_REQUESTED.with(|s| *s.borrow_mut() = in_flight);
    let _ = sent;
    std::ptr::null() // install happens at the next call boundary (below)
}

/// Drain finished OSR compiles for every requested-but-unanswered key
/// (called at each jit_call_body entry — the same poll discipline as the
/// tier-1 install). Installs handles / records blacklists.
fn osr_poll_installs() {
    let Some(driver) = tierup::try_global() else {
        return;
    };
    let mut pending: Vec<(u64, u32)> = OSR_REQUESTED.with(|s| {
        let set = s.borrow();
        OSR_ENTRIES.with(|m| {
            let installed = m.borrow();
            set.iter()
                .filter(|k| !installed.contains_key(k))
                .copied()
                .collect()
        })
    });
    pending.sort_unstable();
    for key in pending {
        if let Some(done) = driver.poll_osr(key.0, key.1) {
            match done {
                Ok(h) => {
                    OSR_ENTRIES.with(|m| m.borrow_mut().insert(key, h.ptr));
                }
                Err(()) => {
                    // Failed-tier-up-disables: blacklist the header.
                    OSR_ENTRIES.with(|m| m.borrow_mut().insert(key, std::ptr::null()));
                    if tier_debug() {
                        eprintln!(
                            "[OSR] fn_key={} header={} compile FAILED (blacklisted)",
                            key.0, key.1
                        );
                    }
                }
            }
        }
    }
}

/// 02.05e: join the background tier-up compiler (process exit seam).
/// Idempotent; a no-op when the driver never started (tier-0 runs).
pub fn shutdown_tierup_driver() {
    tierup::shutdown_global();
}

fn tier_debug() -> bool {
    std::env::var("APX_TIER_DEBUG")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// 02.05a-residual: JIT-exercise counters. The t08 byte-identity suite proves
// TRANSPARENCY, not EXERCISE — the dispatch arm could silently fall back to
// the tree-walker everywhere and still pass. These process-global counters
// make the routing observable: compiles (lowering+baseline succeeded),
// fallbacks (lowering rejected — the fallback table), execs (jit_call_body
// actually ran compiled code). Printed once per process as a single
// machine-parseable stderr line under APX_TIER_STATS=1 (report_line), so the
// mode-pair differential runner can build the per-probe exercise table
// without touching the conformance output channels.
// ---------------------------------------------------------------------------

/// Bodies whose lowering + baseline compile SUCCEEDED (cache-insert path).
static JIT_COMPILES: AtomicU64 = AtomicU64::new(0);
/// Bodies whose lowering was REJECTED (fallback-table insert).
static JIT_FALLBACKS: AtomicU64 = AtomicU64::new(0);
/// jit_call_body invocations that RAN compiled code (cache hits + first
/// calls on a fresh compile; the actual exercise count).
static JIT_EXECS: AtomicU64 = AtomicU64::new(0);

/// 02.05d: read the process-wide exec counter as the deopt backoff's
/// progress signal. Monotonic and cheap (one relaxed load per tier-up
/// probe); per-function fairness comes from the per-key epoch word —
/// this is only the "has enough work happened since" term of
/// `tier_up_allowed`.
fn jit_execs_now() -> u64 {
    JIT_EXECS.load(Ordering::Relaxed)
}

/// Read the stats knobs once per call site (cheap env lookups; the stats
/// path is diagnostic-only).
fn tier_stats() -> bool {
    std::env::var("APX_TIER_STATS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Emit the one-line exercise report when APX_TIER_STATS=1. Called from
/// run_module's exit seam so the counts cover the whole module run.
pub fn report_line() {
    if !tier_stats() {
        return;
    }
    eprintln!(
        "[TIER-STATS] compiles={} fallbacks={} execs={}",
        JIT_COMPILES.load(Ordering::Relaxed),
        JIT_FALLBACKS.load(Ordering::Relaxed),
        JIT_EXECS.load(Ordering::Relaxed),
    );
    // 02.05e: the background driver's tier-state histogram (only when the
    // driver was actually started this run).
    if let Some(line) = tierup::stats_line() {
        eprintln!("{line}");
    }
    // 02.05f: the OSR transfer counter (own APX_OSR_STATS knob — the
    // conformance channels stay untouched without it).
    if let Some(line) = ap_codegen_native::runtime::osr_stats_line() {
        eprintln!("{line}");
    }
}

// ===========================================================================
// Host callbacks — every Python-visible op through the tree-walker's own
// surfaces (byte-identity by construction).
// ===========================================================================

/// Host state one JIT frame runs against (ptr-shared via
/// ExecCtx::host_data — the runtime ABI carries no Rust generics).
/// 02.06: `pub` — the AOT dispatch arm (aot_runtime::call_body) runs the
/// SAME host surfaces; a separate private clone would be a byte-identity
/// drift surface, not a seam.
pub struct HostState {
    pub(crate) interp: Rc<Interpreter>,
    pub(crate) frame: Rc<Frame>,
    /// Interned name table in compile-assigned id order.
    pub(crate) names: Vec<String>,
}

impl HostState {
    /// 02.06: the AOT arm's constructor (same shape as jit_call_body's).
    pub fn new(interp: Rc<Interpreter>, frame: Rc<Frame>, names: Vec<String>) -> Self {
        HostState { interp, frame, names }
    }
}

/// 02.06: the host callbacks the AOT dispatch arm runs its bodies with —
/// the SAME function pointers `host_callbacks()` hands the JIT route
/// (byte-identity by construction; a fork here would be a transparency
/// divergence between `antipythony run` and the built binary).
pub fn aot_host_callbacks() -> HostCallbacks {
    host_callbacks()
}

/// 02.06: frame → slot sync for the AOT arm (load_slots_into_ctx).
pub fn aot_load_slots(ctx: &mut ExecCtx, frame: &Frame, names: &[String]) {
    load_slots_into_ctx(ctx, frame, names);
}

/// 02.06: slot → frame sync for the AOT arm (store_slots_into_frame).
pub fn aot_store_slots(
    frame: &Frame,
    names: &[String],
    ctx: &ExecCtx,
    frame_rc: &Rc<Frame>,
) {
    store_slots_into_frame(frame, names, ctx, frame_rc);
}

/// 02.06: PyException → ExceptionValue rebuild for the AOT arm (rebuild).
pub fn aot_rebuild(ev: PyException) -> crate::exc_hier::ExceptionValue {
    rebuild(ev)
}

/// Read the HostState out of an ExecCtx (raw-pointer seam).
fn host(ctx: &mut ExecCtx) -> &mut HostState {
    unsafe {
        let p = ctx.host_data as *mut HostState;
        &mut *p
    }
}

/// Leak-once intern table for exception CLASS names: PyException's class
/// field is `&'static str`, and distinct class names per run are a small
/// bounded set (builtins + user exception classes) — unlike messages,
/// which stay owned Strings, so hot raise loops (t1_014: 2k raises) do
/// not grow memory per iteration.
fn intern_class(class: &str) -> &'static str {
    thread_local! {
        static INTERNED: RefCell<std::collections::HashMap<String, &'static str>> =
            RefCell::new(std::collections::HashMap::new());
    }
    INTERNED.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(s) = m.get(class) {
            return *s;
        }
        let leaked: &'static str = Box::leak(class.to_string().into_boxed_str());
        m.insert(class.to_string(), leaked);
        leaked
    })
}

fn flow_to_py(f: crate::Flow) -> PyException {
    match f {
        crate::Flow::Raise(ev) => PyException::new(intern_class(&ev.class), str_args(&ev)),
        _ => PyException::new("SystemError", "non-raise Flow in JIT helper"),
    }
}

/// `str(exc)` payload text (args joined; the from_py reconstruction makes
/// this byte-identical to the tree-walker's exception rendering).
fn str_args(ev: &crate::exc_hier::ExceptionValue) -> String {
    let class: &'static str = intern_class(&ev.class);
    let probe = crate::exc_hier::ExceptionValue::new(class, ev.args.clone());
    probe.py_str().unwrap_or_default()
}

/// Build the reconstructed ExceptionValue for a caught JIT exception.
/// from_py packs the message as args[0] (str), mirroring Flow::from_py —
/// the same adaptation the builtin surfaces already use.
fn rebuild(ev: PyException) -> crate::exc_hier::ExceptionValue {
    crate::exc_hier::ExceptionValue::from_py(ev)
}

fn jit_lookup_global(
    ctx: &mut ExecCtx,
    slot: u32,
    name_idx: u32,
    dst_slot: u32,
) -> Result<(), PyException> {
    let name = host(ctx).names[name_idx as usize].clone();
    let h = host(ctx);
    // NOTE: the JIT route is only dispatched for bodies whose lowering
    // succeeded, and the cfg stage routes ANY name not in its local set
    // through LoadGlobal — module-level names included. Locals were already
    // synced into ctx.slots at entry, and temps live ABOVE the local slots,
    // so reading frame.locals here would alias the same storage the JIT is
    // writing through its own slots; skip it and resolve through the
    // tree-walker's full chain.
    let res = crate::interp::lookup_name_pub(&h.interp, &h.frame, &name);
    match res {
        Ok(v) => {
            ctx.slots[dst_slot as usize] = v;
            // 02.05b GlobalIC: record the loaded tag at this site.
            let v = ctx.slots[dst_slot as usize].clone();
            if let Some(cell) = ctx.ic_cell(slot) {
                cell.record(ap_codegen_native::feedback::TypeTag::of(&v), None);
            }
            Ok(())
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_call(
    ctx: &mut ExecCtx,
    slot: u32,
    callee_slot: u32,
    arg_start: u32,
    argc: u32,
    dst_slot: u32,
) -> Result<(), PyException> {
    let callee = ctx.slots[callee_slot as usize].clone();
    // 02.05b CallIC: record the callee tag (callability feedback).
    if let Some(cell) = ctx.ic_cell(slot) {
        cell.record(ap_codegen_native::feedback::TypeTag::of(&callee), None);
    }
    let args: Vec<Value> = ctx.args[arg_start as usize..(arg_start + argc) as usize].to_vec();
    let res = {
        let h = host(ctx);
        // NOTE: the caller frame passed here is the EXECUTING function's
        // own frame; nested call frames chain through call_value's own
        // machinery, and recursion re-enters the SAME JIT dispatch arm
        // (call_function), not this helper, for callee bodies.
        crate::interp::call_value_frame_pub(&h.interp, &h.frame, &callee, &args, &[])
    };
    match res {
        Ok(v) => {
            ctx.slots[dst_slot as usize] = v;
            Ok(())
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_load_attr(
    ctx: &mut ExecCtx,
    slot: u32,
    obj_slot: u32,
    name_idx: u32,
    dst_slot: u32,
) -> Result<(), PyException> {
    let obj = ctx.slots[obj_slot as usize].clone();
    let name = host(ctx).names[name_idx as usize].clone();
    let res = attr_get_public(&host(ctx).interp, &obj, &name);
    match res {
        Ok(Some(v)) => {
            ctx.slots[dst_slot as usize] = v;
            // 02.05b AttrIC: record (receiver tag) at this site.
            if let Some(cell) = ctx.ic_cell(slot) {
                cell.record(ap_codegen_native::feedback::TypeTag::of(&obj), None);
            }
            Ok(())
        }
        Ok(None) => {
            // The tree-walker's AttributeError text shape (interp.rs
            // Expr::Attribute arm's ok_or_else).
            let msg = format!(
                "'{}' object has no attribute '{}'{}",
                crate::classes::display_type_name(&obj),
                name,
                ""
            );
            Err(PyException::new("AttributeError", msg))
        }
        Err(f) => Err(flow_to_py(f)),
    }
}

fn jit_store_attr(
    ctx: &mut ExecCtx,
    obj_slot: u32,
    name_idx: u32,
    v_slot: u32,
) -> Result<(), PyException> {
    let obj = ctx.slots[obj_slot as usize].clone();
    let v = ctx.slots[v_slot as usize].clone();
    let name = host(ctx).names[name_idx as usize].clone();
    let res = crate::classes::attr_set_protocol(&host(ctx).interp, &obj, &name, v);
    res.map_err(flow_to_py)
}

fn host_callbacks() -> HostCallbacks {
    HostCallbacks {
        load_global: jit_lookup_global,
        call: jit_call,
        load_attr: jit_load_attr,
        store_attr: jit_store_attr,
    }
}

// ===========================================================================
// Slot bridging — JIT flat slots ↔ frame locals/cells
// ===========================================================================

/// The function's bridged slot names in compile-assigned order.
///
/// OFF-BY-ONE BRIDGE (02.05a): Func::new pre-bumps num_locals to
/// num_params WITHOUT registering the param names, so the lowerer's
/// self.local("p") assigns the FIRST param name to LocalId(num_params) —
/// slots [0..num_params) are dead and params live at
/// [num_params .. num_params + n_params) in source order. The bridge
/// mirrors exactly that.
///
/// 02.05c: body locals (slots beyond the params) now carry REAL names
/// (cfg::Func::local_names) — the bridge must include them or
/// store_slots_into_frame drops every non-param binding (the `global X;
/// X = v` corpus divergences). `func` is the lowered cfg (available at
/// the call site from the JitEntry's compile); None when the caller
/// lacks it (never, post-02.05c — kept Option for warm ordering).
fn slot_names_for(entry: &FnEntry, body_locals: &[String]) -> Vec<String> {
    let num_params = entry.def.params.len();
    let mut names: Vec<String> = vec![String::new(); num_params];
    names.extend(entry.def.params.iter().map(|p| p.name.clone()));
    // Body locals: the cfg's local_names beyond the (dead + params)
    // prefix — the authoritative name-per-slot record for slots the body
    // assigns (02.05c; without these, store_slots_into_frame dropped
    // every non-param binding — the `global X; X = v` divergences).
    for name in body_locals.iter().skip(names.len()) {
        names.push(name.clone());
    }
    names
}

/// Sync frame locals/cells INTO the JIT slot array (call entry). Unbound
/// slots read as None (the cfg ABI's NULL-if-unbound convention).
fn load_slots_into_ctx(ctx: &mut ExecCtx, frame: &Frame, names: &[String]) {
    for (i, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue; // dead pre-bumped slot (Func::new off-by-one, 02.05a)
        }
        let v = match frame.locals.borrow().get(name) {
            Some(Loc::Slot(s)) => s.borrow().clone(),
            Some(Loc::CellSlot(cell)) => cell.borrow().clone(),
            None => None,
        };
        ctx.slots[i] = v.unwrap_or(Value::None);
    }
}

/// Sync the JIT slot array back INTO the frame (call exit — success AND
/// exception unwind; except blocks observe partially-updated locals).
///
/// 02.05a-residual (this leaf): the sync is REAL now, routed through the
/// tree-walker's exact `store_name` semantics per slot name. The 02.05a
/// revision shipped this as a disabled no-op (`let _ = (frame, names,
/// ctx)` — timeboxed), which made any tier-routed body that binds a
/// non-param local diverge from the tree-walker the moment a caller (or a
/// nested def) observed the frame. The bridge writes each named slot back
/// through `store_name`'s own chain — declared-global → frame global,
/// declared-nonlocal / my-cell → the shared cell, everything else → the
/// frame's local map — so the observable post-call frame state matches the
/// tree-walker's by construction. Unbound JIT slots (dead pre-bumped
/// params, never-assigned temps) are skipped: the cfg ABI models
/// unbound-as-None and store_name(None) would fabricate a binding the
/// tree-walker never makes.
fn store_slots_into_frame(frame: &Frame, names: &[String], ctx: &ExecCtx, frame_rc: &Rc<Frame>) {
    let interp = crate::interp::current_interp_for_bindings_pub();
    let reg = crate::interp::current_registry_pub();
    for (i, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue; // dead pre-bumped slot (Func::new off-by-one, 02.05a)
        }
        // Skip NEVER-ASSIGNED slots: the JIT models unbound as None, but
        // the tree-walker leaves the name out of the frame entirely (a
        // later read must stay NameError/UnboundLocalError, not None).
        // Distinguishing "assigned None" from "never assigned" needs a
        // write marker; the cheap conservative rule is: only propagate a
        // None when the frame ALREADY has a binding for the name (an
        // overwrite of an existing param/local), and skip None entirely
        // when it doesn't.
        let v = ctx.slots[i].clone();
        if matches!(v, Value::None) && !frame.has_binding(name) {
            continue;
        }
        // The tree-walker's exact store chain (declared global → globals;
        // nonlocal/cell → the cell; else the local map). Errors are
        // impossible on these shapes (no class-ns frame ever reaches the
        // JIT hook), so a failure is a bridge bug: surface loudly.
        if let Err(f) = crate::interp::store_name(&interp, frame_rc, name, v, &reg) {
            let why = match f {
                crate::Flow::Raise(ev) => format!("raise {}: {:?}", ev.class, ev.args),
                crate::Flow::Return(_) => "Return flow".to_string(),
                crate::Flow::Break => "Break flow".to_string(),
                crate::Flow::Continue => "Continue flow".to_string(),
                crate::Flow::Next => "Next flow".to_string(),
                crate::Flow::YieldBack(_) => "YieldBack flow".to_string(),
            };
            eprintln!(
                "[TIER] slot sync-back store_name({name}) failed: {why} — \
                 jit_hook bridge bug, refusing to swallow"
            );
        }
    }
}

// ===========================================================================
// Compile + execute (the dispatch arm's callee)
// ===========================================================================

/// Warm-pass compile: populate the compile cache / fallback table WITHOUT
/// executing anything.
///
/// 02.05c gate fix (latent 02.05a bug exposed when the h_call arity fix
/// first let bodies with calls actually compile): the warm pass used to
/// call `jit_call_body`, which RUNS the compiled body — its "NO execution
/// happens here" comment was only true while every call-containing body
/// failed baseline compile and the warm pass was a no-op. A module whose
/// top-level code short-circuits a call (`0 and side()`) printed side
/// effects during warm before main even ran. Warm now goes through the
/// same cache slot call_function will hit, but via this execute-free path.
fn warm_compile_only(entry: &FnEntry) {
    let fn_id = Rc::as_ptr(&entry.def) as usize;
    JIT_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        let _ = cache.entry(fn_id).or_insert_with(|| {
            let r = compile_for(entry).map(Rc::new);
            if r.is_some() {
                JIT_COMPILES.fetch_add(1, Ordering::Relaxed);
            } else {
                JIT_FALLBACKS.fetch_add(1, Ordering::Relaxed);
            }
            r
        });
    });
}

/// Route one function body through the JIT baseline.
///
/// Ok(None) = fallback-table hit (cfg lowering unsupported for this
/// body); caller uses the tree-walker. Some(v) = the JIT'd body's return
/// value. Err(Flow::Raise(..)) = the JIT'd body raised.
pub fn jit_call_body(
    interp: &Rc<Interpreter>,
    entry: &FnEntry,
    frame: &Rc<Frame>,
) -> Result<Option<Value>, crate::Flow> {
    // Cache identity: the FnEntry's def Rc pointer (stable per registered
    // function; G4 dedupe guarantees one entry per code object).
    let fn_id = Rc::as_ptr(&entry.def) as usize;

    let cached = JIT_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache
            .entry(fn_id)
            .or_insert_with(|| {
                // First (and only) lowering attempt for this body: count the
                // outcome so the exercise report distinguishes a real JIT
                // route from a silent all-fallback run.
                let r = compile_for(entry).map(Rc::new);
                if r.is_some() {
                    JIT_COMPILES.fetch_add(1, Ordering::Relaxed);
                } else {
                    JIT_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                }
                r
            })
            .clone()
    });

    if tier_debug() {
        let tag = if cached.is_some() {
            "compiled"
        } else {
            "FALLBACK"
        };
        eprintln!("[TIER] fn {} {tag}", entry.def.name);
    }

    let Some(je) = cached else {
        return Ok(None); // fallback table
    };
    JIT_EXECS.fetch_add(1, Ordering::Relaxed);

    // 02.05e tier-up probe (background driver; replaces 02.05c's
    // synchronous compile-at-the-boundary): when no tier-1 fast path is
    // installed yet and the body is not permanently disabled, first POLL
    // for a finished background compile (install on hit), else — if the
    // driver's state says this body was never enqueued — snapshot the
    // feedback cells ON THIS THREAD (pon snapshot discipline) and enqueue.
    // The caller never blocks on a compile (except APX_SYNC_TIERUP=1
    // debug mode, which keeps the 02.05c inline shape for differential
    // debugging). Failure disables permanently (pon
    // failed-tier-up-disables, owned by the driver's state machine).
    //
    // 02.05d thrash backoff: the probe consults the per-function deopt
    // state FIRST — a Deferred function waits out `threshold << epoch`
    // calls (deopt::tier_up_allowed with this body's lifetime exec
    // counter as the progress signal) and a Disabled one never tiers up
    // again.
    let fn_key = fn_id as u64;
    if je.tier1_fast.get().is_null()
        && !je.tier1_failed.get()
        && ap_codegen_native::deopt::tier_up_allowed(fn_key, jit_execs_now())
    {
        tier_up_step(fn_id, entry, &je);
    }
    // 02.05f: drain finished OSR-entry compiles (the same poll-to-install
    // discipline the tier-1 path uses — the back-edge poll in tier-0 code
    // only consults OSR_ENTRIES, never the driver, so installs land here
    // at call boundaries; the header's own NEXT back-edge crossing
    // transfers).
    osr_poll_installs();
    let tier1_fast = je.tier1_fast.get();
    if std::env::var("APX_CALLDBG").is_ok() {
        let st = ap_codegen_native::deopt::deopt_stats(fn_key);
        eprintln!(
            "[DISPATCH-DBG] fast_null={} failed={} deopt={:?}",
            tier1_fast.is_null(),
            je.tier1_failed.get(),
            st
        );
    }

    let names = slot_names_for(entry, &je.local_names);
    let mut host_state = Box::new(HostState {
        interp: interp.clone(),
        frame: frame.clone(),
        names: je.jit.name_table.clone(),
    });
    let hptr: *mut HostState = &mut *host_state;

    let mut ctx = ExecCtx::new(je.num_slots as usize, host_callbacks());
    ctx.host_data = hptr as *mut u8;
    ctx.names = je.jit.name_table.clone();
    // 02.05b: park the compiled feedback cells (slot id == index) so the
    // recording helpers can reach them. The JitFunc owns the cells; the
    // ctx borrow cannot outlive jit_call_body.
    ctx.ic_base = je.jit.feedback_cells.as_ptr() as *mut _;
    ctx.ic_len = je.jit.feedback_cells.len() as u32;
    load_slots_into_ctx(&mut ctx, frame, &names);

    // 02.05g root-shim seam: capture this host frame as the fp-chain walk
    // start for the duration of the JIT call. The collector (ap-gc
    // rootscan feed) walks the native chain from HERE when a GC fires
    // inside the JIT'd body; the ExecCtx slot array (`ctx`) is a local of
    // THIS frame, so the walk must be able to name it. Cleared on every
    // exit path below — a stale capture would dangle.
    // SAFETY: `caller_frame_pointer()` reads this function's own fp; the
    // frame stays live across `je.jit.call` (all exits clear before
    // returning).
    unsafe { ap_gc::set_capture_point(ap_gc::caller_frame_pointer()) };
    // 02.05e: run the typed fast path when an install has landed; its
    // status-2 hand-off falls back to THIS entry's own baseline twin
    // (slot-identical by the deterministic lowering — see tierup.rs).
    // One entry point, byte-identical results either way.
    //
    // 02.05d thrash backoff dispatch gate: once the pin lands (state ==
    // Disabled after DEOPT_PIN_EPOCH deferred epochs), the fast path is
    // bypassed and the boxed twin runs directly — the pon entry-pointer
    // restore analogue. Semantically free either way (the twin IS the
    // same lowering tier-0 executes), so the gate is a pure de-cost.
    // The word pointer is cached at install time (see tier_up_step), so
    // the gate is ONE relaxed load; an unpinned body skips the map.
    let fast_for_call = match (
        (!tier1_fast.is_null()).then_some(tier1_fast),
        je.deopt_word.borrow().as_ref(),
    ) {
        (Some(_), Some(w))
            if ap_codegen_native::deopt::state_byte_of(w)
                == ap_codegen_native::deopt::TierState::Disabled =>
        {
            None // pinned: run the boxed baseline (== tier-0 code)
        }
        (other, _) => other,
    };
    let call_r = match fast_for_call {
        Some(fast_ptr) => unsafe { tierup::call_fast(fast_ptr, &je.jit, &mut ctx) },
        None => unsafe { je.jit.call(&mut ctx) },
    };
    // 02.05c twin-start hygiene: the tier-1 hand-off parks the twin start
    // block; both call paths consume it. Reset so an independent later
    // call never inherits a stale start block.
    ctx.twin_start = 0;
    ap_gc::clear_capture_point();

    // Sync locals back BEFORE mapping the result: an exception unwind
    // still leaves partially-updated locals observable to except blocks.
    store_slots_into_frame(frame, &names, &ctx, frame);

    match call_r {
        Ok(()) => {
            let r = std::mem::replace(&mut ctx.ret, Value::None);
            Ok(Some(r))
        }
        Err(py) => Err(crate::Flow::Raise(rebuild(py))),
    }
}

/// 02.05e tier-up step (the call-boundary probe; replaces 02.05c's
/// synchronous `try_tier_up`).
///
/// Three states of the world, one cheap probe per not-yet-tier-1 call:
///
/// 1. **Poll**: a background compile may have finished since the last
///    call — `driver.poll(fn_id)` installs it (Release store of the
///    fast-path entry into the JitEntry, QUEUED→TIER1 CAS inside the
///    driver) or consumes a disable marker (compile failed / no typed
///    region → `tier1_failed`, never retried).
/// 2. **Enqueue**: the body is still Tier0 in the driver's machine and a
///    binop feedback cell reached MONO(IntI64, IntI64) — snapshot the
///    cells ON THIS THREAD (pon snapshot-at-enqueue discipline: the
///    compiler never reads live cells) and hand the driver a clone of the
///    lowered cfg (deterministic lowering, Risk-5: the compiler's slot
///    layout is identical to this entry's baseline, so the fast path's
///    guards line up with the cells already recording in flight).
/// 3. **Sync mode** (APX_SYNC_TIERUP=1 debug): keep 02.05c's inline
///    compile exactly, but route the outcome through the driver's state
///    machine so stats/failure ownership have one home in both modes.
///
/// Failure anywhere → DISABLED (never retried, pon
/// failed-tier-up-disables); tier-0 results stay byte-identical.
///
/// 02.05d: the install arms ALSO cache the function's deopt state word
/// (`deopt::word_cell(fn_key)`) on the JitEntry so the per-call dispatch
/// gate in `jit_call_body` is one relaxed load — the pinned-Disabled
/// bypass reads it; a boxed-only body has no fast path to gate.
fn tier_up_step(fn_id: usize, entry: &FnEntry, je: &Rc<JitEntry>) {
    let driver = tierup::global();
    // 1. Poll for a finished background compile (async mode only).
    if !driver.sync_mode() {
        if let Some(handle) = driver.poll(fn_id) {
            ap_codegen_native::runtime::note_tier1_install(handle.regions, &entry.def.name);
            je.tier1_fast.set(handle.fast_ptr);
            // 02.05d: cache the deopt state word so the dispatch loop
            // gates the fast path on it (pon's entry-pointer restore
            // analogue: a DISABLED function dispatches to the twin).
            let fn_key = fn_id as u64;
            *je.deopt_word.borrow_mut() = Some(ap_codegen_native::deopt::word_cell(fn_key));
            // The async compile went through compile_tier1 with this
            // fn_key, which already ran deopt::note_tier1_installed —
            // Tier1/fresh-epoch in the thrash table, matching the install.
            return;
        }
    }
    // 2. Hotness gate + enqueue (exactly-once: the driver's Tier0→Queued
    //    CAS rejects repeat enqueues while a compile is in flight).
    let snapshots: Vec<ap_codegen_native::feedback::CellSnapshot> =
        je.jit.feedback_cells.iter().map(|c| c.snapshot()).collect();
    if !ap_codegen_native::infer::has_int_hotness(&snapshots) {
        return; // not hot yet (or megamorphic) — check again next call
    }
    if driver.sync_mode() {
        // Debug mode: the 02.05c synchronous shape, driver-owned state.
        let fn_key = fn_id as u64;
        match compile_tier1(&je.cfg, &snapshots, fn_key) {
            Ok(Some(t1)) => {
                ap_codegen_native::runtime::note_tier1_install(t1.regions, &entry.def.name);
                driver.note_installed(fn_id);
                // Pin-until-retire, sync variant: the blob must outlive
                // every call through the pointer; JitEntry lives for the
                // process (the 02.05a cache is never evicted), so park
                // the Tier1Func — twin included — right here. The leak is
                // bounded per function exactly as the 02.05c cache was.
                let t1_fast_ptr = t1.fast_ptr;
                std::mem::forget(t1);
                je.tier1_fast.set(t1_fast_ptr);
                // 02.05d: cache the deopt state word (dispatch gate).
                *je.deopt_word.borrow_mut() = Some(ap_codegen_native::deopt::word_cell(fn_key));
                // compile_tier1 already ran note_tier1_installed(fn_key)
                // (Tier1, counters reset) — the thrash table is in sync.
            }
            Ok(None) => {
                // No typed region — permanently boxed (not an error).
                je.tier1_failed.set(true);
                driver.mark_disabled(fn_id);
            }
            Err(e) => {
                if tier_debug() {
                    eprintln!(
                        "[TIER1] fn {} tier-1 compile failed (disabled): {e:?}",
                        entry.def.name
                    );
                }
                je.tier1_failed.set(true);
                driver.mark_disabled(fn_id);
            }
        }
        return;
    }
    // Async enqueue: clone the cfg for the compiler thread (deterministic
    // lowering — same FnDef → identical slot layout, the Risk-5
    // discipline; the recording cells already in flight belong to THIS
    // entry's baseline and stay the twin).
    let _ = driver.request(fn_id, &entry.def.name, je.cfg.clone(), snapshots);
}

/// 02.05c's synchronous tier-up (REMOVED by 02.05e): the compile no longer
/// happens at the call boundary; see `tier_up_step` + tierup.rs.
//
// The compile-cache path (lowering + baseline only — the tier-up hand-off
// above never re-lowers; it clones the cached `cfg`).
fn compile_for(entry: &FnEntry) -> Option<JitEntry> {
    let fn_id = Rc::as_ptr(&entry.def) as usize;
    let func = match cfg::lower_fn(&entry.def) {
        Ok(f) => f,
        Err(e) => {
            if tier_debug() {
                eprintln!("[TIER] fn {} lower: {e}", entry.def.name);
            }
            return None;
        }
    };
    // Slot space: locals first (params 0..n bridged by name), then temps
    // offset above locals (clif.rs temp_slot). Highest temp id = the max
    // temp index emitted — derive from the lowering by re-walking? The
    // cfg stage does not carry a temp count on Func; the safe bound is
    // num_locals + 1 + max TempId seen in clif's emit. Rather than
    // re-walk, scan blocks for the highest TempId.
    let max_temp = func
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .map(temp_of_inst)
        .chain(
            func.blocks
                .iter()
                .filter_map(|b| b.term.as_ref().and_then(term_temp_of)),
        )
        .max()
        .map(|t| t.0 + 1)
        .unwrap_or(0) as u32;
    let num_slots = func.num_locals + max_temp;
    // 02.05f: the tier-0 baseline is compiled WITH live OSR polls (the
    // poll sites probe the host registry under this function's key; with
    // no registry entries they are one helper call + branch per
    // back-edge). Deterministic lowering otherwise — slot layout
    // identical to a compile_baseline compile (Risk-5 contract kept).
    let jit = match ap_codegen_native::clif::compile_baseline_with_osr(&func, fn_id as u64) {
        Ok(j) => j,
        Err(e) => {
            if tier_debug() {
                eprintln!("[TIER] fn {} compile: {e:?}", entry.def.name);
            }
            return None;
        }
    };
    Some(JitEntry {
        jit,
        num_slots,
        local_names: func.local_names.clone(),
        tier1_fast: std::cell::Cell::new(std::ptr::null()),
        tier1_failed: std::cell::Cell::new(false),
        cfg: func,
        deopt_word: std::cell::RefCell::new(None),
    })
}

/// Highest TempId referenced by one inst (for the slot-count bound).
fn temp_of_inst(i: &cfg::Inst) -> cfg::TempId {
    use cfg::Inst;
    let pick = |ts: &[cfg::TempId]| -> cfg::TempId {
        *ts.iter().max_by_key(|t| t.0).unwrap_or(&cfg::TempId(0))
    };
    match i {
        Inst::LoadLocal(_, dst) => *dst,
        Inst::StoreLocal(src, _) => *src,
        Inst::Const(_, dst)
        | Inst::LoadGlobal(_, dst)
        | Inst::UnaryOp(_, _, dst)
        | Inst::LoadAttr(_, _, dst)
        | Inst::Subscript(_, _, dst)
        | Inst::Truthy(_, dst) => *dst,
        Inst::BinOp(_, a, b, dst) | Inst::Compare(_, a, b, dst) => pick(&[*a, *b, *dst]),
        Inst::Call(f, args, dst) => {
            let mut m = *dst;
            for t in args.iter().chain(std::iter::once(f)) {
                if t.0 > m.0 {
                    m = *t;
                }
            }
            m
        }
        Inst::StoreAttr(_, _, v) => *v,
        Inst::MatchExc(_, _, dst) => *dst,
        Inst::Print(v) => *v,
        _ => cfg::TempId(0),
    }
}

fn term_temp_of(t: &cfg::Terminator) -> Option<cfg::TempId> {
    match t {
        cfg::Terminator::Branch(c, _, _) => Some(*c),
        cfg::Terminator::Return(v) | cfg::Terminator::Raise(v, _) => Some(*v),
        cfg::Terminator::Suspend(v, _, _) => Some(*v),
        _ => None,
    }
}

/// Return the frame's cell binding (documented seam where free-cell sync
/// extends beyond params).
#[allow(dead_code)]
fn _cell_slot_of(frame: &Frame, name: &str) -> Option<CellSlot> {
    frame
        .my_cells
        .get(name)
        .cloned()
        .or_else(|| frame.free_cells.get(name).cloned())
}

// ===========================================================================
// 02.05a-residual: eager warm pass
//
// WHY THIS EXISTS: the dispatch arm lives in call_function (interp.rs: the
// ObjKind::Function route), so a body only reaches jit_call_body when its
// function is CALLED at least once. The 16 transparency probes keep every
// module-level def hot, so measured runs DID route bodies — but a run whose
// only def is never called (or whose calls all happen through paths the
// arm doesn't see) reported execs=0 and looked like a silent fallback. The
// warm pass decouples PROOF from CALL PATTERNS: after the registry is
// published (exec_module's init seam), every module-level FnDef body is
// lowered+compiled once (populating the fallback table / compile cache up
// front), so [TIER-STATS] shows compiles+fallbacks for ALL module defs and
// execs counts real jit_call_body executions. The per-body compile-attempt
// idempotence is untouched: the cache key is the def Rc pointer, and the
// warm pass uses the SAME cache slot call_function will hit.
// ===========================================================================

/// Lower + attempt-compile every module-level function body once (warm the
/// compile cache / fallback table). Called from run_module's init seam with
/// the freshly-built module; zero per-call overhead afterwards.
pub fn warm_module_defs(module: &ap_ir::Module) {
    if !tier_mode().jit_enabled() {
        return; // tier 0: the seam is off; nothing to warm, nothing to count
    }
    let interp = crate::interp::current_interp_for_bindings_pub();
    let reg = crate::interp::current_registry_pub();
    for item in &module.items {
        if let ap_ir::Item::FnDef(f) = item {
            // Same id resolution the def-binding pass uses (by_path, exact
            // name); a decorated def defers to Stmt::LocalDef and lands in
            // the nested table with the SAME id space, so a miss here is
            // benign (its warm happens via the cache on first call).
            let Some(id) = reg.by_path_lookup(&f.name) else {
                continue;
            };
            let Some(entry) = reg.entry(id) else {
                continue;
            };
            // Dummy frame: the warm pass only exercises lower+compile (the
            // or_insert_with arm); NO execution happens here, so the frame
            // is never dereferenced. jit_call_body's execs counter must
            // stay call-driven — warm deliberately does not run the body.
            let frame = Rc::new(Frame::new_module(interp.globals.clone()));
            let _ = frame; // compile-only: the frame is never executed against
            if tier_debug() {
                // TEMP residual-diag: why did this body not reach a verdict?
                match cfg::lower_fn(&entry.def) {
                    Ok(_) => eprintln!("[TIER-WARM] fn {} lowerable", entry.def.name),
                    Err(e) => eprintln!("[TIER-WARM] fn {} lower-reject: {e}", entry.def.name),
                }
            }
            warm_compile_only(&entry);
        }
    }
}

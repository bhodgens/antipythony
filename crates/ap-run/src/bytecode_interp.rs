//! BytecodeInterp — the flat-bytecode dispatch loop (fast-path leaf
//! 00-infra/02, plans/fast-path/00-infra/02-frame-and-entry.md).
//!
//! LOCATION NOTE (documented deviation): the leaf spec allows "ap-bytecode
//! or ap-run — your call, document it". This lives in AP-RUN because every
//! opcode's semantics route through the tree-walker's shared helper
//! surfaces (eval_binop / eval_compare / eval_unary / lookup_name /
//! call_value / methods::subscript_get) — all ap-run items, and ap-bytecode
//! is a PURE TRANSLATION layer (no ap-run dep, by design). Moving the loop
//! into ap-bytecode would either invert that dependency or fork the
//! semantics, both forbidden by master.md ("semantics live in shared
//! helpers; no semantic duplication").
//!
//! SHAPE:
//! - One explicit `pc` over `Vec<Instr>`; locals in a fixed slot-indexed
//!   `Vec<Value>`; operand stack `Vec<Value>`. NO recursion for
//!   straight-line code — nested `CallFunction` opcodes recurse through
//!   `call_value` (which re-enters the dispatch arm for bytecode callees),
//!   exactly like the tree-walker. Jump targets are instruction indices
//!   (lowerer-resolved); `CodeObject::validate` bounds are trusted
//!   (buffer-bounds proven at lowering time), with a defensive
//!   pc>=len bail.
//! - LINE BOOKKEEPING (the perf point of the leaf): the CURRENT line is
//!   kept in a thread-local `Cell<u32>` written PER INSTRUCTION (one TLS
//!   store — no lock, no frame write). `frame.last_line` /
//!   `frame.line_track` are synced only at points where user code /
//!   warning machinery can OBSERVE the frame: the instrument ops (BinOp /
//!   UnaryOp / Compare / Subscript / LoadAttr / CallFunction — the ops
//!   whose helpers dispatch user dunders, i.e. where warning emission can
//!   walk py_frames and read the caller's line, s016/s016b/s016c
//!   contract) and on the error exit (exception construction: the frame
//!   exit traceback reads frame.last_line). Per-instruction line source
//!   is the sparse `LineTable` expanded ONCE per CodeObject into a dense
//!   `Vec<u32>` (cached with the compiled body).
//! - SEMANTICS: 100% shared helpers. BinOp -> interp::eval_binop (the
//!   same fn the walker calls). Compare -> interp::eval_compare. Unary ->
//!   interp::eval_unary_pub (extracted verbatim from the walker's arm so
//!   both walkers share one body). Name loads -> lookup_name_pub (the
//!   full locals -> free-cells -> parent-chain -> class-ns -> globals ->
//!   builtins chain — closure reads of enclosing locals resolve through
//!   the same free-cell wiring the walker uses). Calls -> call_value
//!   (correct-but-slow; call-path optimization is 01-core/02).
//! - ENTRY TOGGLE: TierMode::Bytecode (`--tier bytecode` /
//!   APX_TIER=bytecode), default OFF. Off = this module is never
//!   entered (bit-identical to main). On: a function whose body lowers
//!   executes here; lowerer-rejected shapes return Ok(None) and the
//!   dispatch arm falls back to the tree-walker.
//! - COVERAGE: constants, names/locals/globals, arithmetic, comparisons,
//!   attribute/subscript reads, positional calls, tuple/list/dict/set
//!   displays, If/While/For/BoolOp/Break/Continue jumps + loops + aug ops
//!   (leaf 01-core/01): ForNext peeks CPython-style (handle stays on the
//!   stack), AugOp routes the in-place protocol through the shared
//!   aug_binop_value helper, GetIter registers frame-iter ids pruned at
//!   frame exit (G1 discipline). Try/With/Match bodies still fall back
//!   wholesale (01-core/03 owns exceptions).
//!
//! GC SAFETY: the locals vec / operand stack hold `Value` clones on this
//! Rust stack for the frame's life. ap-gc's conservative floor
//! (ap-gc/src/rootscan.rs head note) makes ANY external strong holder a
//! root — same contract every tree-walker Rust local already relies on.
//! This module contains no unsafe Rust.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use ap_bytecode::caches::{AttrPlan, CallPlan, IC_MAGIC};
use ap_bytecode::code::CodeObject;
use ap_bytecode::instr::{BinKind, CompareOp, Instr, UnaryKind};
use ap_bytecode::pool::Const;
use ap_ir::expr::{BinOp, CmpOp, UnaryOp};
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

use crate::exc_hier::ExceptionValue;
use crate::frame::{FnEntry, Frame, Interpreter, Loc};
use crate::ic::IcTable;
use crate::iter::IterState;
use crate::Flow;

// ===========================================================================
// Compile cache (per-thread, keyed by the def Arc pointer — same lifetime
// discipline as jit_hook::JIT_CACHE: the registry pins the def Arc for the
// process, so the key is stable and the compiled body never dangles).
// ===========================================================================

/// One lowered+prepared function body.
struct Compiled {
    code: Arc<CodeObject>,
    /// Sparse LineTable expanded to one line per instruction (built once at
    /// cache-insert; the dispatch loop indexes it directly — no per-step
    /// binary search).
    dense_lines: Arc<Vec<u32>>,
}

thread_local! {
    /// def-ptr -> compiled body (None = lowerer fallback — decided ONCE per
    /// function, never re-attempted per call).
    static BC_CACHE: RefCell<HashMap<usize, Option<Arc<Compiled>>>> =
        RefCell::new(HashMap::new());
    /// THE hot-path line cell (leaf spec: thread-local Cell<u32> written per
    /// instruction; frame atomics touched only at instrument/error points).
    static BC_LINE: Cell<u32> = const { Cell::new(0) };
    /// Leaf fp-inlinecache: per-(thread, code-ptr) inline-cache tables (see
    /// crate::ic header for the no-GIL shape). The compiled-body Arc pin
    /// guarantees the code-ptr key never aliases (the registry holds the
    /// def Arc for the process); tables die with the thread.
    static BC_IC: RefCell<HashMap<usize, std::rc::Rc<IcTable>>> = RefCell::new(HashMap::new());
}

/// IC hit/fill counters (APX_TIER_STATS=1 -> bc_stats_line). Relaxed.
static IC_GLOBAL_HITS: AtomicU64 = AtomicU64::new(0);
static IC_GLOBAL_FILL: AtomicU64 = AtomicU64::new(0);
static IC_ATTR_HITS: AtomicU64 = AtomicU64::new(0);
static IC_ATTR_FILL: AtomicU64 = AtomicU64::new(0);
static IC_CALL_HITS: AtomicU64 = AtomicU64::new(0);

/// This thread's cache table for one compiled body (sized once from the
/// code object's site census).
fn ic_table_for(code: &Arc<CodeObject>) -> std::rc::Rc<IcTable> {
    let key = Arc::as_ptr(code) as usize;
    BC_IC.with(|c| {
        let mut map = c.borrow_mut();
        map.entry(key)
            .or_insert_with(|| {
                let n = code.sites.len();
                std::rc::Rc::new(IcTable::sized(n))
            })
            .clone()
    })
}

/// The site id an instruction at `pc` carries (None = no site on that
/// instruction). Dense SiteTable::by_instr lookup — O(1).
#[inline]
fn ic_site_of(code: &CodeObject, pc: usize) -> Option<u32> {
    code.sites.site_at(pc as u32)
}

/// The receiver's class payload for cacheable attr sites: user instances
/// (their type payload) and user classes (themselves). Primitive receivers
/// (dict/list/str/..) have no class counter to stamp — None (their reads
/// ride the value-family fast arms; classes.rs's interned-Type payload
/// syntheses answer via full walks and stay uncached).
fn receiver_class_ref(obj: &Value) -> Option<Arc<ap_obj::classes::TypeRef>> {
    match obj {
        Value::Obj(o) => match o.kind {
            ap_obj::ObjKind::UserInstance => o
                .type_ref
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .and_then(|w| w.upgrade()),
            ap_obj::ObjKind::UserClass => o
                .class_ref
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            _ => None,
        },
        _ => None,
    }
}

/// Fill an AttrLoad site from a slow-path answer. Mirrors instance_attr /
/// class_attr_get's resolution ORDER (classes.rs) to classify the plan;
/// anything the classifier can't pin to a stamp-guarded surface records
/// NOTHING (the site stays empty — every read re-walks, exact semantics).
///
/// Classification (must match classes.rs instance_attr order):
/// 1. data descriptor on the class -> NOT cacheable (_descr_get dispatch).
/// 2. instance dict hit + NOT a class member shadow -> InstDict.
/// 3. class member: plain Function + not a slot -> BindFunc (func + owner).
///    Plain non-function member (int/str/obj read off the class or via the
///    class receiver) -> ClassValue. Any descriptor kind -> NOT cacheable.
fn ic_attr_fill(ic: &IcTable, site: u32, obj: &Value, name: &str, v: &Value) {
    // try_borrow_mut fill (None = a nested frame of this same body is
    // mid-dispatch on this thread — skip; the site re-fills on a later
    // non-nested miss).
    let Some(t) = receiver_class_ref(obj) else {
        return;
    };
    let stamps = crate::ic::class_stamps(&t);
    // Descriptor check FIRST (data descriptor wins over the instance dict —
    // instance_attr step 1): a property/descriptor answer is dynamic.
    if let Some((entry, _)) = t.lookup_with_owner(name) {
        if crate::classes::is_data_descriptor_scratch(&entry) {
            return;
        }
    }
    let is_inst = matches!(obj, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance);
    let is_cls = matches!(obj, Value::Obj(o) if o.kind == ap_obj::ObjKind::UserClass);
    // Instance dict probe ONCE (step 2 below + the shadow guard for the
    // class-member plans). A ClassValue/BindFunc plan asserts the class
    // supplies the name AND the instance dict does NOT shadow it: an
    // instance write adds no class stamp, so without this guard a later
    // `c.x = ..` / `c.m = ..` would hit the cached class answer (reproduced
    // at ahit>0: warm c.x==1, write c.x=42, read still 1). When shadowed,
    // record NOTHING and re-walk (same sound subset as the InstDict arm).
    let in_inst = instance_dict_has(obj, name);
    if is_inst {
        // Instance receiver: step 2 — instance dict supplies it?
        if in_inst && !t.is_slot(name) {
            // BUT only recordable when the class does NOT supply a member
            // of this name at all (a class member UNDER an instance-dict
            // name is shadowed today, but a later C.x = .. would need the
            // stamps to catch it — they do: names_epoch bumps; still, the
            // dunder/getattribute surfaces make the classification risky,
            // so require NO class member for the record — simplest sound
            // subset, covers the `self.x = ..; read x` loop shape).
            if t.lookup_with_owner(name).is_none() {
                let recorded = ic.with_attrs_mut(|attrs| {
                    let e = &mut attrs[site as usize];
                    e.magic = IC_MAGIC;
                    e.plan = AttrPlan::InstDict as u8;
                    e.type_ptr = stamps.0;
                    e.type_version = stamps.1;
                    e.chain_version = stamps.2;
                    e.names_epoch = stamps.3;
                    e.owner_ptr = 0;
                    // InstDict: the VALUE is never cached (the hit
                    // re-probes the live attrs dict) — leave v = None.
                    let _ = v;
                });
                if recorded.is_some() {
                    IC_ATTR_FILL.fetch_add(1, Ordering::Relaxed);
                }
            }
            return;
        }
    }
    // Class-member resolution (instance step 3 / class step 2). Only PLAIN
    // members record: functions (BindFunc for instances; the class
    // receiver reads the function itself — ClassValue) and non-descriptor
    // values (ClassValue).
    if is_inst && in_inst && !t.is_slot(name) {
        // The slow walk answered from a shadowed class member only because
        // this site still holds an entry from BEFORE the instance write (or
        // a __getattr__/slot race). Clear it so the site stays uncached
        // while the instance dict shadows the name; NEVER record the stale
        // class answer here.
        let _ = ic.with_attrs_mut(|attrs| {
            attrs[site as usize] = Default::default();
        });
        return;
    }
    if let Some((entry, owner)) = t.lookup_with_owner(name) {
        let descr = crate::classes::descriptor_kind_pub(&entry);
        if descr.is_some() {
            return; // dynamic surface
        }
        let is_plain_fn = matches!(&entry, Value::Obj(fo) if fo.kind == ap_obj::ObjKind::Function);
        let plan = if is_inst && is_plain_fn && !t.is_slot(name) {
            AttrPlan::BindFunc
        } else if is_cls || is_inst {
            // Class receiver reading its member (the value IS the answer)
            // or an instance reading a non-function plain member.
            AttrPlan::ClassValue
        } else {
            return;
        };
        let recorded = ic.with_attrs_mut(|attrs| {
            let e = &mut attrs[site as usize];
            e.magic = IC_MAGIC;
            e.plan = plan as u8;
            e.type_ptr = stamps.0;
            e.type_version = stamps.1;
            e.chain_version = stamps.2;
            e.names_epoch = stamps.3;
            e.owner_ptr = Arc::as_ptr(&owner) as usize as u64;
            e.v = entry.clone();
        });
        if recorded.is_some() {
            IC_ATTR_FILL.fetch_add(1, Ordering::Relaxed);
        }
    }
    // No class member, no instance-dict hit: the answer came from a
    // dynamic surface (object virtuals, __getattr__, ...) — record nothing.
}

/// The value-family tag of a primitive receiver (the LoadAttrCall dispatch
/// key — same mapping as the arm's inline match; a fn so the fill helper
/// shares it).
fn tag_of(recv: &Value) -> Option<&'static str> {
    match recv {
        Value::Str(_) => Some("pystr"),
        Value::Dict(_) => Some("pydict"),
        Value::List(_) => Some("pylist"),
        Value::Bytes(_) => Some("pybytes"),
        Value::Int(_) | Value::Bool(_) => Some("pyint"),
        Value::Set(_) | Value::Frozen(_) => Some("pyset"),
        Value::Tuple(_) => Some("pytuple"),
        Value::Float(_) => Some("pyfloat"),
        _ => None,
    }
}

/// True when an instance receiver's live dict holds `name`. The class
/// stamps do NOT cover instance-dict writes, so every ClassValue/BindFunc
/// hit must re-probe this before serving the cached class answer.
#[inline]
fn instance_dict_has(obj: &Value, name: &str) -> bool {
    match obj {
        Value::Obj(o) if o.kind == ap_obj::ObjKind::UserInstance => o
            .attrs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains_str(name),
        _ => false,
    }
}

/// A borrowed attr-entry view for the consult arms (the fields the plan
/// dispatch reads after the stamps validated).
struct AttrHitView {
    plan: u8,
    owner_ptr: u64,
    v: Value,
}

/// Record a fused-call dispatcher decision (leaf fp-inlinecache). Only the
/// routing memoizes — the decision is a pure function of (tag, name, argc),
/// all re-derived at each hit, so there is no invalidation surface.
fn ic_call_fill(ic: &IcTable, site: u32, argc: u16, plan: CallPlan) {
    // Same try_borrow_mut discipline as ic_attr_fill (nested fills skip).
    let _ = ic.with_calls_mut(|calls| {
        debug_assert!((site as usize) < calls.len(), "ic call site OOB");
        let e = &mut calls[site as usize];
        e.magic = IC_MAGIC;
        e.plan = plan as u8;
        e.argc = argc;
    });
}

/// Exercise counters (APX_TIER_STATS=1 -> bc_stats_line). Transparency
/// discipline (02.05a-residual): a toggle that silently tree-walks
/// everything would pass every output gate — these counters make the route
/// observable, and fix_bytecode.py asserts execs > 0 on the bytecode leg.
static BC_COMPILES: AtomicU64 = AtomicU64::new(0);
static BC_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static BC_EXECS: AtomicU64 = AtomicU64::new(0);

// === fp-superinstructions: opt-in opcode histogram ==========================
// APX_BP_STATS=1 turns on per-dispatch opcode/pair/triple counting. One env
// read at process start; the dispatch loop's `if bp_on` check is a single
// predicted branch when the knob is off (zero atomics/locks/allocations).
static BP_HIST: OnceLock<ap_bytecode::stats::OpHistogram> = OnceLock::new();

/// Read once at the dispatch-loop seam (profile-only knob).
#[inline]
fn bp_stats_on() -> bool {
    std::env::var("APX_BP_STATS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Emit the top-20 pair/triple histogram table to stderr (called from the
/// process-exit stats hook, only when APX_BP_STATS=1). Returns None when no
/// dispatch was ever recorded (toggle-off / tree-walk-only runs).
pub fn bp_hist_line() -> Option<String> {
    if !bp_stats_on() {
        return None;
    }
    let h = BP_HIST.get()?;
    if h.report(20).starts_with("[BP-HIST] total_dispatches=0\n") {
        return None;
    }
    Some(h.report(20))
}

/// One-line stats report (called from jit_hook::report_line under
/// APX_TIER_STATS=1). Empty when nothing was ever compiled (toggle-off runs
/// print nothing — zero output-channel footprint by default).
pub fn bc_stats_line() -> Option<String> {
    let compiles = BC_COMPILES.load(Ordering::Relaxed);
    let fallbacks = BC_FALLBACKS.load(Ordering::Relaxed);
    let execs = BC_EXECS.load(Ordering::Relaxed);
    let gh = IC_GLOBAL_HITS.load(Ordering::Relaxed);
    let gf = IC_GLOBAL_FILL.load(Ordering::Relaxed);
    let ah = IC_ATTR_HITS.load(Ordering::Relaxed);
    let af = IC_ATTR_FILL.load(Ordering::Relaxed);
    let ch = IC_CALL_HITS.load(Ordering::Relaxed);
    if compiles == 0 && fallbacks == 0 && execs == 0 {
        return None;
    }
    Some(format!(
        "[BC-STATS] compiles={compiles} fallbacks={fallbacks} execs={execs} \
         ic: ghit={gh} gfill={gf} ahit={ah} afill={af} chit={ch}"
    ))
}

// ===========================================================================
// Entry (the dispatch arm's callee — mirrors jit_hook::jit_call_body's
// Ok(None)-means-fallback contract).
// ===========================================================================

/// Route one function body through the BytecodeInterp.
///
/// Ok(None) = fallback (lowerer rejected / generator / aug-assign body) —
/// the caller runs the tree-walker, byte-identical. Some(v) = the body's
/// return value. Err(Flow::Raise(..)) = the body raised.
pub fn call_body_bytecode(
    interp: &Arc<Interpreter>,
    entry: &FnEntry,
    frame: &Arc<Frame>,
) -> Result<Option<Value>, Flow> {
    // Generators own fiber machinery (parked frames + resume) — the flat
    // frame here cannot suspend. yield is lowerer-rejected anyway
    // (belt-and-suspenders at the cheaper site).
    if entry.def.is_generator {
        return Ok(None);
    }
    let fn_id = Arc::as_ptr(&entry.def) as usize;
    let compiled = BC_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache
            .entry(fn_id)
            .or_insert_with(|| compile_for(entry))
            .clone()
    });
    let Some(c) = compiled else {
        return Ok(None); // fallback table
    };
    BC_EXECS.fetch_add(1, Ordering::Relaxed);
    run_frame(interp, entry, frame, &c)
}

/// Lower + prepare once per function. None = tree-walker fallback.
fn compile_for(entry: &FnEntry) -> Option<Arc<Compiled>> {
    if entry.def.is_generator {
        return None;
    }
    // AugAssign lowers to the dedicated AugOp opcode since leaf 01-core/01
    // (in-place protocol inside the shared aug_binop_value helper) — no
    // refusal needed anymore. Everything else the runtime semantics need is
    // either lowered faithfully or lowerer-rejected at function granularity.
    let code = match ap_bytecode::lower_fn(&entry.def) {
        ap_bytecode::lower::LowerOutcome::Code(code) => code,
        ap_bytecode::lower::LowerOutcome::Fallback(_) => {
            BC_FALLBACKS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
    };
    debug_assert!(
        code.validate().is_ok(),
        "lowerer emitted an invalid CodeObject for {}",
        entry.def.name
    );
    let dense = dense_lines_of(&code);
    BC_COMPILES.fetch_add(1, Ordering::Relaxed);
    Some(Arc::new(Compiled {
        code: Arc::new(code),
        dense_lines: Arc::new(dense),
    }))
}

/// Expand the sparse table: one line per instruction. The lowerer always
/// records instr 0 (the def line), so the table is non-empty for any
/// lowered body; a (defensive) empty table maps everything to the def line.
fn dense_lines_of(code: &CodeObject) -> Vec<u32> {
    let mut dense = vec![code.def_line as u32; code.instrs.len()];
    let mut cur = code.def_line as u32;
    let mut it = code.lines.iter();
    // Entries are sorted by instr_idx (LineTable invariant) — fill forward.
    let mut next = it.next();
    for (i, slot) in dense.iter_mut().enumerate() {
        while let Some(e) = next {
            if e.instr_idx as usize <= i {
                cur = e.line;
                next = it.next();
            } else {
                break;
            }
        }
        *slot = cur;
    }
    dense
}

// ===========================================================================
// The dispatch loop
// ===========================================================================

/// Sync the frame's observable line cells (instrument points + error exit).
/// Two Cell writes, no locks — the walker's per-instrument discipline
/// (s016b/c) minus the per-statement ordinal machinery.
fn sync_frame_line(frame: &Frame, line: u32) {
    let l = line.max(1) as usize;
    frame.last_line.set(l);
    frame.line_track.set(l);
}

fn raise_flow(class: &str, msg: impl Into<String>) -> Flow {
    Flow::Raise(ExceptionValue::msg(class, msg))
}

// ===========================================================================
// fp-exceptions: the in-flight exception machinery (bytecode-mode mirror of
// exec_try's handler protocol). ONE parked-exception slot per thread — the
// CheckExcept/PopExc/Raise ops consult it; no cost on non-exception paths.
// ===========================================================================

thread_local! {
    /// The parked exception between the unwind and the handler's
    /// CheckExcept (None whenever no bytecode try is mid-catch). The
    /// PRE-handler current_exception rides alongside (the handler body's
    /// raises chain against it — exec_try's exact current_exception
    /// save/restore protocol), plus the tb_frames snapshot base for the
    /// handled-path truncate (exec_try's tb_snapshot discipline).
    static BC_PARKED_EXC: RefCell<Option<(ExceptionValue, Option<ExceptionValue>, usize)>> =
        const { RefCell::new(None) };
}

/// Park an arriving exception at the handler boundary (arc unwind).
/// Mirrors exec_try's catch-entry tb work: the interpreter-wide tb_frames
/// is first truncated back to THIS frame's entry base (`tb_base` —
/// dropping the callee frame-exit pushes the raising unwind accumulated;
/// exec_try's tb_snapshot discipline — callee frames die with the
/// exception), then the CATCHING frame's entry is prepended (raise_line
/// when the tb is empty — the W112 raise-line pin — else this frame's
/// current line), deduped; the snapshot rides the park for the
/// handled-path truncate (PopExc).
fn exc_enter(interp: &Arc<Interpreter>, frame: &Arc<Frame>, exc: ExceptionValue, tb_base: usize) {
    let prev = interp
        .current_exception
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .map(|a| (*a).clone());
    // exec_try's tb_snapshot: the frame's entry base (callee pushes die).
    {
        let mut d = interp.tb_frames.lock().unwrap_or_else(|p| p.into_inner());
        let at = tb_base.min(d.len());
        let _ = d.split_off(at);
    }
    let tb_snapshot = tb_base;
    BC_PARKED_EXC.with(|c| {
        *c.borrow_mut() = Some((exc.clone(), prev, tb_snapshot));
    });
    *interp
        .current_exception
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(exc.clone()));
    // exec_try's catch-entry insert (W112 block): the catching frame's
    // traceback entry, outermost-first.
    let line = if exc.tb.is_empty() && exc.raise_line > 0 {
        exc.raise_line.max(1)
    } else {
        frame.last_line.get().max(1)
    };
    let file = frame
        .globals
        .get(&Value::Str(PyString::new("__file__")))
        .and_then(|fv| match fv {
            Value::Str(s) => Some(s.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| interp.script_path.clone());
    let already = exc
        .tb
        .first()
        .is_some_and(|(q, l, f)| q == &frame.qualname && *l == line && f == &file);
    if !already {
        // exc is already parked/current — mutate the parked copy too.
        let mut exc2 = exc;
        exc2.tb
            .insert(0, (frame.qualname.clone(), line, file.clone()));
        interp
            .tb_frames
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_front((frame.qualname.clone(), line, file));
        *interp
            .current_exception
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(exc2.clone()));
        BC_PARKED_EXC.with(|c| {
            if let Some(slot) = &mut *c.borrow_mut() {
                slot.0 = exc2;
            }
        });
    }
}

/// CheckExcept (typed matcher): peek the parked exception, evaluate
/// `matches` via the shared exc_matches route (the same MRO/tuple/bare
/// semantics the walker's exec_try uses), pop the type expr, push the
/// instance (or None filler) on match. Err(raise) = the type-expr
/// evaluation raised (catch-time TypeError etc.).
fn check_except(interp: &Arc<Interpreter>, ty: &Value) -> Result<Value, Flow> {
    let parked = BC_PARKED_EXC.with(|c| c.borrow().as_ref().map(|(e, _, _)| e.clone()));
    let Some(exc) = parked else {
        return Err(raise_flow(
            "SystemError",
            "CheckExcept with no parked exception",
        ));
    };
    let matches = crate::interp::exc_matches_pub(interp, ty, &exc)?;
    if matches {
        Ok(exception_instance_value(&exc))
    } else {
        Ok(Value::None) // filler: the dispatch arm discards it (no-match arc)
    }
}

/// CheckExceptBare: always matches; pushes the instance.
fn check_except_bare() -> Value {
    let parked = BC_PARKED_EXC.with(|c| c.borrow().as_ref().map(|(e, _, _)| e.clone()));
    match parked {
        Some(exc) => exception_instance_value(&exc),
        None => Value::None,
    }
}

/// PopExc { unbind_slot }: pop the park (restoring the pre-try
/// current_exception + truncating tb_frames to the snapshot — the handled
/// path's raising frames die with the handled exception) and unbind the
/// except-as name.
fn pop_exc(
    interp: &Arc<Interpreter>,
    frame: &Frame,
    unbind_slot: u16,
    locals: &mut [Option<Value>],
    code: &CodeObject,
) {
    let saved = BC_PARKED_EXC.with(|c| c.borrow_mut().take());
    if let Some((_, prev, tb_snapshot)) = saved {
        {
            let mut d = interp.tb_frames.lock().unwrap_or_else(|p| p.into_inner());
            let at = tb_snapshot.min(d.len());
            let _ = d.split_off(at);
        }
        *interp
            .current_exception
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = prev.map(Arc::new);
    }
    if unbind_slot != u16::MAX {
        if let Some(name) = code.local_names.get(unbind_slot as usize) {
            let _ = frame.delete_name(name);
            if (unbind_slot as usize) < locals.len() {
                locals[unbind_slot as usize] = None;
            }
        }
    }
}

/// Raise { mode }: the tree-walker's Stmt::Raise arm semantics — bare
/// re-raise (in-flight/current), value raise, or `raise X from Y`
/// chaining. raise_line = the frame's synced last_line.
fn raise_op(
    interp: &Arc<Interpreter>,
    frame: &Frame,
    mode: u8,
    exc_val: Option<Value>,
    cause_val: Option<Value>,
) -> Result<Flow, Flow> {
    // Bare raise (mode 2): re-raise current/in-flight.
    let Some(v) = exc_val else {
        let inflight = interp
            .current_exception
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .map(|a| (*a).clone());
        let mut e = match inflight {
            Some(prev) => prev,
            None => {
                return Err(raise_flow(
                    "RuntimeError",
                    "No active exception to re-raise",
                ))
            }
        };
        e.raise_line = frame.last_line.get();
        return Ok(Flow::Raise(e));
    };
    let mut e = crate::interp::raise_from_value_pub(&v)?;
    if mode == 1 {
        if let Some(cv) = cause_val {
            let cause = if matches!(cv, Value::None) {
                None // raise X from None suppresses context (S-06-8)
            } else {
                Some(Box::new(crate::interp::raise_from_value_pub(&cv)?))
            };
            e.cause = cause;
            // CPython pin: `from Y` does NOT clear __context__ — the
            // implicit chain stays; __suppress_context__=True hides it
            // from the render. from None chains too.
            if e.context.is_none() {
                if let Some(prev) = interp
                    .current_exception
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                {
                    let prev = (*prev).clone();
                    if prev.class != "StopIteration" || !prev.args.is_empty() {
                        e.context = Some(Box::new(prev));
                    }
                }
            }
            if let Some(Value::Obj(io)) = &e.instance {
                let mut a = io.attrs.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(ctx) = &e.context {
                    let _ = a.set(
                        Value::Str(PyString::new("__context__")),
                        crate::interp::exception_instance_pub(ctx),
                    );
                }
                if let Some(cause) = &e.cause {
                    let _ = a.set(
                        Value::Str(PyString::new("__cause__")),
                        crate::interp::exception_instance_pub(cause),
                    );
                }
                let _ = a.set(
                    Value::Str(PyString::new("__suppress_context__")),
                    Value::Bool(true), // any explicit `from` suppresses
                );
            }
            e.raise_line = frame.last_line.get();
            return Ok(Flow::Raise(e));
        }
    }
    // Implicit __context__ chaining AT RAISE TIME (mode 0/2) — the walker
    // Raise arm's exact fp-exceptions block (single-sourced semantics).
    if e.context.is_none() && e.cause.is_none() {
        if let Some(prev) = interp
            .current_exception
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            let prev = (*prev).clone();
            // Self-chain guard (re-raise must not chain itself — infinite
            // render recursion); identical to the walker Raise arm.
            let same_inst = e
                .instance
                .as_ref()
                .zip(prev.instance.as_ref())
                .map(|(a, b)| match (a, b) {
                    (Value::Obj(x), Value::Obj(y)) => std::sync::Arc::ptr_eq(x, y),
                    _ => false,
                })
                .unwrap_or(false);
            let is_same = same_inst
                || (e.instance.is_none()
                    && prev.instance.is_none()
                    && e.class == prev.class
                    && e.raise_line == prev.raise_line
                    && e.args.len() == prev.args.len());
            if !is_same && (prev.class != "StopIteration" || !prev.args.is_empty()) {
                e.context = Some(Box::new(prev));
            }
        }
    }
    if e.context.is_some() || e.cause.is_some() {
        if let Some(Value::Obj(io)) = &e.instance {
            let mut a = io.attrs.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(ctx) = &e.context {
                let _ = a.set(
                    Value::Str(PyString::new("__context__")),
                    crate::interp::exception_instance_pub(ctx),
                );
            }
            if let Some(cause) = &e.cause {
                let _ = a.set(
                    Value::Str(PyString::new("__cause__")),
                    crate::interp::exception_instance_pub(cause),
                );
            }
            let _ = a.set(
                Value::Str(PyString::new("__suppress_context__")),
                Value::Bool(e.cause.is_some()),
            );
        }
    }
    e.raise_line = frame.last_line.get();
    Ok(Flow::Raise(e))
}

/// Materialize the handler-bound exception instance (exception_instance's
/// route — the same Value the walker binds to `except E as e`).
fn exception_instance_value(exc: &ExceptionValue) -> Value {
    crate::interp::exception_instance_pub(exc)
}

// ===========================================================================
// Smi fast paths (leaf 01-core/01 perf targets). The shared helpers'
// generality (user-dunder walks, exc machinery, bool allocs) costs
// ~200ns/step; the loop targets need ~20ns. These arms run ONLY when the
// operand SHAPES guarantee identical semantics (raw smi ints; for floor
// mod, positive operands); any other shape falls to the shared helper —
// semantics live in the shared helpers, this is a guarded entry into
// them, not a fork.
// ===========================================================================

/// Raw smi x op y for the pure-int arith ops whose ap_obj::ops::binop int
/// path is exactly this (Int::add/sub/mul are checked and Smi-or-bigint
/// promote identically — an overflow here means the slow path would
/// return a Big; None = take the slow path). Never handles Mod (floor-mod
/// sign rules) — that stays on the shared helper.
#[inline]
fn fast_smi_binop(a: &Value, op: ap_ir::BinOp, b: &Value) -> Option<Value> {
    let (Value::Int(x), Value::Int(y)) = (a, b) else {
        return None;
    };
    let (Int::Smi(xi), Int::Smi(yi)) = (x, y) else {
        return None; // bigint arm: full machinery
    };
    let v = match op {
        ap_ir::BinOp::Add => xi.checked_add(*yi)?,
        ap_ir::BinOp::Sub => xi.checked_sub(*yi)?,
        ap_ir::BinOp::Mul => xi.checked_mul(*yi)?,
        _ => return None,
    };
    Some(Value::Int(Int::Smi(v)))
}

/// The attr_get fast_eligible arm's exact (receiver, name) -> fresh
/// BoundMethod mint, without the deep walk. Mirrors its family gates:
/// native_attr claims pylist/pydict/pyset method tables FIRST (those
/// receivers' surfaces ride the native dispatcher — dotted marker mints,
/// NOT bare names), so a native_attr hit here would be wrong; the walk's
/// arm only answers when native_attr returns None. For the four primitive
/// receivers + these names native_attr provably returns None (the tables
/// claim dotted names like "pylist.sort", not bare "append") — verified
/// against builtins_modules::native_attr's match shapes.
#[inline]
fn fast_bound_mint(obj: &Value, name: &str) -> Option<Value> {
    let family_ok = match obj {
        Value::List(_) => {
            matches!(
                name,
                "append" | "insert" | "sort" | "reverse" | "extend" | "remove"
            )
        }
        Value::Dict(_) => matches!(
            name,
            "keys"
                | "values"
                | "items"
                | "update"
                | "setdefault"
                | "popitem"
                | "get"
                | "fromkeys"
                | "pop"
        ),
        Value::Set(_) | Value::Frozen(_) => matches!(
            name,
            "difference"
                | "difference_update"
                | "intersection"
                | "intersection_update"
                | "union"
                | "update"
                | "copy"
                | "clear"
                | "discard"
                | "remove"
                | "pop"
                | "issubset"
                | "issuperset"
                | "symmetric_difference"
                | "symmetric_difference_update"
                | "add"
        ),
        // Leaf fp-calls: str/bytes/int/float/tuple surfaces — the same
        // (tag, name) table native_attr's mint arm consults. A hit means
        // the deep walk would answer with the dotted-marker mint below;
        // answering HERE skips ~1.2k lines of sequential checks.
        Value::Str(_) => crate::builtins_modules::builtin_value_method_exists_pub("pystr", name),
        Value::Bytes(_) => {
            crate::builtins_modules::builtin_value_method_exists_pub("pybytes", name)
        }
        Value::Int(_) | Value::Bool(_) => {
            crate::builtins_modules::builtin_value_method_exists_pub("pyint", name)
        }
        Value::Float(_) => {
            crate::builtins_modules::builtin_value_method_exists_pub("pyfloat", name)
        }
        Value::Tuple(_) => {
            crate::builtins_modules::builtin_value_method_exists_pub("pytuple", name)
        }
        _ => false,
    };
    if !family_ok {
        return None;
    }
    // CPython identity pin: a fresh BoundMethod per read (d.get is d.get
    // is False) — never memoize. Same mint site as the walk's arm.
    // Mint name: the walk's arm answers the value-table families with the
    // DOTTED marker ("pystr.startswith" — native_attr's exact mint; the
    // BoundMethod call route dispatches dotted markers through
    // call_named_kw -> call_builtin_values). list/dict/set families keep
    // the BARE name (methods::call_method's own arms answer those).
    let mint_name = match obj {
        Value::Str(_) => format!("pystr.{name}"),
        Value::Bytes(_) => format!("pybytes.{name}"),
        Value::Int(_) | Value::Bool(_) => format!("pyint.{name}"),
        Value::Float(_) => format!("pyfloat.{name}"),
        Value::Tuple(_) => format!("pytuple.{name}"),
        _ => name.to_string(),
    };
    let m = ap_obj::PyObject::with_self(ap_obj::ObjKind::BoundMethod, mint_name, obj.clone());
    Some(Value::Obj(m))
}

/// Truthiness for the hot shapes with the EXACT semantics of
/// interp::truthy_public (Int: != 0; Bool: itself; None: false). Every
/// other shape (str/list/tuple/dict/user) falls to the shared helper.
#[inline]
fn fast_truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Int(Int::Smi(n)) => Some(*n != 0),
        Value::Bool(b) => Some(*b),
        Value::None => Some(false),
        _ => None,
    }
}

/// Outcome of the frame: its return value, or (line-at-failure, flow).
type FrameOutcome = Result<Value, (u32, Flow)>;

fn run_frame(
    interp: &Arc<Interpreter>,
    entry: &FnEntry,
    frame: &Arc<Frame>,
    c: &Compiled,
) -> Result<Option<Value>, Flow> {
    let code = &c.code;
    let n_instr = code.instrs.len();

    // Fixed locals vec, slot-indexed. `None` = UNBOUND (the
    // never-assigned sentinel); `Some(Value::None)` is a real `x = None`
    // store — the two must stay distinct (a stored None reads fine; an
    // unbound slot raises the tree-walker's exact UnboundLocalError text).
    // Params prefill from the frame the call boundary built (Loc::Slot or
    // a cell-bound param).
    let mut locals: Vec<Option<Value>> = vec![None; code.n_locals];
    {
        let fl = frame.locals.borrow();
        for (i, p) in entry.def.params.iter().enumerate() {
            if (i as usize) >= locals.len() {
                break;
            }
            let v = match fl.get(&p.name) {
                Some(Loc::Slot(s)) => s.borrow().clone(),
                Some(Loc::CellSlot(cell)) => cell.borrow().clone(),
                None => None,
            };
            locals[i] = v;
        }
    }
    // Defaults / kw-only / *args / **kwargs: the call boundary's binder
    // filled EVERY param name into the frame's locals map (defaults via
    // eval_default, VarArg as a tuple, KwArg as a dict) BEFORE dispatch.
    // The loop above only prefills the first N slots (source order); bind
    // any param that landed at a later slot or under a different name
    // position by NAME — the compile-time slot table is authoritative.
    {
        let fl = frame.locals.borrow();
        for p in &entry.def.params {
            if let (Some(slot), Some(v)) = (code.visible_slot_of(&p.name), fl.get(&p.name)) {
                if locals.len() > slot as usize && locals[slot as usize].is_none() {
                    let lv = match v {
                        Loc::Slot(s) => s.borrow().clone(),
                        Loc::CellSlot(cell) => cell.borrow().clone(),
                    };
                    if lv.is_some() {
                        locals[slot as usize] = lv;
                    }
                }
            }
        }
    }

    let mut stack: Vec<Value> = Vec::with_capacity(8);
    let mut pc: usize = 0;
    // fp-superinstructions profile: one env read per FRAME (cheap; the
    // stats path is diagnostic-only). When off, the per-dispatch `if bp_on`
    // below is a single predicted branch — no atomics, locks or allocation.
    let bp_on = bp_stats_on();
    if bp_on {
        BP_HIST.get_or_init(ap_bytecode::stats::OpHistogram::default);
    }
    let mut bp_prev1: u16 = ap_bytecode::stats::NO_OP;
    let mut bp_prev2: u16 = ap_bytecode::stats::NO_OP;
    let mut cur_line: u32 = c.dense_lines.first().copied().unwrap_or(1);
    // fp-exceptions: the interpreter-wide tb_frames length at frame ENTRY —
    // exc_enter truncates back to this before snapshotting (the callee
    // frame-exit pushes of the raising unwind die with the exception).
    let tb_base = interp
        .tb_frames
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .len();
    // Registered even-id iterators this frame created (GetIter) — pruned at
    // frame exit (break/unwind AND exhaustion both drop their entries inside
    // pull_iter; this covers the BREAK/mid-stream exits, the G1 discipline).
    let mut frame_iter_ids: Vec<u64> = Vec::new();
    // ForNext handle cache (leaf 01-core/01): the most recent GetIter'd
    // iterator Arc, matched against the peeked stack handle by pointer
    // identity — skips the iterators-table Mutex on every loop step.
    let mut cached_it: Option<std::sync::Arc<crate::iter::PyIterator>> = None;
    // Leaf fp-inlinecache: this thread's per-code cache table (shared Rc
    // across frames of the same body on this thread; filled on miss). The
    // local binding must stay MUTABLE and unique for fills — `ic` is the
    // only Rc handle on this thread's stack during dispatch (the map holds
    // its own clone).
    let ic = ic_table_for(code);

    // Macro: propagate a helper failure as (current line, flow) so the exit
    // path can sync the frame line before the traceback is built. The macro
    // BODIES live inside the dispatch loop below (hygiene: `break 'dispatch`
    // resolves against the label in scope at the macro definition site).

    let outcome: FrameOutcome = 'dispatch: loop {
        // Macros defined INSIDE the labeled loop: `break 'dispatch` in a
        // macro body only resolves against labels in scope where the macro
        // was DEFINED (hygiene), so these must sit after the loop label.
        macro_rules! hop {
            ($r:expr) => {
                match $r {
                    Ok(v) => v,
                    Err(f) => {
                        // fp-exceptions: a RAISE in flight routes through
                        // this frame's exception table (zero-cost until
                        // raised). No arc -> propagate (frame Err exit).
                        if let Flow::Raise(ref exc) = f {
                            if let Some(entry) = code.excs.find_handler(pc as u32) {
                                stack.truncate(entry.stack_depth as usize);
                                exc_enter(interp, frame, exc.clone(), tb_base);
                                pc = entry.handler as usize;
                                continue 'dispatch;
                            }
                        }
                        break 'dispatch Err((cur_line, f));
                    }
                }
            };
        }
        // fp-exceptions: route an INLINE-CONSTRUCTED raise through the
        // exception table (same arc lookup hop! does) — LoadAttr's
        // AttributeError, the unbound-load NameError family, GetIter's
        // TypeError, the fast LoadAttrCall arms all construct their errors
        // inline and must reach the handler like helper raises do.
        macro_rules! raise_or_propagate {
            ($f:expr) => {{
                let f = $f;
                if let Flow::Raise(ref exc) = f {
                    if let Some(entry) = code.excs.find_handler(pc as u32) {
                        stack.truncate(entry.stack_depth as usize);
                        exc_enter(interp, frame, exc.clone(), tb_base);
                        pc = entry.handler as usize;
                        continue 'dispatch;
                    }
                }
                break 'dispatch Err((cur_line, f));
            }};
        }
        macro_rules! pop {
            () => {
                match stack.pop() {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode operand stack underflow"),
                        ))
                    }
                }
            };
        }
        if pc >= n_instr {
            // Defensive: the lowerer appends Return{Value,None}, so control
            // never runs off the end. Bail loudly rather than wrap.
            break Err((
                cur_line,
                raise_flow("SystemError", "bytecode pc ran off the instruction stream"),
            ));
        }
        let ins = code.instrs[pc];
        // fp-superinstructions profile (APX_BP_STATS=1 only). Record the
        // dynamic predecessor chain at the dispatch boundary. A jump BREAKS
        // adjacency (the next dispatch's predecessor is unrelated), which
        // is exactly what a peephole may rewrite: only fall-through pairs
        // are fusable. `continue` skips a frame, so preds live in locals.
        if bp_on {
            let h = BP_HIST.get().expect("bp hist initialized");
            let cur = ins.opcode_index() as u16;
            h.record(cur, bp_prev1, bp_prev2);
            if ins.is_jump() {
                bp_prev1 = ap_bytecode::stats::NO_OP;
                bp_prev2 = ap_bytecode::stats::NO_OP;
            } else {
                bp_prev2 = bp_prev1;
                bp_prev1 = cur;
            }
        }
        // Line bookkeeping: the LOCAL cur_line drives every error/exit sync
        // (sync_frame_line). Leaf fp-calls FIX: cur_line now follows the
        // dense line table (one Vec index per instruction, ~1ns) — the
        // pre-leaf code initialized it once at frame start and NEVER
        // advanced it, so every error exit / instrument sync reported the
        // def-line region (exposed by the leaf's unbound-probe: a NameError
        // at `return x + missing` reported the `x = 1` line). s016
        // attribution reads frame.last_line (synced at instrument ops +
        // error exit) — now with the CORRECT per-instruction line.

        cur_line = *c
            .dense_lines
            .get(pc)
            .unwrap_or_else(|| c.dense_lines.first().unwrap_or(&1));
        match ins {
            // ---- constants -------------------------------------------------
            Instr::LoadConst(idx) => {
                let v = match const_value(code, idx) {
                    Ok(v) => v,
                    Err(f) => break Err((cur_line, f)),
                };
                stack.push(v);
                pc += 1;
            }
            Instr::LoadConstNone => {
                stack.push(Value::None);
                pc += 1;
            }
            Instr::LoadConstTrue => {
                stack.push(Value::Bool(true));
                pc += 1;
            }
            Instr::LoadConstFalse => {
                stack.push(Value::Bool(false));
                pc += 1;
            }
            Instr::LoadConstEmptyTuple => {
                stack.push(Value::Tuple(Gc::new(Tuple(Vec::new()))));
                pc += 1;
            }

            // ---- names ------------------------------------------------------
            Instr::LoadFast(slot) => {
                let s = slot as usize;
                // Hot path first: bound slot -> one clone. The bounds check
                // rides the same branch (slots are lowerer-proven < n_locals,
                // this is belt-and-braces).
                match locals.get(s) {
                    Some(Some(v)) => stack.push(v.clone()),
                    Some(None) => {
                        // Unbound local: the tree-walker's exact error shape.
                        // (Some(Value::None) is a real `x = None` store and
                        // reads fine — the Option IS the unbound marker.)
                        let name = code
                            .local_names
                            .get(s)
                            .map(|n| n.as_str())
                            .unwrap_or("<slot>");
                        raise_or_propagate!(raise_flow(
                            "UnboundLocalError",
                            format!(
                                "cannot access local variable '{name}' where it is not \
                                 associated with a value"
                            ),
                        ));
                    }
                    None => {
                        break Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode local slot out of range"),
                        ));
                    }
                }
                pc += 1;
            }
            Instr::StoreFast(slot) => {
                let s = slot as usize;
                if s >= locals.len() {
                    break Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode local slot out of range"),
                    ));
                }
                let n = stack.len();
                if n == 0 {
                    break 'dispatch Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode operand stack underflow"),
                    ));
                }
                // Swap the value out of the stack slot in place (no clone).
                let v = std::mem::replace(&mut stack[n - 1], Value::None);
                stack.truncate(n - 1);
                locals[s] = Some(v);
                pc += 1;
            }
            Instr::LoadGlobal(idx) => {
                // NOT a raw globals-dict read: resolve through the
                // tree-walker's full chain (locals -> free cells -> parent
                // chain -> class ns -> globals -> builtins). The lowerer
                // emits LoadGlobal only for names with no slot — closure
                // reads of enclosing locals resolve through the free-cell
                // wiring exactly as the walker does.
                //
                // INLINE CACHE (leaf fp-inlinecache): module/builtin reads
                // resolve from the (globals, builtins) dict pair; the cache
                // stamps that pair's epochs + identity, so a warm hit is
                // two epoch loads + two ptr compares + a direct unpack.
                // LookupName chain shapes (closure cells, class-ns frames,
                // the T1-d module-globals/exc-class fallbacks) are NOT
                // cacheable — they record nothing and stay fully dynamic.
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<name>");
                if let Some(site) = ic_site_of(code, pc) {
                    let g_epoch = frame.globals.mutation_epoch();
                    let b_epoch = interp.builtins.mutation_epoch();
                    let g_ptr = crate::ic::gc_ptr(&frame.globals);
                    let b_ptr = crate::ic::gc_ptr(&interp.builtins);
                    let hit = ic.with_globals(|g| {
                        debug_assert!((site as usize) < g.len(), "ic global site OOB");
                        g[site as usize].is_hit(g_epoch, b_epoch, g_ptr, b_ptr)
                    });
                    if let Some(v) = hit {
                        IC_GLOBAL_HITS.fetch_add(1, Ordering::Relaxed);
                        stack.push(v);
                        pc += 1;
                        continue;
                    }
                    match crate::interp::lookup_name_pub(interp, frame, name) {
                        Ok(v) => {
                            // Recordable = answered from the dict pair. The
                            // cheap probe: the name present in globals OR
                            // builtins right now — that's exactly where the
                            // chain found it (locals/cells/class-ns would
                            // have made it unbound in the FRAME map the
                            // lowerer proves absent; module-globals/exc
                            // fallbacks answer when BOTH dict probes miss,
                            // so a present-check mirrors the provenance).
                            let recordable = frame.globals.contains_str(name)
                                || interp.builtins.contains_str(name);
                            if recordable {
                                let recorded = ic.with_globals_mut(|g| {
                                    let e = &mut g[site as usize];
                                    e.magic = IC_MAGIC;
                                    e.kind = ap_bytecode::caches::global_kind::POSITIVE;
                                    e.globals_epoch = g_epoch;
                                    e.builtins_epoch = b_epoch;
                                    e.globals_ptr = g_ptr;
                                    e.builtins_ptr = b_ptr;
                                    e.v = v.clone();
                                });
                                if recorded.is_some() {
                                    IC_GLOBAL_FILL.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            stack.push(v);
                            pc += 1;
                        }
                        Err(f) => break Err((cur_line, f)),
                    }
                } else {
                    let v = hop!(crate::interp::lookup_name_pub(interp, frame, name));
                    stack.push(v);
                    pc += 1;
                }
            }
            Instr::StoreGlobal(idx) => {
                // Module-namespace store. Unreachable for bytecode-routed
                // function bodies (every assigned name gets a slot; `global`
                // declarations are lowerer-rejected) — defensive arm writes
                // the frame's globals dict directly.
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.clone())
                    .unwrap_or_else(|| "<name>".to_string());
                let v = pop!();
                frame.store_global(&name, v);
                pc += 1;
            }

            // ---- stack ------------------------------------------------------
            Instr::PopTop => {
                let _ = pop!();
                pc += 1;
            }

            // ---- operators (instrument ops: sync the frame line first —
            // their helpers dispatch user dunders / raise with the frame's
            // line in the traceback) ---------------------------------------
            Instr::BinOp(kind) => {
                sync_frame_line(frame, cur_line);
                // No-clone pops for the smi fast path: peek both operands
                // through references; only the SLOW path (shared helper)
                // clones (it takes owned Values). The n = stack.len()
                // guard is the underflow check (pop! equivalent).
                let n = stack.len();
                if n < 2 {
                    break 'dispatch Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode operand stack underflow"),
                    ));
                }
                let op = bin_ir(kind);
                let (a_ref, b_ref) = (&stack[n - 2], &stack[n - 1]);
                let fast = fast_smi_binop(a_ref, op, b_ref);
                if let Some(v) = fast {
                    stack.truncate(n - 2);
                    stack.push(v);
                    pc += 1;
                    continue;
                }
                let b = pop!();
                let a = pop!();
                let v = hop!(crate::interp::eval_binop(&a, op, &b));
                stack.push(v);
                pc += 1;
            }
            Instr::UnaryOp(kind) => {
                sync_frame_line(frame, cur_line);
                let x = pop!();
                let v = hop!(crate::interp::eval_unary_pub(interp, unary_ir(kind), x));
                stack.push(v);
                pc += 1;
            }
            Instr::Compare(op) => {
                sync_frame_line(frame, cur_line);
                let b = pop!();
                let a = pop!();
                let r = hop!(crate::interp::compare_values_pub(
                    interp,
                    &a,
                    cmp_ir(op),
                    &b
                ));
                stack.push(Value::Bool(r));
                pc += 1;
            }
            Instr::AugOp { kind, sym } => {
                // The AugAssign arm's VALUE op (leaf 01-core/01): the shared
                // aug_binop_value — in-place __iX__ protocol + list += alias
                // mutation — the SAME body the tree-walker's Stmt::AugAssign
                // arm routes through. `sym` (+=, //...) is error-message
                // fidelity only; the dispatch key is `kind`.
                sync_frame_line(frame, cur_line);
                let _ = sym;
                let n = stack.len();
                if n < 2 {
                    break 'dispatch Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode operand stack underflow"),
                    ));
                }
                let op = bin_ir(kind);
                // Smi fast path: a iop b on two raw smis rebinds (immutable
                // value, no aliasing concern) — identical to aug_binop_value.
                let fast = fast_smi_binop(&stack[n - 2], op, &stack[n - 1]);
                if let Some(v) = fast {
                    stack.truncate(n - 2);
                    stack.push(v);
                    pc += 1;
                    continue;
                }
                let b = pop!();
                let a = pop!();
                let v = hop!(crate::interp::aug_binop_value(interp, op, &a, &b));
                stack.push(v);
                pc += 1;
            }

            // ---- attribute / subscript (instrument ops) --------------------
            Instr::LoadAttr { idx, site } => {
                sync_frame_line(frame, cur_line);
                let obj = pop!();
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<attr>");
                // INLINE CACHE (leaf fp-inlinecache): user-class receiver
                // reads (the c.x / c.m loop shape). The entry stamps the
                // receiver's class payload identity + type_version +
                // chain_surgery_version + names-dict epoch; a warm hit is
                // four stamp compares + the plan's direct answer. Primitive
                // receivers ride fast_bound_mint below (their method tables
                // are value-family constants — no class counter to stamp).
                let mut ic_answered = false;
                if let Some(t) = receiver_class_ref(&obj) {
                    let stamps = crate::ic::class_stamps(&t);
                    let plan_hit = ic.with_attrs(|attrs| {
                        debug_assert!((site as usize) < attrs.len(), "ic attr site OOB");
                        let e = &attrs[site as usize];
                        if !e.stamps_valid(stamps.0, stamps.1, stamps.2, stamps.3) {
                            return None;
                        }
                        Some((e.plan, e.owner_ptr, e.v.clone()))
                    });
                    if let Some((plan, owner_ptr, cached_v)) = plan_hit {
                        let e = AttrHitView {
                            plan,
                            owner_ptr,
                            v: cached_v,
                        };
                        match e.plan {
                            p if p == AttrPlan::InstDict as u8 => {
                                // Instance-dict supplies the name. The VALUE
                                // re-probes (one lock probe, no walk): the
                                // entry caches the class-level RESOLUTION
                                // (name answered by the instance dict under
                                // these class stamps), not the value — an
                                // `o.x = new` rebind re-reads correctly even
                                // though no class counter moved.
                                let v = match &obj {
                                    Value::Obj(o) => o
                                        .attrs
                                        .lock()
                                        .unwrap_or_else(|p| p.into_inner())
                                        .get(&Value::Str(PyString::new(name))),
                                    _ => None,
                                };
                                if let Some(v) = v {
                                    IC_ATTR_HITS.fetch_add(1, Ordering::Relaxed);
                                    stack.push(v);
                                    pc += 1;
                                    ic_answered = true;
                                }
                                // None = the name vanished from the attrs
                                // dict without a class-stamp change (a del
                                // route that bypasses the stamps — instance
                                // dict pops don't bump class counters). The
                                // cached CLASS resolution is now wrong: fall
                                // through to the slow path, which answers
                                // AttributeError or the class member — and
                                // the miss below REFILLS with the fresh
                                // plan (overwriting the stale entry).
                            }
                            p if p == AttrPlan::BindFunc as u8 => {
                                // The MRO supplies a plain function (the
                                // cached answer). Mint the BoundMethod
                                // FRESH per read (CPython identity pin:
                                // c.m is c.m is False — never cache the
                                // mint), with the DEFINING class as the
                                // method owner (zero-arg super() start,
                                // S-03-18). The entry stores the function
                                // Value + the defining class's Arc ptr;
                                // the stamps guarantee the class surface
                                // is unchanged, and the owner re-derives
                                // from the LIVE MRO (one scan answering
                                // only the owner — the function itself
                                // comes from the entry).
                                // `c.m = lambda: ..` shadows this class
                                // method with NO class-stamp movement:
                                // re-probe the live instance dict before
                                // minting a bound method from the cached fn.
                                if !instance_dict_has(&obj, name) && e.owner_ptr != 0 {
                                    let f = e.v.clone();
                                    let owner = t
                                        .lookup_with_owner(name)
                                        .map(|(_, o)| o)
                                        .filter(|o| Arc::as_ptr(o) as usize as u64 == e.owner_ptr);
                                    if let Some(owner_t) = owner {
                                        let bm = ap_obj::PyObject::with_self(
                                            ap_obj::ObjKind::BoundMethod,
                                            name.to_string(),
                                            obj.clone(),
                                        );
                                        *bm.func.lock().unwrap_or_else(|p| p.into_inner()) =
                                            Some(f);
                                        *bm.method_owner
                                            .lock()
                                            .unwrap_or_else(|p| p.into_inner()) =
                                            Some(std::sync::Arc::downgrade(&owner_t));
                                        IC_ATTR_HITS.fetch_add(1, Ordering::Relaxed);
                                        stack.push(Value::Obj(bm));
                                        pc += 1;
                                        ic_answered = true;
                                    }
                                    // Owner ptr drifted (stamps passed, so
                                    // unexpected — belt-and-braces): fall
                                    // through to the slow walk.
                                }
                            }
                            p if p == AttrPlan::ClassValue as u8 => {
                                // Plain class-dict member read off a CLASS
                                // receiver (C.member): value stable under
                                // the stamps. On an INSTANCE receiver this
                                // plan means the instance dict did not
                                // shadow it at fill time — `c.x = 42` moves
                                // no class stamp, so re-probe the live dict
                                // and fall through to the walk if shadowed.
                                if !instance_dict_has(&obj, name) {
                                    let v = e.v.clone();
                                    IC_ATTR_HITS.fetch_add(1, Ordering::Relaxed);
                                    stack.push(v);
                                    pc += 1;
                                    ic_answered = true;
                                }
                            }
                            _ => {} // unknown plan byte: re-walk
                        }
                    }
                }
                if ic_answered {
                    continue;
                }
                // FAST MINT (leaf 01-core/01): the per-iteration
                // `out.append`/`d.get` bound-method read. attr_get's deep
                // walk costs ~1.7us per mint; this guard covers the exact
                // (receiver-family, family-gated name) pairs the walk's
                // fast_eligible arm answers with `PyObject::with_self` —
                // same mint, minus the walk. Non-matching shapes fall to
                // the full attr_get (which owns ALL the fall-through
                // semantics — nothing here changes an answer, only the
                // route).
                if let Some(v) = fast_bound_mint(&obj, name) {
                    stack.push(v);
                    pc += 1;
                    continue;
                }
                match crate::interp::attr_get_public(interp, &obj, name) {
                    Ok(Some(v)) => {
                        // FILL (miss arm): classify the answer the slow
                        // path produced and record the plan + stamps. Only
                        // sound-with-stamps shapes record; everything else
                        // (descriptors, properties, __getattr__ answers,
                        // dynamic surfaces) stays uncached — Slow = the
                        // slot stays empty and every read re-walks.
                        ic_attr_fill(&ic, site, &obj, name, &v);
                        stack.push(v);
                    }
                    Ok(None) => {
                        // The tree-walker's exact AttributeError shape
                        // (interp.rs Expr::Attribute arm, module suffix
                        // included).
                        let extra = match &obj {
                            Value::Obj(mo) if mo.kind == ap_obj::ObjKind::Module => {
                                format!(" (module '{}')", mo.name)
                            }
                            _ => String::new(),
                        };
                        raise_or_propagate!(raise_flow(
                            "AttributeError",
                            format!(
                                "'{}' object has no attribute '{}'{}",
                                crate::classes::display_type_name(&obj),
                                name,
                                extra
                            ),
                        ));
                    }
                    Err(f) => raise_or_propagate!(f),
                }
                pc += 1;
            }
            Instr::Subscript => {
                sync_frame_line(frame, cur_line);
                let k = pop!();
                let o = pop!();
                let v = hop!(crate::methods::subscript_get(&o, &k));
                stack.push(v);
                pc += 1;
            }
            Instr::StoreSubscript => {
                // Lowerer layout (lower_store_target, Subscript arm): Assign
                // pushed the VALUE first, then obj, then key — so TOS=key,
                // TOS-1=obj, TOS-2=value. Map to subscript_set(obj, key, value).
                let key = pop!();
                let obj = pop!();
                let value = pop!();
                hop!(crate::methods::subscript_set(&obj, &key, value));
                pc += 1;
            }
            Instr::StoreAttr { idx } => {
                // `obj.attr = v` (leaf 02-calls): Assign pushed the VALUE
                // first, then the obj expr — TOS=obj, TOS-1=value. The
                // tree-walker's exact route (classes::attr_set_protocol —
                // user __setattr__ / descriptor / cell semantics).
                sync_frame_line(frame, cur_line);
                let obj = pop!();
                let value = pop!();
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<attr>");
                hop!(crate::classes::attr_set_protocol(interp, &obj, name, value));
                pc += 1;
            }

            // ---- calls (correct-but-slow: through call_value, like the
            // tree-walker; the frame line is the CALL instrument's line for
            // the callee's whole execution — s016 contract) -----------------
            Instr::CallFunction { argc, .. } => {
                sync_frame_line(frame, cur_line);
                let argc = argc as usize;
                if stack.len() < argc + 1 {
                    break Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode call stack underflow"),
                    ));
                }
                let args = stack.split_off(stack.len() - argc);
                let callee = pop!();
                // (The leaf's BoundMethod fast arms were folded into the
                // fused LoadAttrCall opcode — `obj.method(x)` shapes never
                // mint a BoundMethod now. A BoundMethod VALUE reaching
                // CallFunction is the rare first-class route (hoisted
                // `ap = out.append`) and rides call_value unchanged.)
                let v = hop!(crate::interp::call_value_frame_pub(
                    interp,
                    frame,
                    &callee,
                    &args,
                    &[]
                ));
                stack.push(v);
                pc += 1;
            }

            Instr::CallFunctionKw {
                argc,
                kw_first,
                n_kw,
                ..
            } => {
                // Kwarg'd call (leaf 02-calls): the binding is call_value's
                // kwargs slice — the tree-walker's exact machinery
                // (defaults/kw-only/**kwargs binding all live there). Stack:
                // callee, argc positionals, n_kw kwarg VALUES (in
                // names[kw_first..] order), callee on top.
                sync_frame_line(frame, cur_line);
                let argc = argc as usize;
                let n_kw = n_kw as usize;
                if stack.len() < argc + n_kw + 1 {
                    break Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode call stack underflow"),
                    ));
                }
                // Pop the n_kw VALUES first (top-down = LAST kwarg first),
                // reverse to push order, THEN zip with names[kw_first..] —
                // pairing during the pops would hand the LAST value to the
                // FIRST name.
                let mut kvals: Vec<Value> = Vec::with_capacity(n_kw);
                for _ in 0..n_kw {
                    kvals.push(stack.pop().expect("kwarg value checked above"));
                }
                kvals.reverse();
                let mut kwargs: Vec<(String, Value)> = Vec::with_capacity(n_kw);
                for (i, v) in kvals.into_iter().enumerate() {
                    let name = code
                        .names
                        .get(kw_first as usize + i)
                        .cloned()
                        .unwrap_or_else(|| "<kw>".to_string());
                    kwargs.push((name, v));
                }
                let args = stack.split_off(stack.len() - argc);
                let callee = pop!();
                let v = hop!(crate::interp::call_value_frame_pub(
                    interp, frame, &callee, &args, &kwargs
                ));
                stack.push(v);
                pc += 1;
            }

            Instr::LoadAttrCall { idx, argc, site } => {
                // FUSED method call (leaf 01-core/01). Stack: receiver,
                // then args. Semantics = attr_get + call_value EXACTLY;
                // the fast arms below are pure routing shortcuts for the
                // shapes methods::call_method answers natively. Any other
                // shape mints the BoundMethod through attr_get_public and
                // calls it through call_value_frame_pub — the identical
                // two-opcode semantics, one opcode.
                sync_frame_line(frame, cur_line);
                let argc = argc as usize;
                if stack.len() < argc + 1 {
                    break Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode call stack underflow"),
                    ));
                }
                let args = stack.split_off(stack.len() - argc);
                let recv = pop!();
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<attr>");
                // INLINE CACHE (leaf fp-inlinecache): the (receiver tag,
                // name, argc) -> dispatcher decision memoizes here. The
                // key re-derives from the live stack receiver each hit
                // (value-family tags are constants — no mutation, no
                // version stamp needed); the entry saves only the
                // tag/classification work on the slow shapes.
                let recv_tag: Option<&'static str> = tag_of(&recv);
                let cached_plan = ic
                    .with_calls(|calls| {
                        debug_assert!((site as usize) < calls.len(), "ic call site OOB");
                        calls[site as usize].is_hit(argc as u16)
                    })
                    .map(|p| {
                        (
                            p == CallPlan::PrimValues as u8,
                            p == CallPlan::CallMethod as u8,
                        )
                    });
                let fast: Option<Result<Value, Flow>> = match (&recv, name) {
                    (Value::List(l), "append") if argc == 1 => {
                        l.append(args[0].clone());
                        Some(Ok(Value::None))
                    }
                    // PyDict::get returns Option<Value>; a missing key is
                    // Value::None — the same answer call_method's dict arm
                    // gives (dict.get never raises).
                    (Value::Dict(d), "get") if argc == 1 => {
                        Some(Ok(d.get(&args[0]).unwrap_or(Value::None)))
                    }
                    (Value::Set(s), "add") if argc == 1 => match s.add(args[0].clone()) {
                        Ok(()) => Some(Ok(Value::None)),
                        Err(e) => Some(Err(Flow::from_py(e))),
                    },
                    _ => None,
                };
                if let Some(r) = fast {
                    match r {
                        Ok(v) => {
                            stack.push(v);
                            pc += 1;
                            continue;
                        }
                        Err(f) => break Err((cur_line, f)),
                    }
                }
                // PRIMITIVE-RECEIVER DIRECT DISPATCH (leaf fp-calls): str/
                // bytes/int/float/tuple/list/dict/set receivers whose method
                // is in the native value surface ride call_builtin_values
                // DIRECTLY (receiver as args[0]) — skipping the attr walk +
                // fresh BoundMethod mint (~1.5us) the generic route pays per
                // call. The (tag, name) existence gate is the SAME table the
                // mint path consults, and call_builtin_values is the exact
                // dispatcher the minted dotted marker routes to — semantics
                // single-sourced, only the mint intermediate is bypassed.
                // Uncovered shapes (user receivers, dunders, module attrs)
                // fall to the mint route below unchanged.
                let via_call_method = matches!(
                    name,
                    "upper"
                        | "lower"
                        | "join"
                        | "format"
                        | "partition"
                        | "encode"
                        | "translate"
                        | "__len__"
                );
                // The cached decision short-circuits the tag-match +
                // table-exists probes; the dispatcher call itself is
                // identical (the cache never answers, only routes).
                let try_call_method = match cached_plan {
                    Some((_, cm)) => cm,
                    None => recv_tag.is_some() && via_call_method,
                };
                let try_values = match cached_plan {
                    Some((pv, _)) => pv,
                    None => match recv_tag {
                        Some(tag) => {
                            !via_call_method
                                && crate::builtins_modules::builtin_value_method_exists_pub(
                                    tag, name,
                                )
                        }
                        None => false,
                    },
                };
                if try_call_method {
                    let r = match crate::builtins_modules::current_interp() {
                        Some(i) => crate::methods::call_method(&i, &recv, name, &args),
                        None => None,
                    };
                    match r {
                        Some(Ok(v)) => {
                            if cached_plan.is_none() {
                                ic_call_fill(&ic, site, argc as u16, CallPlan::CallMethod);
                            }
                            stack.push(v);
                            pc += 1;
                            continue;
                        }
                        Some(Err(f)) => break Err((cur_line, f)),
                        // call_method answered None = name unknown on
                        // this receiver. NOT always an AttributeError:
                        // e.g. dict.set_int_max_str_digits-style marker
                        // arms return None for shape mismatches. Fall
                        // through to the generic mint route, whose
                        // attr_get owns the full AttributeError/None
                        // semantics.
                        None => {}
                    }
                } else if try_values {
                    // The call_builtin_values table (pystr.startswith,
                    // pybytes.split, pyint.bit_length, ...). Receiver
                    // first — call_builtin_values treats args[0] as recv.
                    let Some(i) = crate::builtins_modules::current_interp() else {
                        break Err((
                            cur_line,
                            raise_flow("RuntimeError", "no interpreter on this thread"),
                        ));
                    };
                    let _ = &i;
                    let mut a2: Vec<Value> = Vec::with_capacity(argc + 1);
                    a2.push(recv.clone());
                    a2.extend_from_slice(&args);
                    match crate::builtins_modules::call_builtin_values(
                        tag_of(&recv).unwrap_or(""),
                        name,
                        &a2,
                        &[],
                    ) {
                        Some(Ok(v)) => {
                            if cached_plan.is_none() {
                                ic_call_fill(&ic, site, argc as u16, CallPlan::PrimValues);
                            }
                            stack.push(v);
                            pc += 1;
                            continue;
                        }
                        Some(Err(f)) => break Err((cur_line, f)),
                        // Table claims the name but no arm answered
                        // (marker/subclass shapes) — fall to the generic
                        // route, identical semantics.
                        None => {}
                    }
                }
                // FALLBACK: mint through the walk + call (two-opcode shape).
                let callee = match crate::interp::attr_get_public(interp, &recv, name) {
                    Ok(Some(v)) => v,
                    Ok(None) => {
                        let extra = match &recv {
                            Value::Obj(mo) if mo.kind == ap_obj::ObjKind::Module => {
                                format!(" (module '{}')", mo.name)
                            }
                            _ => String::new(),
                        };
                        raise_or_propagate!(raise_flow(
                            "AttributeError",
                            format!(
                                "'{}' object has no attribute '{}'{}",
                                crate::classes::display_type_name(&recv),
                                name,
                                extra
                            ),
                        ));
                    }
                    Err(f) => raise_or_propagate!(f),
                };
                let v = hop!(crate::interp::call_value_frame_pub(
                    interp,
                    frame,
                    &callee,
                    &args,
                    &[]
                ));
                stack.push(v);
                pc += 1;
            }

            // ---- displays ----------------------------------------------------
            Instr::BuildTuple(count) => {
                let items = match take_many(&mut stack, count as usize) {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode build stack underflow"),
                        ));
                    }
                };
                stack.push(Value::Tuple(Gc::new(Tuple(items))));
                pc += 1;
            }
            Instr::BuildList(count) => {
                let items = match take_many(&mut stack, count as usize) {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode build stack underflow"),
                        ));
                    }
                };
                stack.push(Value::List(PyList::from_vec(items)));
                pc += 1;
            }
            Instr::BuildDict(count) => {
                let items = match take_many(&mut stack, (count as usize) * 2) {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode build stack underflow"),
                        ));
                    }
                };
                let dict = ap_obj::dict::PyDict::new();
                // Items are in push order: key, value, key, value, ...
                for pair in items.chunks(2) {
                    hop!(dict
                        .set(pair[0].clone(), pair[1].clone())
                        .map_err(Flow::from_py));
                }
                stack.push(Value::Dict(dict));
                pc += 1;
            }
            Instr::BuildSet(count) => {
                let items = match take_many(&mut stack, count as usize) {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode build stack underflow"),
                        ));
                    }
                };
                let set = ap_obj::dict::PySet::new();
                for it in items {
                    hop!(set.add(it).map_err(Flow::from_py));
                }
                stack.push(Value::Set(set));
                pc += 1;
            }

            // ---- control flow (jump targets are instruction indices;
            // the lowerer emits no jumps this leaf — defensive coverage for
            // 01-core) -------------------------------------------------------
            Instr::Jump(t) => {
                pc = t as usize;
            }
            Instr::JumpIfFalse(t) => {
                let v = pop!();
                let b = match fast_truthy(&v) {
                    Some(b) => b,
                    None => hop!(crate::interp::truthy_public(interp, &v)),
                };
                pc = if b { pc + 1 } else { t as usize };
            }
            Instr::JumpIfTrue(t) => {
                let v = pop!();
                let b = match fast_truthy(&v) {
                    Some(b) => b,
                    None => hop!(crate::interp::truthy_public(interp, &v)),
                };
                pc = if b { t as usize } else { pc + 1 };
            }
            Instr::JumpIfFalseOrPop(t) => {
                // CPython JUMP_IF_FALSE_OR_POP: falsy TOS is the RESULT —
                // jump KEEPING it; truthy TOS is discarded and the next
                // operand evaluates. (Value rule: `a and b` yields a when
                // a is falsy.)
                let v = match stack.last().cloned() {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode operand stack underflow"),
                        ));
                    }
                };
                let b = match fast_truthy(&v) {
                    Some(b) => b,
                    None => hop!(crate::interp::truthy_public(interp, &v)),
                };
                if b {
                    stack.pop();
                    pc += 1;
                } else {
                    pc = t as usize;
                }
            }
            Instr::JumpIfTrueOrPop(t) => {
                // CPython JUMP_IF_TRUE_OR_POP: truthy TOS is the RESULT —
                // jump KEEPING it; falsy TOS is discarded.
                let v = match stack.last().cloned() {
                    Some(v) => v,
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode operand stack underflow"),
                        ));
                    }
                };
                let b = match fast_truthy(&v) {
                    Some(b) => b,
                    None => hop!(crate::interp::truthy_public(interp, &v)),
                };
                if b {
                    pc = t as usize;
                } else {
                    stack.pop();
                    pc += 1;
                }
            }
            Instr::GetIter => {
                // iter(TOS) — register through the tree-walker's Stmt::For
                // entry (same registration surface, same iterator table).
                let o = pop!();
                let id = match crate::interp::register_iter_pub(interp, &o) {
                    Ok(id) => id,
                    Err(f) => break Err((cur_line, f)),
                };
                // Frame-exit prune list (G1 discipline: a break out of the
                // loop must not pin the registered iterator). Odd ids are
                // the generator-protocol space — never pruned here.
                if id % 2 == 0 {
                    frame_iter_ids.push(id);
                }
                // Park the handle for ForNext's cache (even ids only; the
                // registry lookup under the table lock happens once here).
                cached_it = if id % 2 == 0 {
                    interp
                        .iterators
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&id)
                        .map(|it| it.clone())
                } else {
                    None
                };
                let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
                *obj.gen_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
                stack.push(Value::Obj(obj));
                pc += 1;
            }
            Instr::ForNext(t) => {
                // CPython FOR_ITER shape: the iterator handle STAYS on the
                // stack (peek, not pop) — the next back-edge iteration
                // re-reads it; exhaustion pops it and jumps to the exit.
                //
                // FAST PATH (leaf 01-core/01): the overwhelmingly common
                // pull is an even-id table iterator whose state is one of
                // the simple cursors. That pull needs NO CallFn probe (the
                // CALLFN_ITER_EXISTS gate stays false for plain loops), NO
                // generators-table probe (even id), and NO prune scan —
                // inline the cursor arms and take the mutex once. Any other
                // state (generator, CallFn, dict, zip...) routes through
                // pull_iter_pub unchanged.
                let n = stack.len();
                if n == 0 {
                    break 'dispatch Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode operand stack underflow"),
                    ));
                }
                // Peek WITHOUT cloning the handle Value: gen_id is behind a
                // Mutex inside the Obj — read through the stack ref.
                let id = match &stack[n - 1] {
                    Value::Obj(o) => *o.gen_id.lock().unwrap_or_else(|p| p.into_inner()),
                    _ => None,
                };
                let Some(id) = id else {
                    break Err((
                        cur_line,
                        raise_flow("TypeError", "'NoneType' object is not an iterator"),
                    ));
                };
                // Inline even-id cursor pull; odd ids (generators) and any
                // non-cursor state go through the full pull_iter_pub.
                let mut fast: Option<Option<Value>> = None;
                if id % 2 == 0 {
                    // Handle-cache hit: GetIter parked the Arc<PyIterator>
                    // right after registering it. If the peeked handle is
                    // still THAT iterator (same Arc pointer), pull straight
                    // from the cached handle — no table lookup, no table
                    // Mutex (only the iterator's own state Mutex, which
                    // pull_iter takes anyway).
                    let cached_ok = match (&cached_it, &stack[n - 1]) {
                        (Some(it), Value::Obj(o)) => {
                            // Identity by table id: the wrapper PyObject is
                            // minted fresh per GetIter, but its gen_id IS
                            // the unique registry id we cached against.
                            *o.gen_id.lock().unwrap_or_else(|p| p.into_inner()) == it.table_id.get()
                                && it.table_id.get().is_some()
                        }
                        _ => false,
                    };
                    if cached_ok {
                        let it = cached_it.as_ref().unwrap();
                        let mut st = it.state.lock().unwrap_or_else(|p| p.into_inner());
                        match &mut *st {
                            IterState::Seq(items, cur) => {
                                fast = Some(if *cur < items.len() {
                                    let v = items[*cur].clone();
                                    *cur += 1;
                                    Some(v)
                                } else {
                                    None
                                });
                            }
                            IterState::SharedList(l, cur) => {
                                let v = l.get(*cur);
                                if v.is_some() {
                                    *cur += 1;
                                }
                                fast = Some(v);
                            }
                            IterState::SharedTuple(tup, cur) => {
                                fast = Some(if *cur < tup.0.len() {
                                    let v = tup.0[*cur].clone();
                                    *cur += 1;
                                    Some(v)
                                } else {
                                    None
                                });
                            }
                            IterState::Range {
                                cur, stop, step, ..
                            } => {
                                let cont = if *step >= 0 {
                                    *cur < *stop
                                } else {
                                    *cur > *stop
                                };
                                fast = Some(if cont {
                                    let v = *cur;
                                    *cur += *step;
                                    Some(Value::Int(Int::Smi(v)))
                                } else {
                                    None
                                });
                            }
                            IterState::DictKeys(it2) => fast = Some(it2.next()),
                            _ => {}
                        }
                    }
                    if fast.is_none() {
                        let its = interp.iterators.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(it) = its.get(&id) {
                            let mut st = it.state.lock().unwrap_or_else(|p| p.into_inner());
                            match &mut *st {
                                IterState::Seq(items, cur) => {
                                    fast = Some(if *cur < items.len() {
                                        let v = items[*cur].clone();
                                        *cur += 1;
                                        Some(v)
                                    } else {
                                        None
                                    });
                                }
                                IterState::SharedList(l, cur) => {
                                    let v = l.get(*cur);
                                    if v.is_some() {
                                        *cur += 1;
                                    }
                                    fast = Some(v);
                                }
                                IterState::SharedTuple(tup, cur) => {
                                    fast = Some(if *cur < tup.0.len() {
                                        let v = tup.0[*cur].clone();
                                        *cur += 1;
                                        Some(v)
                                    } else {
                                        None
                                    });
                                }
                                IterState::Range {
                                    cur, stop, step, ..
                                } => {
                                    let cont = if *step >= 0 {
                                        *cur < *stop
                                    } else {
                                        *cur > *stop
                                    };
                                    fast = Some(if cont {
                                        let v = *cur;
                                        *cur += *step;
                                        Some(Value::Int(Int::Smi(v)))
                                    } else {
                                        None
                                    });
                                }
                                IterState::DictKeys(dk) => fast = Some(dk.next()),
                                _ => {}
                            }
                        }
                    }
                }
                let pulled = match fast {
                    Some(v) => v,
                    None => hop!(crate::interp::pull_iter_pub(interp, id)),
                };
                match pulled {
                    Some(v) => {
                        stack.push(v);
                        pc += 1;
                    }
                    None => {
                        stack.pop(); // the exhausted iterator handle
                                     // Exhaustion prune (pull_iter's 02.04 discipline —
                                     // the fast path bypasses pull_iter, so it prunes
                                     // here): drop the finished iterator's table entry.
                        if id % 2 == 0 {
                            interp
                                .iterators
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&id);
                        }
                        pc = t as usize; // -> loop exit
                    }
                }
            }

            // ---- returns ------------------------------------------------------
            Instr::ReturnValue => {
                let v = pop!();
                break Ok(v);
            }
            Instr::ReturnNone => break Ok(Value::None),

            // ---- exceptions (leaf fp-exceptions) ------------------------------
            Instr::Raise { mode } => {
                // mode 0: raise TOS. mode 1: raise TOS0 from TOS (cause on
                // top). mode 2: bare raise (no operands — re-raise).
                sync_frame_line(frame, cur_line);
                let (exc_val, cause_val) = match mode {
                    2 => (None, None),
                    1 => {
                        let c = pop!();
                        let e = pop!();
                        (Some(e), Some(c))
                    }
                    _ => {
                        let e = pop!();
                        (Some(e), None)
                    }
                };
                let f = raise_op(interp, frame, mode, exc_val, cause_val)?;
                if let Flow::Raise(ref exc) = f {
                    if let Some(entry) = code.excs.find_handler(pc as u32) {
                        stack.truncate(entry.stack_depth as usize);
                        exc_enter(interp, frame, exc.clone(), tb_base);
                        pc = entry.handler as usize;
                        continue 'dispatch;
                    }
                }
                break Err((cur_line, f));
            }
            Instr::CheckExcept(target) => {
                // TOS = the handler's type expr. Parked exc -> match ->
                // instance push (fall through) | no match -> target arc.
                sync_frame_line(frame, cur_line);
                let ty = pop!();
                let bound = hop!(check_except(interp, &ty));
                if matches!(bound, Value::None) {
                    pc = target as usize;
                } else {
                    stack.push(bound);
                    pc += 1;
                }
            }
            Instr::CheckExceptBare => {
                stack.push(check_except_bare());
                pc += 1;
            }
            Instr::PopExc { unbind_slot } => {
                pop_exc(interp, frame, unbind_slot, &mut locals, code);
                pc += 1;
            }
            Instr::ResumeUnwind => {
                // Re-propagate the parked in-flight exception (the
                // lowerer's finally-unwind epilogue / no-match arc ends
                // here). Nested-catch consumed the park -> current.
                let parked = BC_PARKED_EXC.with(|c| c.borrow().as_ref().map(|(e, _, _)| e.clone()));
                let exc = match parked {
                    Some(e) => e,
                    None => {
                        let cur = interp
                            .current_exception
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .clone();
                        match cur {
                            Some(e) => (*e).clone(),
                            None => {
                                break Err((
                                    cur_line,
                                    raise_flow(
                                        "SystemError",
                                        "ResumeUnwind with no in-flight exception",
                                    ),
                                ))
                            }
                        }
                    }
                };
                sync_frame_line(frame, cur_line);
                if let Some(entry) = code.excs.find_handler(pc as u32) {
                    stack.truncate(entry.stack_depth as usize);
                    exc_enter(interp, frame, exc, tb_base);
                    pc = entry.handler as usize;
                    continue 'dispatch;
                }
                break Err((cur_line, Flow::Raise(exc)));
            }

            // ---- fallback marker (entry filters it; never dispatched) ----
            Instr::FallbackToTreeWalk => {
                break Err((
                    cur_line,
                    raise_flow(
                        "SystemError",
                        "bytecode dispatch reached FallbackToTreeWalk (entry routing bug)",
                    ),
                ));
            }
        }
    };

    // Frame exit: publish the final line to the TLS cell (see the loop
    // header comment — the per-instruction store was removed; observers
    // reading BC_LINE between frames see the last executed line).
    BC_LINE.with(|l| l.set(cur_line));
    // fp-exceptions: a park left outstanding at frame exit (the exception
    // propagated OUT of a bytecode handler chain) — the matching tb_frames
    // entries die with it (exec_try's unwind truncation discipline).
    let parked_at_exit = BC_PARKED_EXC.with(|c| c.borrow_mut().take());
    if let Some((_, _, tb_snapshot)) = parked_at_exit {
        let mut d = interp.tb_frames.lock().unwrap_or_else(|p| p.into_inner());
        let at = tb_snapshot.min(d.len());
        let _ = d.split_off(at);
    }
    // Frame exit: prune every iterator this frame registered (break /
    // return / unwind paths; exhaustion already pruned inside pull_iter).
    // Mirrors the tree-walker Stmt::For exit's table removal (G1).
    if !frame_iter_ids.is_empty() {
        let mut its = interp.iterators.lock().unwrap_or_else(|p| p.into_inner());
        for id in frame_iter_ids {
            its.remove(&id);
        }
    }

    match outcome {
        Ok(v) => {
            sync_frame_line(frame, cur_line);
            sync_slots_into_frame(frame, entry, code, &locals);
            Ok(Some(v))
        }
        Err((line, flow)) => {
            // Exception construction point: the frame exit (call_function's
            // Err arm) reads frame.last_line for THIS frame's traceback
            // entry — sync it from the failing instruction's line.
            sync_frame_line(frame, line);
            sync_slots_into_frame(frame, entry, code, &locals);
            Err(flow)
        }
    }
}

/// Pop `count` items preserving push order ([i0..iTOS] -> [i0..i_{n-1}]).
fn take_many(stack: &mut Vec<Value>, count: usize) -> Option<Vec<Value>> {
    if stack.len() < count {
        return None;
    }
    Some(stack.split_off(stack.len() - count))
}

/// Sync local slots back into the frame (call exit — success AND unwind),
/// mirroring jit_hook::store_slots_into_frame's unbound discipline: the
/// tree-walker never fabricates a binding for a never-assigned name, so an
/// unbound slot (None) with no existing binding is skipped (UnboundLocal
/// reads stay UnboundLocalError, not None). Bytecode bodies have no cells
/// and no declared globals (lowerer rejects those shapes), so plain
/// store_local is the exact store chain for every reachable slot.
fn sync_slots_into_frame(
    frame: &Frame,
    entry: &FnEntry,
    code: &CodeObject,
    locals: &[Option<Value>],
) {
    let _ = entry;
    for (i, name) in code.local_names.iter().enumerate() {
        if i >= locals.len() {
            break;
        }
        let Some(v) = &locals[i] else {
            continue; // never assigned: no binding to fabricate
        };
        let v = v.clone();
        if matches!(v, Value::None) && !frame.has_binding(name) {
            continue;
        }
        frame.store_local(name, v);
    }
}

// ---- IR tag mapping (Copy mirrors; total maps, no semantics here) --------

fn bin_ir(k: BinKind) -> BinOp {
    match k {
        BinKind::Add => BinOp::Add,
        BinKind::Sub => BinOp::Sub,
        BinKind::Mul => BinOp::Mul,
        BinKind::MatMul => BinOp::MatMul,
        BinKind::Div => BinOp::Div,
        BinKind::FloorDiv => BinOp::FloorDiv,
        BinKind::Mod => BinOp::Mod,
        BinKind::Pow => BinOp::Pow,
        BinKind::LShift => BinOp::LShift,
        BinKind::RShift => BinOp::RShift,
        BinKind::BitOr => BinOp::BitOr,
        BinKind::BitXor => BinOp::BitXor,
        BinKind::BitAnd => BinOp::BitAnd,
    }
}

fn unary_ir(k: UnaryKind) -> UnaryOp {
    match k {
        UnaryKind::Plus => UnaryOp::Plus,
        UnaryKind::Minus => UnaryOp::Minus,
        UnaryKind::Invert => UnaryOp::Invert,
        UnaryKind::Not => UnaryOp::Not,
    }
}

fn cmp_ir(k: CompareOp) -> CmpOp {
    match k {
        CompareOp::Eq => CmpOp::Eq,
        CompareOp::NotEq => CmpOp::NotEq,
        CompareOp::Lt => CmpOp::Lt,
        CompareOp::LtE => CmpOp::LtE,
        CompareOp::Gt => CmpOp::Gt,
        CompareOp::GtE => CmpOp::GtE,
        CompareOp::Is => CmpOp::Is,
        CompareOp::IsNot => CmpOp::IsNot,
        CompareOp::In => CmpOp::In,
        CompareOp::NotIn => CmpOp::NotIn,
    }
}

/// Decode a const-pool entry (same shapes the walker's literal arms build).
fn const_value(code: &CodeObject, idx: u32) -> Result<Value, Flow> {
    match code.consts.get(idx) {
        Const::Int(i) => Ok(Value::Int(Int::from_i64(*i))),
        Const::BigInt(digits) => match Int::parse_base10(digits) {
            Ok(i) => Ok(Value::Int(i)),
            Err(()) => Err(raise_flow("SystemError", "bad bigint literal")),
        },
        Const::Float(f) => Ok(Value::Float(*f)),
        Const::Str(s) => Ok(Value::Str(PyString::new(s.clone()))),
        Const::Bytes(b) => Ok(Value::Bytes(Gc::new(b.clone()))),
    }
}

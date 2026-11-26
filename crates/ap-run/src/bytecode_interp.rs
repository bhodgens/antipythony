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
use std::sync::Arc;

use ap_bytecode::code::CodeObject;
use ap_bytecode::instr::{BinKind, CompareOp, Instr, UnaryKind};
use ap_bytecode::pool::Const;
use ap_ir::expr::{BinOp, CmpOp, UnaryOp};
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

use crate::exc_hier::ExceptionValue;
use crate::frame::{FnEntry, Frame, Interpreter, Loc};
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
}

/// Exercise counters (APX_TIER_STATS=1 -> bc_stats_line). Transparency
/// discipline (02.05a-residual): a toggle that silently tree-walks
/// everything would pass every output gate — these counters make the route
/// observable, and fix_bytecode.py asserts execs > 0 on the bytecode leg.
static BC_COMPILES: AtomicU64 = AtomicU64::new(0);
static BC_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static BC_EXECS: AtomicU64 = AtomicU64::new(0);

/// One-line stats report (called from jit_hook::report_line under
/// APX_TIER_STATS=1). Empty when nothing was ever compiled (toggle-off runs
/// print nothing — zero output-channel footprint by default).
pub fn bc_stats_line() -> Option<String> {
    let compiles = BC_COMPILES.load(Ordering::Relaxed);
    let fallbacks = BC_FALLBACKS.load(Ordering::Relaxed);
    let execs = BC_EXECS.load(Ordering::Relaxed);
    if compiles == 0 && fallbacks == 0 && execs == 0 {
        return None;
    }
    Some(format!(
        "[BC-STATS] compiles={compiles} fallbacks={fallbacks} execs={execs}"
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

    let mut stack: Vec<Value> = Vec::with_capacity(8);
    let mut pc: usize = 0;
    let mut cur_line: u32 = c.dense_lines.first().copied().unwrap_or(1);
    // Registered even-id iterators this frame created (GetIter) — pruned at
    // frame exit (break/unwind AND exhaustion both drop their entries inside
    // pull_iter; this covers the BREAK/mid-stream exits, the G1 discipline).
    let mut frame_iter_ids: Vec<u64> = Vec::new();

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
                    Err(f) => break 'dispatch Err((cur_line, f)),
                }
            };
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
        // THE leaf's core bookkeeping change: one TLS store per instruction.
        cur_line = c.dense_lines[pc];
        BC_LINE.with(|l| l.set(cur_line));

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
                if s >= locals.len() {
                    break Err((
                        cur_line,
                        raise_flow("SystemError", "bytecode local slot out of range"),
                    ));
                }
                match &locals[s] {
                    None => {
                        // Unbound local: the tree-walker's exact error shape.
                        // (Some(Value::None) is a real `x = None` store and
                        // reads fine — the Option IS the unbound marker.)
                        let name = code
                            .local_names
                            .get(s)
                            .map(|n| n.as_str())
                            .unwrap_or("<slot>");
                        break Err((
                            cur_line,
                            raise_flow(
                                "UnboundLocalError",
                                format!(
                                    "cannot access local variable '{name}' where it is not \
                                     associated with a value"
                                ),
                            ),
                        ));
                    }
                    Some(v) => stack.push(v.clone()),
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
                let v = pop!();
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
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<name>");
                let v = hop!(crate::interp::lookup_name_pub(interp, frame, name));
                stack.push(v);
                pc += 1;
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
                let b = pop!();
                let a = pop!();
                let v = hop!(crate::interp::eval_binop(&a, bin_ir(kind), &b));
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
                let b = pop!();
                let a = pop!();
                let v = hop!(crate::interp::aug_binop_value(
                    interp,
                    bin_ir(kind),
                    &a,
                    &b
                ));
                stack.push(v);
                pc += 1;
            }

            // ---- attribute / subscript (instrument ops) --------------------
            Instr::LoadAttr { idx, .. } => {
                sync_frame_line(frame, cur_line);
                let obj = pop!();
                let name = code
                    .names
                    .get(idx as usize)
                    .map(|n| n.as_str())
                    .unwrap_or("<attr>");
                match crate::interp::attr_get_public(interp, &obj, name) {
                    Ok(Some(v)) => stack.push(v),
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
                        break Err((
                            cur_line,
                            raise_flow(
                                "AttributeError",
                                format!(
                                    "'{}' object has no attribute '{}'{}",
                                    crate::classes::display_type_name(&obj),
                                    name,
                                    extra
                                ),
                            ),
                        ));
                    }
                    Err(f) => break Err((cur_line, f)),
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
                let b = hop!(crate::interp::truthy_public(interp, &v));
                pc = if b { pc + 1 } else { t as usize };
            }
            Instr::JumpIfTrue(t) => {
                let v = pop!();
                let b = hop!(crate::interp::truthy_public(interp, &v));
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
                let b = hop!(crate::interp::truthy_public(interp, &v));
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
                let b = hop!(crate::interp::truthy_public(interp, &v));
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
                let obj = ap_obj::PyObject::new(ap_obj::ObjKind::Iterator, "iterator");
                *obj.gen_id.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
                stack.push(Value::Obj(obj));
                pc += 1;
            }
            Instr::ForNext(t) => {
                // CPython FOR_ITER shape: the iterator handle STAYS on the
                // stack (peek, not pop) — the next back-edge iteration
                // re-reads it; exhaustion pops it and jumps to the exit.
                let h = match stack.last() {
                    Some(h) => h.clone(),
                    None => {
                        break 'dispatch Err((
                            cur_line,
                            raise_flow("SystemError", "bytecode operand stack underflow"),
                        ))
                    }
                };
                let id = match &h {
                    Value::Obj(o) => *o.gen_id.lock().unwrap_or_else(|p| p.into_inner()),
                    _ => None,
                };
                let Some(id) = id else {
                    break Err((
                        cur_line,
                        raise_flow("TypeError", "'NoneType' object is not an iterator"),
                    ));
                };
                match hop!(crate::interp::pull_iter_pub(interp, id)) {
                    Some(v) => {
                        stack.push(v);
                        pc += 1;
                    }
                    None => {
                        stack.pop(); // the exhausted iterator handle
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

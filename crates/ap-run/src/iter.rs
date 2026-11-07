//! Iterator protocol for the v0 executor (task brief §2: for-loop iterators
//! over range, str, list, tuple, dict keys, set; enumerate/zip/generator
//! resumption; StopIteration plumbing S-02-17).
//!
//! Each iterator carries a SNAPSHOT of what it walks (values cloned as
//! handles — mutation-during-iteration is UB in CPython too) plus a cursor.
//! The executor holds iterators in `Interpreter::iterators` keyed by u64 id;
//! `Value::Obj(ObjKind::Iterator)` is the opaque handle.

use std::cell::RefCell;

use crate::exc_hier::ExceptionValue;
use crate::Flow;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

/// Iterator state. Every variant's `next` yields Some(value) or ends.
pub enum IterState {
    /// Index cursor into a SNAPSHOT — EXCEPT `Shared`: list/tuple sources
    /// carry the container's own Rc (G1: a for-loop over a 100MB list used
    /// to clone the whole container per iter(); now the iterator shares the
    /// backing store and clones only the yielded VALUE per step).
    Seq(Vec<Value>, usize),
    /// G1: index cursor sharing the ORIGINAL container's backing Rc. The
    /// RefCell borrow per step mirrors PyList aliasing semantics (append
    /// during iteration is visible — same UB-in-CPython caveat as snapshots,
    /// but no full-container clone per iter()).
    SharedList(Gc<PyList>, usize),
    /// G1: tuple variant (immutable; the Rc shares the backing Vec).
    SharedTuple(Gc<Tuple>, usize),
    /// range: integer cursor with start/stop/step.
    Range {
        cur: i64,
        stop: i64,
        step: i64,
        big: Option<(Int, Int, Int)>,
    },
    /// Dict key iteration (insertion order).
    DictKeys(std::vec::IntoIter<Value>),
    /// A suspended generator's id (executor resumes its frame).
    Generator(u64),
    /// enumerate wrapper over an inner iterator id.
    Enumerate { inner: u64, counter: i64 },
    /// zip over several inner iterators.
    Zip(Vec<u64>),
    /// 02.08 stdlib iterators (itertools.count/cycle/repeat/..., enumerate/
    /// zip re-landing, str/keys views). State is self-contained; `next`
    /// delegates to builtins_modules (own file, no interp access needed).
    Native(Box<crate::builtins_modules::NativeIterState>),
}

/// A materialized iterator.
pub struct PyIterator {
    pub state: std::cell::RefCell<IterState>,
    /// G1/F5 (oom-bughunt 2026-11-06): the registered table id this
    /// iterator was inserted under (None for unregistered/anon handles).
    /// On Drop, the id lands in DEAD_ITER_IDS for the next registry prune —
    /// so an iterator nobody pulls to exhaustion (break-path escapes beyond
    /// Stmt::For, abandoned next() chains) releases its table slot AND the
    /// values its state pins (the old full-container Seq snapshot) instead
    /// of pinning them for process life.
    pub table_id: std::cell::Cell<Option<u64>>,
}

thread_local! {
    /// Ids of dropped PyIterators whose registry entries must go. Drained
    /// by `take_dead_iter_ids` (called from the interp pull paths).
    static DEAD_ITER_IDS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

impl Drop for PyIterator {
    fn drop(&mut self) {
        if let Some(id) = self.table_id.get() {
            // try_with: iterators living in the interpreter's own teardown
            // (registry maps dropping at process exit) fire this hook AFTER
            // TLS destruction began — a plain .with panics AccessError and
            // aborts (observed: classes_differential iter_in_for, rc=133
            // AFTER correct stdout). A dead TLS means the registry prune
            // list is going away too — the entry dies with the process.
            let _ = DEAD_ITER_IDS.try_with(|d| d.borrow_mut().push(id));
        }
    }
}

/// F5: take the ids of PyIterators dropped since the last call. The caller
/// (interp pull paths) removes their registry entries; an abandoned iterator
/// then releases its pinned state even when nothing iterates it to
/// exhaustion. Note: the id recorded at Drop is only VALID if the table
/// entry still points at THAT object — the interp prune double-checks
/// pointer equality via this method's pairing (id, was-table_id), so a
/// recycled id never kills a live entry.
pub fn take_dead_iter_ids() -> Vec<u64> {
    DEAD_ITER_IDS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// Fence-perf fix 2026-11-06: queue an exhausted iterator's table id for the
/// next prune without touching the registry inline (next() can run while the
/// caller holds an interp.iterators borrow — inline removal panics).
pub fn queue_dead_iter_id(id: u64) {
    DEAD_ITER_IDS.with(|d| d.borrow_mut().push(id));
}

impl PyIterator {
    pub fn new(state: IterState) -> Gc<PyIterator> {
        Gc::new(PyIterator {
            state: std::cell::RefCell::new(state),
            table_id: std::cell::Cell::new(None),
        })
    }

    /// Build an iterator over any iterable Value. Returns Err for
    /// non-iterables with the exact TypeError text.
    pub fn of(v: &Value) -> Result<Gc<PyIterator>, ExceptionValue> {
        match v {
            // G1: share the container's backing store instead of cloning it
            // (a for-loop over a 100MB list was a 100MB copy per iter()).
            Value::List(l) => Ok(PyIterator::new(IterState::SharedList(l.clone(), 0))),
            Value::Tuple(t) => Ok(PyIterator::new(IterState::SharedTuple(t.clone(), 0))),
            Value::Str(s) => Ok(PyIterator::new(IterState::Seq(
                s.as_str()
                    .chars()
                    .map(|c| Value::Str(PyString::new(c.to_string())))
                    .collect(),
                0,
            ))),
            Value::Bytes(b) => Ok(PyIterator::new(IterState::Seq(
                (**b)
                    .iter()
                    .map(|&x| Value::Int(Int::Smi(x as i64)))
                    .collect(),
                0,
            ))),
            Value::Dict(d) => {
                let keys = d.keys();
                Ok(PyIterator::new(IterState::DictKeys(keys.into_iter())))
            }
            Value::Set(s) | Value::Frozen(s) => {
                Ok(PyIterator::new(IterState::Seq(s.iter().collect(), 0)))
            }
            Value::Obj(o) => match o.kind {
                ap_obj::ObjKind::Range => {
                    let ints = o.ints.borrow();
                    let (start, stop, step) = (ints[0], ints[1], ints[2]);
                    Ok(PyIterator::new(IterState::Range {
                        cur: start,
                        stop,
                        step,
                        big: None,
                    }))
                }
                ap_obj::ObjKind::Iterator => {
                    // Iterating an iterator returns itself.
                    let existing = o.gen_id.borrow().map(|_| ());
                    let _ = existing;
                    // Reconstruct: iterators store their own id in gen_id.
                    Err(ExceptionValue::msg(
                        "TypeError",
                        "iter() re-entry on iterators is not supported in v0",
                    ))
                }
                _ => Err(ExceptionValue::msg(
                    "TypeError",
                    format!("'{}' object is not iterable", v.type_name()),
                )),
            },
            _ => Err(ExceptionValue::msg(
                "TypeError",
                format!("'{}' object is not iterable", v.type_name()),
            )),
        }
    }

    /// Build a range iterator (all three bounds as i64; v0 corpus never
    /// overflows i64 range bounds).
    pub fn of_range(start: i64, stop: i64, step: i64) -> Gc<PyIterator> {
        PyIterator::new(IterState::Range {
            cur: start,
            stop,
            step,
            big: None,
        })
    }

    /// Pull the next value: None = exhausted.
    pub fn next(&self) -> Option<Value> {
        let mut st = self.state.borrow_mut();
        let out = match &mut *st {
            IterState::Seq(items, cur) => {
                if *cur < items.len() {
                    let v = items[*cur].clone();
                    *cur += 1;
                    Some(v)
                } else {
                    None
                }
            }
            // G1: borrow the shared container per step (never clone it).
            // The borrow is dropped before returning — a re-entrant
            // mutation from a generator/worker step cannot double-borrow.
            IterState::SharedList(l, cur) => {
                let items = l.items();
                if *cur < items.len() {
                    let v = items[*cur].clone();
                    drop(items);
                    *cur += 1;
                    Some(v)
                } else {
                    None
                }
            }
            IterState::SharedTuple(t, cur) => {
                if *cur < t.0.len() {
                    let v = t.0[*cur].clone();
                    *cur += 1;
                    Some(v)
                } else {
                    None
                }
            }
            IterState::Range {
                cur, stop, step, ..
            } => {
                let cont = if *step >= 0 {
                    *cur < *stop
                } else {
                    *cur > *stop
                };
                if cont {
                    let v = *cur;
                    *cur += *step;
                    Some(Value::Int(Int::Smi(v)))
                } else {
                    None
                }
            }
            IterState::DictKeys(it) => it.next(),
            IterState::Generator(id) => {
                // T2 root fix: resume through the interpreter. Collectors
                // (list.extend, tuple, set, sorted, dict.update, snapshot)
                // all pull via this method; returning None silently dropped
                // every generator-driven drain (pytest collected 0 items).
                match crate::builtins_modules::current_interp() {
                    Some(ic) => match crate::interp::gen_pull_public(&ic, *id) {
                        Ok(Some(v)) => Some(v),
                        Ok(None) => None,
                        Err(f) => {
                            // W112: do NOT swallow body errors as
                            // exhaustion - log the class so the failing
                            // attr read is visible; the error surfaces
                            // via pending staging on the next explicit
                            // pull of this generator.
                            if let crate::Flow::Raise(e) = &f {
                                if ap_obj::debug_flags::debug_flag(
                                    ap_obj::debug_flags::ApxFlag::GenDebug,
                                ) {
                                    let msg = e
                                        .args
                                        .first()
                                        .map(|v| {
                                            crate::builtins::py_str_of(v)
                                                .map(|s| s.as_str().to_string())
                                                .unwrap_or_default()
                                        })
                                        .unwrap_or_default();
                                    eprintln!(
                                        "[ITER-GEN-ERR] id={} class={} msg={}",
                                        id, e.class, msg
                                    );
                                }
                            }
                            crate::interp::set_pending_generator_error_for(*id, f);
                            None
                        }
                    },
                    None => None,
                }
            }
            IterState::Enumerate { .. } | IterState::Zip(_) => {
                // These resume through the interpreter (they need frame
                // access); handled in interp.rs, never via this method.
                None
            }
            IterState::Native(n) => n.next(),
        };
        if out.is_none() {
            // Fence-perf fix 2026-11-06: exhausted iterator — detach own
            // table entry NOW. The table holds a STRONG handle, so the F5
            // Drop hook can never fire for table-inserted iterators; a
            // native drain (list()/collect()/extend — no pull_iter on the
            // path) left the entry pinned forever (~400B/call on the
            // ast.walk eager-sink path: 1.6GB at the fence's 1000
            // iterations). Removing at exhaustion is safe: a later pull on
            // an exhausted iterator is None either way; pulls for def-gens
            // route through gen_pull (generator registry), not this table.
            if let Some(id) = self.table_id.get() {
                // Defer via DEAD_ITER_IDS (inline removal here can race the
                // caller's interp.iterators borrow). The next iterable_of /
                // pull_iter prune removes the entry.
                crate::iter::queue_dead_iter_id(id);
            }
        }
        out
    }
}

/// Create a list from any iterable (list(iterable) builtin path).
pub fn to_list(it: &Gc<PyIterator>) -> Result<Gc<PyList>, Flow> {
    let mut out = Vec::new();
    while let Some(v) = it.next() {
        out.push(v);
    }
    Ok(PyList::from_vec(out))
}

/// Length of a sized iterable for len(range(n)) etc.
pub fn range_len(start: i64, stop: i64, step: i64) -> i64 {
    if step == 0 {
        return 0;
    }
    if step > 0 {
        ((stop - start).max(0) + step - 1) / step
    } else {
        ((start - stop).max(0) + (-step) - 1) / (-step)
    }
}

/// tuple(...) / sorted(...) collectors.
pub fn collect(it: &Gc<PyIterator>) -> Vec<Value> {
    let mut out = Vec::new();
    while let Some(v) = it.next() {
        out.push(v);
    }
    out
}

/// StopIteration singleton error (bare, no args).
pub fn stop_iteration() -> Flow {
    Flow::Raise(ExceptionValue::new("StopIteration", vec![]))
}

/// Unused-warning shield for Tuple import in future extensions.
#[allow(dead_code)]
fn _t(_: Gc<Tuple>) {}

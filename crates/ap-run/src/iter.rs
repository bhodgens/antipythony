//! Iterator protocol for the v0 executor (task brief §2: for-loop iterators
//! over range, str, list, tuple, dict keys, set; enumerate/zip/generator
//! resumption; StopIteration plumbing S-02-17).
//!
//! Each iterator carries a SNAPSHOT of what it walks (values cloned as
//! handles — mutation-during-iteration is UB in CPython too) plus a cursor.
//! The executor holds iterators in `Interpreter::iterators` keyed by u64 id;
//! `Value::Obj(ObjKind::Iterator)` is the opaque handle.

use crate::exc_hier::ExceptionValue;
use crate::Flow;
use ap_obj::int::{Gc, Int};
use ap_obj::value::{PyList, PyString, Tuple, Value};

/// Iterator state. Every variant's `next` yields Some(value) or ends.
pub enum IterState {
    /// Index cursor into a snapshot.
    Seq(Vec<Value>, usize),
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
}

impl PyIterator {
    pub fn new(state: IterState) -> Gc<PyIterator> {
        Gc::new(PyIterator {
            state: std::cell::RefCell::new(state),
        })
    }

    /// Build an iterator over any iterable Value. Returns Err for
    /// non-iterables with the exact TypeError text.
    pub fn of(v: &Value) -> Result<Gc<PyIterator>, ExceptionValue> {
        match v {
            Value::List(l) => Ok(PyIterator::new(IterState::Seq(l.items().to_vec(), 0))),
            Value::Tuple(t) => Ok(PyIterator::new(IterState::Seq(t.0.clone(), 0))),
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
        match &mut *st {
            IterState::Seq(items, cur) => {
                if *cur < items.len() {
                    let v = items[*cur].clone();
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
                                if std::env::var("APX_GEN_DEBUG").is_ok() {
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
                            crate::interp::set_pending_generator_error_for(
                                *id, f,
                            );
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
        }
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

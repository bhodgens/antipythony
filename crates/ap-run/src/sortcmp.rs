//! Sort comparators — audit Fix B (pp2-sort leaf).
//!
//! The pre-fix sort paths paid the FULL compare protocol per comparison:
//! `py_lt` → `user_lt_public` → `user_compare` → 4x `special_lookup`
//! (a_slot, b_slot, b_rslot, a_rslot — each an MRO walk under a Mutex)
//! plus a `value_is_proper_subclass_of` pair, on EVERY `<` of a sort.
//! CPython resolves the comparison ONCE per sort (listobject.c resolves
//! `LT` at listsort entry; the per-compare cost is a function pointer).
//!
//! This module hoists that resolution: per sort call we classify the
//! elements once and pick a comparator:
//!
//! * [`SortCmp::RawInt`] / [`SortCmp::RawStr`] — the existing raw
//!   i64 / str comparators from the methods.rs homogeneous v0 arm
//!   (measured 5.2x CPython on that path), now shared by sorted()
//!   keyless arms.
//! * [`SortCmp::RawFloat`] — the CPython 3.10+ float TOTAL ORDER
//!   (listobject.c's `float_total_ordering` / `unsafe_float_compare`
//!   adaptive selection), which also fixes the 6 NaN gate rows
//!   (values_cmp errored NaN compares; CPython sorts them).
//! * [`SortCmp::MixedNumeric`] — int/bool/float mixes via
//!   `cmp_int_f64`-exact numeric comparison (mirrors values_cmp's
//!   numeric arms without its TypeError arm).
//! * [`SortCmp::Generic`] — the per-compare dispatch fallback. For
//!   user-instance lists it is reached only AFTER a once-per-sort
//!   probe: single-class, reflected-free `__lt__` lists are detected
//!   in O(1)-per-sort and the hoisted flag is recorded, so the
//!   generic closure consults the resolved slot instead of re-walking
//!   the MRO 4x per compare. Mixed/diverse shapes keep the exact
//!   pre-Fix-B per-compare semantics (incl. classification-dependent
//!   TypeError texts).
//!
//! Escalation discipline (ppfix-sort contract, unchanged): any error
//! from any comparator is parked in the caller's error cell and
//! re-raised after `sort_by` returns — `sort_by` cannot short-circuit.

use crate::interp;
use crate::Flow;
use ap_obj::{ObjKind, PyString, Value};

/// The homogeneity class of a sort's element list (one O(n) pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Homog {
    /// Every element `Value::Int`/`Value::Bool` (bool ⊂ int).
    AllInt,
    /// Every element `Value::Str`.
    AllStr,
    /// Every element `Value::Float`.
    AllFloat,
    /// Int/Bool/Float in any mix (still orderable numerically).
    MixedNumeric,
    /// Anything else — generic per-compare dispatch.
    Other,
}

/// One O(n) pass: classify the element slice for comparator selection.
/// CPython's `unsafe_long_compare`/`unsafe_object_compare` adaptive
/// pre-scan equivalent: homogeneity decides whether a raw arm is sound.
pub fn classify(items: &[Value]) -> Homog {
    let mut saw_int = false;
    let mut saw_float = false;
    let mut saw_str = false;
    for v in items {
        match v {
            Value::Int(_) | Value::Bool(_) => saw_int = true,
            Value::Float(_) => saw_float = true,
            Value::Str(_) => saw_str = true,
            _ => return Homog::Other,
        }
    }
    match (saw_int, saw_float, saw_str) {
        (true, false, false) => Homog::AllInt,
        (false, false, true) => Homog::AllStr,
        (false, true, false) => Homog::AllFloat,
        // Numeric mix ONLY when no str is present — int/str (or
        // float/str) mixes are NOT orderable and must fall to Other
        // (Generic → py_lt → the exact CPython TypeError; routing them
        // here silently ordered them Equal, ppfix-sort bug 1 regress).
        (true, true, false) => Homog::MixedNumeric,
        _ => Homog::Other,
    }
}

// ---------------------------------------------------------------------------
// Raw comparators (shared with the methods.rs v0 homogeneous arm)
// ---------------------------------------------------------------------------

/// Raw int comparison, exact across Smi/Big arms (`cmp_int`).
/// `as_int()` maps Bool → Smi(0/1) (bool ⊂ int, S-01-9) and is total on
/// the AllInt/MixedNumeric classifications.
#[inline]
pub fn raw_int_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.as_int(), b.as_int()) {
        (Some(x), Some(y)) => x.cmp_int(&y),
        _ => std::cmp::Ordering::Equal, // unreachable on classified input
    }
}

/// Raw str comparison — code-point order (V016: WTF-8 surrogate bytes
/// (ED..) must not sort above astral UTF-8 (F0..); the surrogate path
/// compares code points explicitly).
#[inline]
pub fn raw_str_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => {
            if x.has_surrogates() || y.has_surrogates() {
                x.code_points().cmp(&y.code_points())
            } else {
                x.as_str().cmp(y.as_str())
            }
        }
        _ => std::cmp::Ordering::Equal, // unreachable on classified input
    }
}

/// CPython 3.10+ float TOTAL ORDER: -inf < … < -0.0 == 0.0 < … < inf <
/// NaN (NaN == NaN for ordering — sort stability then preserves input
/// order among NaNs, matching CPython's `float_total_ordering`). The
/// decorate-reverse scheme applies reverse OUTSIDE the comparator, so
/// this total order alone gives CPython's exact NaN placement under
/// reverse=True too.
#[inline]
pub fn float_total_order(x: f64, y: f64) -> std::cmp::Ordering {
    match x.partial_cmp(&y) {
        Some(ord) => ord,
        None => {
            // x is NaN (y cannot be NaN when partial_cmp is None
            // without x being NaN; both-NaN also lands here).
            if y.is_nan() {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Greater
            }
        }
    }
}

/// Raw float comparison on the AllFloat classification.
#[inline]
pub fn raw_float_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => float_total_order(*x, *y),
        _ => std::cmp::Ordering::Equal, // unreachable on classified input
    }
}

/// Mixed int/bool/float numeric comparison — mirrors values_cmp's
/// numeric arms (int<->float via the EXACT `cmp_int_f64`, no double
/// rounding: `2**53 + 1 == 9007199254740992.0` stays false) WITHOUT
/// its non-numeric TypeError arm (classification proved every element
/// numeric). NaN total order included (gate rows E.nan.mixed_num*).
#[inline]
pub fn mixed_numeric_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => float_total_order(*x, *y),
        (Value::Int(_) | Value::Bool(_), Value::Int(_) | Value::Bool(_)) => raw_int_cmp(a, b),
        (Value::Int(_) | Value::Bool(_), Value::Float(f)) => {
            let ord = ap_obj::dict::cmp_int_f64(&a.as_int().unwrap(), *f);
            // cmp_int_f64 already ranks NaN Greater (total order).
            ord
        }
        (Value::Float(f), Value::Int(_) | Value::Bool(_)) => {
            ap_obj::dict::cmp_int_f64(&b.as_int().unwrap(), *f).reverse()
        }
        _ => std::cmp::Ordering::Equal, // unreachable on classified input
    }
}

// ---------------------------------------------------------------------------
// The comparator enum + per-compare entry
// ---------------------------------------------------------------------------

/// The comparator resolved for one sort call (Fix B: resolve once, not
/// 4x-MRO per compare).
pub enum SortCmp {
    RawInt,
    RawStr,
    RawFloat,
    MixedNumeric,
    /// Per-compare full dispatch (exact pre-Fix-B semantics). When
    /// `hoisted` carries a resolved `__lt__` slot (single-class
    /// reflected-free user list), the generic closure consults it
    /// FIRST (user_compare's a_slot-first, no-reflected shape) and
    /// only falls back to `py_lt` on that slot's error — the MRO walk
    /// happens once per sort, not 4x per compare.
    Generic { hoisted_lt: Option<Value> },
}

/// One comparison outcome. Errors ride out for the parked-cell
/// escalation (ppfix-sort Err-propagation contract).
pub enum CmpOutcome {
    Ord(std::cmp::Ordering),
    Err(Flow),
}

#[inline]
fn truthy_ord(v: Value) -> CmpOutcome {
    CmpOutcome::Ord(if v.is_truthy() {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Greater
    })
}

impl SortCmp {
    /// One comparison. Never panics on classified input.
    #[inline]
    pub fn cmp(&self, a: &Value, b: &Value) -> CmpOutcome {
        match self {
            SortCmp::RawInt => CmpOutcome::Ord(raw_int_cmp(a, b)),
            SortCmp::RawStr => CmpOutcome::Ord(raw_str_cmp(a, b)),
            SortCmp::RawFloat => CmpOutcome::Ord(raw_float_cmp(a, b)),
            SortCmp::MixedNumeric => CmpOutcome::Ord(mixed_numeric_cmp(a, b)),
            SortCmp::Generic { hoisted_lt } => {
                // Fix B hoist: the once-per-sort resolved slot answers
                // the common single-class user list. `f.__lt__(a, b)`
                // via call_value_pub3 (module caller frame — the same
                // caller shape user_compare's call_slot uses).
                if let Some(f) = hoisted_lt {
                    match interp::call_value_pub3(f, &[a.clone(), b.clone()]) {
                        Ok(v) => return truthy_ord(v),
                        Err(_) => {
                            // Slot raised (or returned non-flow
                            // control): fall through to the full
                            // per-compare dispatch so the error text
                            // and reflected fallback stay exact.
                        }
                    }
                }
                match interp::user_lt_public(a, b) {
                    Some(r) => match r {
                        Ok(true) => CmpOutcome::Ord(std::cmp::Ordering::Less),
                        Ok(false) => CmpOutcome::Ord(std::cmp::Ordering::Greater),
                        Err(flow) => CmpOutcome::Err(flow),
                    },
                    None => match crate::builtins_modules::py_lt_public(a, b) {
                        Ok(true) => CmpOutcome::Ord(std::cmp::Ordering::Less),
                        Ok(false) => CmpOutcome::Ord(std::cmp::Ordering::Greater),
                        Err(flow) => CmpOutcome::Err(flow),
                    },
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level entry: build the comparator for an element list
// ---------------------------------------------------------------------------

/// Build the sort comparator for `items` (no key applied). One O(n)
/// classification + at most ONE `__lt__`/reflected probe set per sort
/// for a user-instance list — the per-compare cost afterwards is a
/// plain call or a raw primitive compare.
///
/// Single-class user-instance lists WITHOUT any reflected ordering
/// slot resolve `__lt__` once (hoisted). Everything else (mixed user
/// classes, user+builtin mixes, reflected-slot classes —
/// functools.total_ordering shapes) keeps the exact per-compare
/// dispatch: the reflected-priority and cross-type fallback shapes
/// differ per pair and must not be guessed from one element.
pub fn sort_cmp_of(items: &[Value]) -> SortCmp {
    match classify(items) {
        Homog::AllInt => SortCmp::RawInt,
        Homog::AllStr => SortCmp::RawStr,
        Homog::AllFloat => SortCmp::RawFloat,
        Homog::MixedNumeric => SortCmp::MixedNumeric,
        Homog::Other => {
            let mut hoisted_lt: Option<Value> = None;
            if let Some(first) = items.first() {
                if matches!(first, Value::Obj(o) if o.kind == ObjKind::UserInstance) {
                    // Single-class + no reflected slots => the
                    // per-compare shape is exactly a.__lt__(b) on that
                    // class (user_compare's non-proper arm resolves to
                    // the same call). The class identity check is
                    // Arc::ptr_eq on the TypeRef — cheap, no MRO walk.
                    let first_class = {
                        if let Value::Obj(o) = first {
                            o.type_ref
                                .read()
                                .unwrap_or_else(|p| p.into_inner())
                                .as_ref()
                                .and_then(|w| w.upgrade())
                        } else {
                            None
                        }
                    };
                    if let Some(fc) = first_class {
                        let all_same = items.iter().all(|v| match v {
                            Value::Obj(o) if o.kind == ObjKind::UserInstance => {
                                let tc = o
                                    .type_ref
                                    .read()
                                    .unwrap_or_else(|p| p.into_inner())
                                    .as_ref()
                                    .and_then(|w| w.upgrade());
                                matches!(&tc, Some(t) if std::sync::Arc::ptr_eq(t, &fc))
                            }
                            _ => false,
                        });
                        if all_same {
                            // The hoist is only sound when NO
                            // reflected ordering slot exists anywhere
                            // on the class (a __gt__/__le__/__ge__
                            // anywhere changes user_compare's
                            // resolution order per pair).
                            let no_reflected = crate::classes::special_lookup_typed(
                                &fc, "__gt__",
                            )
                            .is_none()
                                && crate::classes::special_lookup_typed(
                                    &fc, "__le__",
                                )
                                .is_none()
                                && crate::classes::special_lookup_typed(
                                    &fc, "__ge__",
                                )
                                .is_none();
                            if no_reflected {
                                if let Some((f, _)) =
                                    crate::classes::special_lookup_typed(&fc, "__lt__")
                                {
                                    hoisted_lt = Some(f);
                                }
                            }
                        }
                    }
                }
            }
            SortCmp::Generic { hoisted_lt }
        }
    }
}

/// Sort a Vec with the resolved comparator + the parked error cell
/// (ppfix-sort escalation shape). Returns Err on the FIRST compare
/// error (the resulting order of `items` is unspecified in that case —
/// the same contract as every pre-fix sort_by path).
pub fn sort_items_with(items: &mut Vec<Value>) -> Result<(), Flow> {
    let cmp = sort_cmp_of(items);
    let mut err: Option<Flow> = None;
    items.sort_by(|a, b| {
        if err.is_some() {
            return std::cmp::Ordering::Equal;
        }
        match cmp.cmp(a, b) {
            CmpOutcome::Ord(o) => o,
            CmpOutcome::Err(f) => {
                err = Some(f);
                std::cmp::Ordering::Equal
            }
        }
    });
    match err {
        Some(f) => Err(f),
        None => Ok(()),
    }
}

/// Doc-anchor re-export: `PyString` appears in Value::Str payloads.
#[allow(dead_code)]
type _StrDoc = PyString;

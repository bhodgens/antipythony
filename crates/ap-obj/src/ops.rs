//! The v1 operations surface: arithmetic with Python type promotion,
//! `repr()`/`str()` with the digit-limit check, and truthiness entry points.
//!
//! This is the trait/entry surface 02.02's executor calls. Protocol hooks
//! (`__add__` dispatch on user types) wrap these builtins in 02.03.
//!
//! Promotion rules (S-01-6/7/8, C-12..C-16):
//! - bool folds to int (S-01-9); bool ⊕ bool → int (True+True == 2).
//! - int ⊕ int → exact int (smi or bigint, invisible boundary, C-13).
//! - `/` always float (S-01-6); int/int true division is CORRECTLY ROUNDED
//!   even past 2^53 (num-bigint → decimal → f64 parse, matching CPython's
//!   long_true_divide round-half-even).
//! - int ⊕ float and float ⊕ float → IEEE-754 double (S-01-7/8).
//! - int ** negative-int → float (S-01-2); 0 ** -n raises the exact text.
//! - division/modulo by zero raises `ZeroDivisionError: division by zero`
//!   for ALL operand type pairs (A-1: one message, not "float division").
//! - int⊕float ops with a >2^53-magnitude int convert the int CORRECTLY
//!   (no double rounding): convert via decimal parse (exact), matching
//!   CPython's float(int) rounding.

use crate::dict::PyDict;
use crate::exc::PyException;
use crate::hash;
use crate::int::{Gc, Int};
use crate::value::{PyString, Tuple, Value};

/// Binary arithmetic ops the executor dispatches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    TrueDiv,
    FloorDiv,
    Mod,
    Pow,
    LShift,
    RShift,
    BitAnd,
    BitOr,
    BitXor,
}

/// `a <op> b` for numeric (+ sequence-add/mul in the v1 value model).
pub fn binop(a: &Value, op: BinOp, b: &Value) -> Result<Value, PyException> {
    // 02.04a fix pass: str % args — printf-style formatting (_py_warnings
    // and much of the stdlib depend on it).
    if op == BinOp::Mod {
        if let Value::Str(fmt) = a {
            let args: Vec<Value> = match b {
                Value::Tuple(t) => t.0.to_vec(),
                Value::List(l) => l.items().to_vec(),
                other => vec![other.clone()],
            };
            return crate::str_repr::percent_format(fmt.as_str(), &args);
        }
    }
    match op {
        BinOp::Add => match (a, b) {
            (Value::Str(x), Value::Str(y)) => Ok(Value::Str(PyString::new(format!(
                "{}{}",
                x.as_str(),
                y.as_str()
            )))),
            (Value::Bytes(x), Value::Bytes(y)) => {
                let mut v = (**x).clone();
                v.extend_from_slice(y);
                Ok(Value::Bytes(Gc::new(v)))
            }
            (Value::List(x), Value::List(y)) => {
                let mut items = x.items().to_vec();
                items.extend(y.items().iter().cloned());
                Ok(Value::List(crate::value::PyList::from_vec(items)))
            }
            (Value::Tuple(x), Value::Tuple(y)) => {
                let mut items = x.0.clone();
                items.extend(y.0.iter().cloned());
                Ok(Value::Tuple(Gc::new(Tuple(items))))
            }
            _ => numeric_binop(a, op, b),
        },
        BinOp::Mul => match (a, b) {
            // sequence repeat: str * int / int * str etc. (S-01-17: int
            // multiplier; negative multiplier → empty)
            (Value::Str(x), Value::Int(_) | Value::Bool(_)) => {
                repeat_str(x.as_str(), b.as_int().unwrap())
            }
            (Value::Int(_) | Value::Bool(_), Value::Str(y)) => {
                repeat_str(y.as_str(), a.as_int().unwrap())
            }
            (Value::Bytes(x), Value::Int(_) | Value::Bool(_)) => {
                let n = to_repeat_count(b.as_int().unwrap())?;
                let mut v = Vec::with_capacity(x.len().saturating_mul(n));
                for _ in 0..n {
                    v.extend_from_slice(x);
                }
                Ok(Value::Bytes(Gc::new(v)))
            }
            (Value::Int(_) | Value::Bool(_), Value::Bytes(y)) => {
                let n = to_repeat_count(a.as_int().unwrap())?;
                let mut v = Vec::with_capacity(y.len().saturating_mul(n));
                for _ in 0..n {
                    v.extend_from_slice(y);
                }
                Ok(Value::Bytes(Gc::new(v)))
            }
            (Value::List(x), Value::Int(_) | Value::Bool(_)) => {
                let n = to_repeat_count(b.as_int().unwrap())?;
                let mut items = Vec::with_capacity(x.len().saturating_mul(n));
                for _ in 0..n {
                    items.extend(x.items().iter().cloned());
                }
                Ok(Value::List(crate::value::PyList::from_vec(items)))
            }
            (Value::Int(_) | Value::Bool(_), Value::List(y)) => {
                let n = to_repeat_count(a.as_int().unwrap())?;
                let mut items = Vec::with_capacity(y.len().saturating_mul(n));
                for _ in 0..n {
                    items.extend(y.items().iter().cloned());
                }
                Ok(Value::List(crate::value::PyList::from_vec(items)))
            }
            (Value::Tuple(x), Value::Int(_) | Value::Bool(_)) => {
                let n = to_repeat_count(b.as_int().unwrap())?;
                let mut items = Vec::with_capacity(x.len().saturating_mul(n));
                for _ in 0..n {
                    items.extend(x.0.iter().cloned());
                }
                Ok(Value::Tuple(Gc::new(Tuple(items))))
            }
            (Value::Int(_) | Value::Bool(_), Value::Tuple(y)) => {
                let n = to_repeat_count(a.as_int().unwrap())?;
                let mut items = Vec::with_capacity(y.len().saturating_mul(n));
                for _ in 0..n {
                    items.extend(y.0.iter().cloned());
                }
                Ok(Value::Tuple(Gc::new(Tuple(items))))
            }
            _ => numeric_binop(a, op, b),
        },
        _ => numeric_binop(a, op, b),
    }
}

fn to_repeat_count(i: Int) -> Result<usize, PyException> {
    match i.to_i64() {
        Some(n) if n >= 0 => usize::try_from(n).map_err(|_| memory_error()),
        Some(_) => Ok(0), // negative repeat → empty
        None => Err(memory_error()),
    }
}

fn memory_error() -> PyException {
    PyException::new("MemoryError", "")
}

fn repeat_str(s: &str, mult: Int) -> Result<Value, PyException> {
    let n = to_repeat_count(mult)?;
    Ok(Value::Str(PyString::new(s.repeat(n))))
}

/// Numeric fast paths + promotion. Sequence/str types were handled above.
fn numeric_binop(a: &Value, op: BinOp, b: &Value) -> Result<Value, PyException> {
    let a_num = a.as_int().is_some() || matches!(a, Value::Float(_));
    let b_num = b.as_int().is_some() || matches!(b, Value::Float(_));
    if !a_num || !b_num {
        return Err(unsupported_binop(a, op, b));
    }
    // float involved anywhere → float op (S-01-7), except int**int-neg etc.
    let a_float = matches!(a, Value::Float(_));
    let b_float = matches!(b, Value::Float(_));
    if a_float || b_float {
        return float_binop(a, op, b);
    }
    // pure int/bool path
    let (x, y) = (a.as_int().unwrap(), b.as_int().unwrap());
    match op {
        BinOp::Add => Ok(Value::Int(x.add(&y))),
        BinOp::Sub => Ok(Value::Int(x.sub(&y))),
        BinOp::Mul => Ok(Value::Int(x.mul(&y))),
        BinOp::TrueDiv => true_div_int_int(&x, &y),
        BinOp::FloorDiv => {
            check_zero(&y)?;
            Ok(Value::Int(x.floor_div(&y)))
        }
        BinOp::Mod => {
            check_zero(&y)?;
            Ok(Value::Int(x.rem(&y)))
        }
        BinOp::Pow => {
            if y.is_negative() {
                // negative exponent: float result (S-01-2). 0 ** -n raises.
                if x.is_zero() {
                    return Err(PyException::zero_to_negative_power());
                }
                let base = int_to_f64_exact(&x)?;
                let exp = int_to_f64_exact(&y)?;
                return Ok(Value::Float(base.powf(exp)));
            }
            // memory-bound exactness (C-12); exponent sanity: CPython raises
            // OverflowError only when the RESULT cannot fit memory — here
            // the allocation grows until it does or the harness kills us.
            let v = x.pow(&y).expect("pow with non-negative exponent is Some");
            Ok(Value::Int(v))
        }
        BinOp::LShift => {
            let n = shift_amount(&y)?;
            Ok(Value::Int(x.shl(n)))
        }
        BinOp::RShift => {
            let n = shift_amount(&y)?;
            Ok(Value::Int(x.shr(n)))
        }
        BinOp::BitAnd => Ok(Value::Int(x.bitand(&y))),
        BinOp::BitOr => Ok(Value::Int(x.bitor(&y))),
        BinOp::BitXor => Ok(Value::Int(x.bitxor(&y))),
    }
}

/// Negative shift count → ValueError (probed text), huge count → memory.
fn shift_amount(y: &Int) -> Result<u64, PyException> {
    match y.to_i64() {
        // CPython raises MemoryError once the RESULT cannot be allocated
        // (probed V002: 1 << (2**62) -> MemoryError, not an allocation
        // abort). Any shift beyond i32 bits cannot fit a heap-constrained
        // bigint in v0; raise the exact text before allocating.
        Some(n) if n >= 0 => {
            if n > (1 << 31) {
                return Err(memory_error());
            }
            Ok(n as u64)
        }
        Some(_) => Err(PyException::new("ValueError", "negative shift count")),
        None => Err(memory_error()),
    }
}

fn check_zero(y: &Int) -> Result<(), PyException> {
    if y.is_zero() {
        Err(PyException::zero_division())
    } else {
        Ok(())
    }
}

/// int / int → float, correctly rounded (S-01-6, C-14): when the quotient
/// exceeds float range, CPython raises
/// `OverflowError: integer division result too large for a float` (probed:
/// `10**400 / 1`) — even though both operands convert individually. The
/// exact-rounded quotient is computed via decimal long division of the
/// top digits (cheap approximation of CPython's l_divmod handling: exact
/// rounding is what matters, and decimal-parse gives it).
fn true_div_int_int(x: &Int, y: &Int) -> Result<Value, PyException> {
    if y.is_zero() {
        return Err(PyException::zero_division());
    }
    // Fast path: both fit i64 (or f64 exactly).
    if let (Some(a), Some(b)) = (x.to_i64(), y.to_i64()) {
        return Ok(Value::Float(a as f64 / b as f64));
    }
    // General: scale numerator by 2^64 for precision, quotient via bigint
    // div, remainder adjusts by one ulp. Simpler exact route: compute the
    // quotient's decimal scientific form to 25 significant digits and parse
    // (f64 parse is correctly rounded; 25 digits >> 17 needed digits makes
    // the parse exact for any tie pattern that survives division).
    let num = x.to_string_raw();
    let den = y.to_string_raw();
    let (nneg, num_mag) = split_sign(&num);
    let (dneg, den_mag) = split_sign(&den);
    let neg = nneg != dneg;
    let q = scientific_quotient(&num_mag, &den_mag)?;
    let v: f64 = q
        .parse()
        .map_err(|_| PyException::int_div_too_large_for_float())?;
    if v.is_infinite() {
        return Err(PyException::int_div_too_large_for_float());
    }
    Ok(Value::Float(if neg { -v } else { v }))
}

fn split_sign(s: &str) -> (bool, &str) {
    match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    }
}

/// Decimal scientific quotient `0.d…e±X` with enough digits for exact f64
/// rounding, or Err(overflow) when |num/den| would exceed ~1.8e308.
fn scientific_quotient(num: &str, den: &str) -> Result<String, PyException> {
    // digit-length estimate of the quotient's decimal exponent
    let exp_est = num.len() as i64 - den.len() as i64;
    if exp_est > 309 {
        return Err(PyException::int_div_too_large_for_float());
    }
    // Long division on digit strings producing 45 significant digits
    // (f64 needs at most 17 + tie-breaking slack; the f64 parser rounds
    // the overlong decimal correctly).
    let mut qdigits = String::new();
    // rem as digit string, built incrementally
    let mut rem = String::new();
    let num_digits = num.as_bytes();
    let mut pos = 0usize;
    let want = 45usize;
    while qdigits.len() < want {
        // bring down the next digit (or a trailing 0 past the dividend end)
        if pos < num_digits.len() {
            rem.push(num_digits[pos] as char);
            pos += 1;
        } else {
            rem.push('0');
        }
        let rem_trim = rem.trim_start_matches('0');
        let rem_norm: &str = if rem_trim.is_empty() { "0" } else { rem_trim };
        let (digit, new_rem) = divide_digit_str(rem_norm, den);
        qdigits.push((b'0' + digit) as char);
        rem = new_rem;
    }
    // Straight long-division accounting: at digit k (1-based), the running
    // remainder has been scaled by 10^k from num's most-significant end, so
    // the quotient digits represent num/den as 0.d1d2… × 10^(N - D + 1)
    // where N = |num| digits, D = |den| digits (e.g. 10^30/3: N=30, D=1 →
    // 0.333… × 10^30 ✓).
    let total_exp = num.len() as i64 - den.len() as i64 + 1;
    Ok(format!("0.{qdigits}e{total_exp}"))
}

/// One quotient digit of rem/den (both magnitude digit strings), plus the
/// new remainder. Uses num-bigint on the small strings (correctness-first;
/// 02.05 optimizes the hot path).
fn divide_digit_str(rem: &str, den: &str) -> (u8, String) {
    use num_bigint::BigInt;
    use num_integer::Integer;
    use num_traits::Zero;
    let r = BigInt::parse_bytes(rem.as_bytes(), 10).unwrap_or_else(BigInt::zero);
    let d = BigInt::parse_bytes(den.as_bytes(), 10).unwrap_or_else(BigInt::zero);
    // digit = floor(r / d), guaranteed 0..=9 by the caller's invariants
    let (q, new_rem) = r.div_rem(&d);
    let digit = q
        .to_string()
        .parse::<u8>()
        .unwrap_or_else(|_| panic!("quotient digit out of range: {q}"));
    (digit, new_rem.to_string())
}

/// float-involved op (S-01-7/8, C-16). The int operand converts exactly.
fn float_binop(a: &Value, op: BinOp, b: &Value) -> Result<Value, PyException> {
    // str/bytes reached here = unsupported pairing
    if !matches!(a, Value::Float(_) | Value::Int(_) | Value::Bool(_))
        || !matches!(b, Value::Float(_) | Value::Int(_) | Value::Bool(_))
    {
        return Err(unsupported_binop(a, op, b));
    }
    let af = value_as_f64(a)?;
    let bf = value_as_f64(b)?;
    let r: f64 = match op {
        BinOp::Add => af + bf,
        BinOp::Sub => af - bf,
        BinOp::Mul => af * bf,
        BinOp::TrueDiv => {
            if bf == 0.0 {
                return Err(PyException::zero_division());
            }
            af / bf
        }
        BinOp::FloorDiv => {
            if bf == 0.0 {
                return Err(PyException::zero_division());
            }
            // CPython float__floordiv_ (S-01-3 for floats)
            (af / bf).floor()
        }
        BinOp::Mod => {
            // 02.04a fix pass: str % args — printf-style formatting
            // (_py_warnings, and stdlib at large, rely on it).
            if let Value::Str(fmt) = a {
                let args: Vec<Value> = match b {
                    Value::Tuple(t) => t.0.to_vec(),
                    Value::List(l) => l.items().to_vec(),
                    other => vec![other.clone()],
                };
                return crate::str_repr::percent_format(fmt.as_str(), &args);
            }
            if bf == 0.0 {
                return Err(PyException::zero_division());
            }
            // float_rem: fmod + sign-of-divisor fix (transcribed from
            // floatobject.c, matches S-01-4 float analogue)
            let mut m = af % bf; // Rust % == C fmod for floats
            if m != 0.0 {
                if (bf < 0.0) != (m < 0.0) {
                    m += bf;
                }
            } else {
                m = if bf.is_sign_negative() { -0.0 } else { 0.0 };
            }
            return Ok(Value::Float(m));
        }
        BinOp::Pow => {
            // float ** float with CPython's special cases
            if bf == 0.0 {
                return Ok(Value::Float(1.0));
            }
            if af == 0.0 && bf < 0.0 {
                return Err(PyException::float_zero_to_negative_power());
            }
            af.powf(bf)
        }
        BinOp::LShift | BinOp::RShift | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            return Err(unsupported_binop(a, op, b))
        }
    };
    Ok(Value::Float(r))
}

/// Exact value→f64 (int conversion may overflow → OverflowError text).
fn value_as_f64(v: &Value) -> Result<f64, PyException> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Bool(b) => Ok(f64::from(*b)),
        Value::Int(i) => int_to_f64_exact(i),
        _ => unreachable!("float_binop pre-checks operand types"),
    }
}

/// int → f64 with the S-01-11 OverflowError on out-of-range.
pub fn int_to_f64_exact(i: &Int) -> Result<f64, PyException> {
    i.to_f64().ok_or_else(PyException::int_too_large_for_float)
}

fn unsupported_binop(a: &Value, op: BinOp, b: &Value) -> PyException {
    let is_sequence = matches!(
        a,
        Value::Str(_) | Value::Bytes(_) | Value::List(_) | Value::Tuple(_)
    );
    let msg = match op {
        BinOp::Add if !is_sequence => {
            // Numeric left operand: CPython's generic message (c027 probe
            // `1 + "x"`); sequence text only applies to sequences.
            format!(
                "unsupported operand type(s) for {}: '{}' and '{}'",
                "+",
                a.type_name(),
                b.type_name()
            )
        }
        BinOp::Add => {
            if a.type_name() == "bytes" {
                // bytes.__add__ text differs from str's (probed V015).
                format!("can't concat {} to bytes", b.type_name())
            } else {
                format!(
                    "can only concatenate {} (not \"{}\") to {}",
                    a.type_name(),
                    b.type_name(),
                    a.type_name()
                )
            }
        }
        BinOp::Mul => format!(
            "can't multiply sequence by non-int of type '{}'",
            b.type_name()
        ),
        _ => format!(
            "unsupported operand type(s) for {}: '{}' and '{}'",
            match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::TrueDiv => "/",
                BinOp::FloorDiv => "//",
                BinOp::Mod => "%",
                BinOp::Pow => "** or pow()",
                BinOp::LShift => "<<",
                BinOp::RShift => ">>",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
                BinOp::BitXor => "^",
            },
            a.type_name(),
            b.type_name()
        ),
    };
    PyException::new("TypeError", msg)
}

// -- repr / str ---------------------------------------------------------------

/// `repr(v)` (S-01-40..45) with the C-14 int digit-limit check: `repr(int)`
/// IS `str(int)`, so a >4300-digit int raises the str-form ValueError here.
pub fn repr(v: &Value) -> Result<String, PyException> {
    // Int limit first (the value layer cannot raise from fmt_repr's recursion
    // without breaking container reprs — CPython raises there too, probed:
    // repr([10**5000]) raises; implemented by pre-checking the root and all
    // int leaves below).
    check_int_limits(v)?;
    let mut out = String::new();
    let mut active = std::collections::HashSet::new();
    v.fmt_repr(&mut out, &mut active);
    Ok(out)
}

/// `str(v)`: identical to repr for builtins (S-01-40/41/44; str(container)
/// == repr(container)), and str(str) is the string itself (S-01-42).
pub fn to_str(v: &Value) -> Result<String, PyException> {
    match v {
        Value::Str(s) => Ok(s.as_str().to_string()),
        other => repr(other),
    }
}

/// C-14/A-2 check: every int reachable from `v` must fit the digit limit
/// for str conversion. (CPython raises ValueError on `print([10**5000])`.)
/// Cycles are guarded with the same address-set as `fmt_repr`.
fn check_int_limits(v: &Value) -> Result<(), PyException> {
    fn walk(v: &Value, active: &mut std::collections::HashSet<usize>) -> Result<(), PyException> {
        match v {
            Value::Int(i) => {
                if i.decimal_digits() > crate::int::get_int_str_digit_limit() {
                    return Err(PyException::int_str_limit(
                        crate::int::get_int_str_digit_limit(),
                    ));
                }
                Ok(())
            }
            Value::Bool(_) | Value::None | Value::Float(_) => Ok(()),
            Value::Str(_) | Value::Bytes(_) => Ok(()),
            Value::List(l) => {
                let id = crate::value::addr_of(l);
                if !active.insert(id) {
                    return Ok(()); // cycle: already checked along this path
                }
                let items = l.items();
                let r = items.iter().try_for_each(|e| walk(e, active));
                active.remove(&id);
                r
            }
            Value::Tuple(t) => t.0.iter().try_for_each(|e| walk(e, active)),
            Value::Dict(d) => {
                // Cycle guard keyed on the dict payload address (same key
                // as fmt_repr; V065: ys["self"] = ys).
                let ptr = std::rc::Rc::as_ptr(d) as *const u8 as usize;
                if !active.insert(ptr) {
                    return Ok(()); // cycle
                }
                let r = d
                    .items()
                    .into_iter()
                    .try_for_each(|(k, val)| walk(&k, active).and_then(|()| walk(&val, active)));
                active.remove(&ptr);
                r
            }
            Value::Set(s) => s.iter().try_for_each(|e| walk(&e, active)),
            Value::Frozen(s) => s.iter().try_for_each(|e| walk(&e, active)),
            Value::Ellipsis | Value::Obj(_) => Ok(()),
        }
    }
    let mut active = std::collections::HashSet::new();
    walk(v, &mut active)
}

// -- hash / eq / cmp entry ----------------------------------------------------

/// `hash(v)` (S-01-35..39). Frozenset VALUES in v1 are `Value::Set` frozen
/// by the executor; `py_hash` on a set arm raises `unhashable type: 'set'`
/// (correct for mutable sets), and frozenset hashing lands in 02.03 when
/// frozenset becomes a distinct type (documented in dict.rs).
pub fn hash(v: &Value) -> Result<i64, PyException> {
    hash::py_hash(v)
}

/// `a == b` (S-01-30/33).
pub fn eq(a: &Value, b: &Value) -> bool {
    crate::dict::values_eq(a, b)
}

/// `a < b` → bool or the exact TypeError (S-01-34); executor raises.
pub fn lt(a: &Value, b: &Value) -> Result<bool, PyException> {
    crate::dict::values_cmp(a, b).map(|o| o == std::cmp::Ordering::Less)
}

/// `divmod(a, b)` (S-01-5).
pub fn divmod(a: &Value, b: &Value) -> Result<(Value, Value), PyException> {
    match (a, b) {
        (Value::Float(_), _) | (_, Value::Float(_)) => {
            // float divmod: _float_div_mod transcription (floatobject.c)
            let x = value_as_f64(a)?;
            let y = value_as_f64(b)?;
            if y == 0.0 {
                return Err(PyException::zero_division());
            }
            let mut m = x % y;
            let mut d = (x - m) / y;
            if m != 0.0 {
                if (y < 0.0) != (m < 0.0) {
                    m += y;
                    d -= 1.0;
                }
            } else {
                m = if y.is_sign_negative() { -0.0 } else { 0.0 };
            }
            // snap quotient
            let fd = if d != 0.0 {
                let fl = d.floor();
                if d - fl > 0.5 {
                    fl + 1.0
                } else {
                    fl
                }
            } else {
                let true_q = x / y;
                if true_q.is_sign_negative() {
                    -0.0
                } else {
                    0.0
                }
            };
            Ok((Value::Float(fd), Value::Float(m)))
        }
        _ => {
            let (x, y) = (
                a.as_int().ok_or_else(|| type_err_divmod(a, b))?,
                b.as_int().ok_or_else(|| type_err_divmod(a, b))?,
            );
            if y.is_zero() {
                return Err(PyException::zero_division());
            }
            let (q, r) = x.divmod(&y);
            Ok((Value::Int(q), Value::Int(r)))
        }
    }
}

fn type_err_divmod(a: &Value, b: &Value) -> PyException {
    PyException::new(
        "TypeError",
        format!(
            "unsupported operand type(s) for divmod(): '{}' and '{}'",
            a.type_name(),
            b.type_name()
        ),
    )
}

/// Unary `-` (numeric).
pub fn neg(v: &Value) -> Result<Value, PyException> {
    match v {
        Value::Int(i) => Ok(Value::Int(i.neg())),
        Value::Bool(b) => Ok(Value::Int(Int::Smi(-i64::from(*b)))),
        Value::Float(f) => Ok(Value::Float(-*f)),
        other => Err(PyException::new(
            "TypeError",
            format!("bad operand type for unary -: '{}'", other.type_name()),
        )),
    }
}

/// `~` (int only).
pub fn invert(v: &Value) -> Result<Value, PyException> {
    match v {
        Value::Int(i) => Ok(Value::Int(i.invert())),
        Value::Bool(b) => Ok(Value::Int(Int::Smi(!i64::from(*b)))),
        other => Err(PyException::new(
            "TypeError",
            format!("bad operand type for unary ~: '{}'", other.type_name()),
        )),
    }
}

/// int(float) truncation toward zero (S-01-11) with exact error texts.
pub fn float_to_int(f: f64) -> Result<Int, PyException> {
    if f.is_nan() {
        return Err(PyException::nan_to_int());
    }
    if f.is_infinite() {
        return Err(PyException::inf_to_int());
    }
    if f == 0.0 {
        return Ok(Int::Smi(0));
    }
    // f = ±mant × 2^exp exactly (mant is a 53-bit integer); truncation
    // toward zero means shifting the MAGNITUDE right (an arithmetic shift
    // on the signed value would floor — wrong for negatives, S-01-11:
    // int(-3.7) == -3, not -4).
    let (mant, exp) = float_mant_exp(f);
    let mag = num_bigint::BigInt::from(mant.unsigned_abs());
    let truncated = if exp < 0 {
        mag >> (-exp as usize)
    } else {
        mag << (exp as usize)
    };
    Ok(Int::from_big(if mant < 0 { -truncated } else { truncated }))
}

/// Split a finite float into exact `(mantissa_i64, exp2)` with
/// f == mant × 2^exp (helper shared with the comparison code path).
fn float_mant_exp(f: f64) -> (i64, i32) {
    let bits = f.to_bits();
    let sign = if bits >> 63 == 1 { -1i64 } else { 1 };
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let frac = bits & ((1u64 << 52) - 1);
    if raw_exp == 0 {
        (sign * (frac as i64), -1074)
    } else {
        (sign * ((frac | (1u64 << 52)) as i64), raw_exp - 1075)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::Str(PyString::new(x))
    }
    fn i(v: i64) -> Value {
        Value::Int(Int::from_i64(v))
    }
    fn f(v: f64) -> Value {
        Value::Float(v)
    }

    #[test]
    fn true_plus_true_is_two() {
        let r = binop(&Value::Bool(true), BinOp::Add, &Value::Bool(true)).unwrap();
        assert!(matches!(r, Value::Int(Int::Smi(2))));
        assert_eq!(r.type_name(), "int"); // arithmetic widens to int (S-01-9)
        let m = binop(&Value::Bool(true), BinOp::Mul, &i(3)).unwrap();
        assert_eq!(repr(&m).unwrap(), "3");
    }

    #[test]
    fn float_promotion_and_division() {
        assert_eq!(
            repr(&binop(&i(6), BinOp::TrueDiv, &i(3)).unwrap()).unwrap(),
            "2.0"
        );
        assert_eq!(
            repr(&binop(&i(3), BinOp::TrueDiv, &i(2)).unwrap()).unwrap(),
            "1.5"
        );
        assert_eq!(
            repr(&binop(&i(1), BinOp::Add, &f(2.0)).unwrap()).unwrap(),
            "3.0"
        );
        assert_eq!(
            repr(&binop(&f(0.1), BinOp::Add, &f(0.2)).unwrap()).unwrap(),
            "0.30000000000000004"
        );
        // overflow to inf silently (S-01-8)
        assert_eq!(
            repr(&binop(&f(1e308), BinOp::Mul, &f(10.0)).unwrap()).unwrap(),
            "inf"
        );
    }

    #[test]
    fn zero_division_messages() {
        for (a, b) in [(i(1), i(0)), (f(1.0), f(0.0)), (f(1.0), i(0))] {
            let err = binop(&a, BinOp::TrueDiv, &b).unwrap_err();
            assert_eq!(err.to_string(), "ZeroDivisionError: division by zero");
            let err = binop(&a, BinOp::Mod, &b).unwrap_err();
            assert_eq!(err.to_string(), "ZeroDivisionError: division by zero");
        }
    }

    #[test]
    fn float_floor_div_and_mod() {
        // 7.0 // 2.0 == 3.0, -7.0 % 2.0 == 1.0, 7.5 % -2 == -0.5
        assert_eq!(
            repr(&binop(&f(7.0), BinOp::FloorDiv, &f(2.0)).unwrap()).unwrap(),
            "3.0"
        );
        assert_eq!(
            repr(&binop(&f(-7.0), BinOp::Mod, &f(2.0)).unwrap()).unwrap(),
            "1.0"
        );
        assert_eq!(
            repr(&binop(&f(7.5), BinOp::Mod, &f(-2.0)).unwrap()).unwrap(),
            "-0.5"
        );
        assert_eq!(
            repr(&binop(&f(-7.5), BinOp::FloorDiv, &f(2.0)).unwrap()).unwrap(),
            "-4.0"
        );
        // mixed: 7 // 2.0 → 3.0
        assert_eq!(
            repr(&binop(&i(7), BinOp::FloorDiv, &f(2.0)).unwrap()).unwrap(),
            "3.0"
        );
    }

    #[test]
    fn big_true_division() {
        // 10**30 / 3 == 3.333333333333333e+29 (probed)
        let big = Value::Int(Int::parse_base10(&format!("1{}", "0".repeat(30))).unwrap());
        assert_eq!(
            repr(&binop(&big, BinOp::TrueDiv, &i(3)).unwrap()).unwrap(),
            "3.333333333333333e+29"
        );
        // 10**400 / 1 raises OverflowError: integer division result too large for a float
        let huge = Value::Int(Int::parse_base10(&format!("1{}", "0".repeat(400))).unwrap());
        let err = binop(&huge, BinOp::TrueDiv, &i(1)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "OverflowError: integer division result too large for a float"
        );
    }

    #[test]
    fn int_float_eq_exact() {
        // (2**53+1) == 9007199254740992.0 → False (probed, exact compare)
        let big = Value::Int(Int::parse_base10("9007199254740993").unwrap());
        assert!(!eq(&big, &f(9007199254740992.0)));
        assert!(lt(&f(9007199254740992.0), &big).unwrap());
        // (10**30+1) == 1e30 → False
        let big2 = Value::Int(Int::parse_base10(&format!("1{}", "0".repeat(29))).unwrap());
        let big2 = Value::Int(big2.as_int().unwrap().add(&Int::Smi(1)));
        assert!(!eq(&big2, &f(1e30)));
        // 1 == 1.0 == True (S-01-33)
        assert!(eq(&i(1), &f(1.0)));
        assert!(eq(&Value::Bool(true), &f(1.0)));
        assert!(eq(&i(0), &f(-0.0)));
    }

    #[test]
    fn pow_paths() {
        // 2 ** -3 == 0.125
        assert_eq!(
            repr(&binop(&i(2), BinOp::Pow, &i(-3)).unwrap()).unwrap(),
            "0.125"
        );
        // 0 ** -1 raises exact text
        let err = binop(&i(0), BinOp::Pow, &i(-1)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "ZeroDivisionError: zero to a negative power"
        );
        // 2 ** 200 exact
        let r = binop(&i(2), BinOp::Pow, &i(200)).unwrap();
        assert_eq!(
            repr(&r).unwrap(),
            "1606938044258990275541962092341162602522202993782792835301376"
        );
    }

    #[test]
    fn shifts() {
        assert_eq!(
            repr(&binop(&i(-5), BinOp::RShift, &i(1)).unwrap()).unwrap(),
            "-3"
        );
        let err = binop(&i(1), BinOp::LShift, &i(-1)).unwrap_err();
        assert_eq!(err.to_string(), "ValueError: negative shift count");
        assert_eq!(
            repr(&binop(&i(1), BinOp::LShift, &i(100)).unwrap()).unwrap(),
            "1267650600228229401496703205376"
        );
    }

    #[test]
    fn divmod_mixed() {
        let (q, r) = divmod(&i(-7), &i(2)).unwrap();
        assert_eq!(
            (repr(&q).unwrap(), repr(&r).unwrap()),
            ("-4".to_string(), "1".to_string())
        );
        let (q, r) = divmod(&f(7.0), &i(2)).unwrap();
        assert_eq!(
            (repr(&q).unwrap(), repr(&r).unwrap()),
            ("3.0".to_string(), "1.0".to_string())
        );
        let (q, r) = divmod(&f(-7.5), &f(2.0)).unwrap();
        assert_eq!(
            (repr(&q).unwrap(), repr(&r).unwrap()),
            ("-4.0".to_string(), "0.5".to_string())
        );
    }

    #[test]
    fn repr_containers_and_cycles() {
        // [1, 'a', (2,), {1: 'x'}] (S-01-44 probed bytes)
        let d = PyDict::new();
        d.set(i(1), s("x")).unwrap();
        let tup = Value::Tuple(Gc::new(Tuple(vec![i(2)])));
        let list = crate::value::PyList::from_vec(vec![i(1), s("a"), tup, Value::Dict(d)]);
        assert_eq!(
            repr(&Value::List(list)).unwrap(),
            "[1, 'a', (2,), {1: 'x'}]"
        );
        // self-referential: xs = [1]; xs.append(xs) → [1, [...]]
        let xs = crate::value::PyList::from_vec(vec![i(1)]);
        let v = Value::List(xs.clone());
        xs.append(v);
        assert_eq!(repr(&Value::List(xs.clone())).unwrap(), "[1, [...]]");
        // key collapse: {1:'a', 1.0:'b', True:'c'} → {1: 'c'}
        let d2 = PyDict::new();
        d2.set(i(1), s("a")).unwrap();
        d2.set(f(1.0), s("b")).unwrap();
        d2.set(Value::Bool(true), s("c")).unwrap();
        assert_eq!(repr(&Value::Dict(d2)).unwrap(), "{1: 'c'}");
    }

    #[test]
    fn dict_order_after_deletes() {
        // S-01-21 probed sequence
        let mut d = PyDict::new();
        d.set(s("b"), i(1)).unwrap();
        d.set(s("a"), i(2)).unwrap();
        assert_eq!(repr(&Value::Dict(d.clone())).unwrap(), "{'b': 1, 'a': 2}");
        d.set(s("b"), i(3)).unwrap(); // keeps position
        assert_eq!(repr(&Value::Dict(d.clone())).unwrap(), "{'b': 3, 'a': 2}");
        d.pop(&s("b"));
        d.set(s("b"), i(9)).unwrap(); // moved to end
        assert_eq!(repr(&Value::Dict(d)).unwrap(), "{'a': 2, 'b': 9}");
    }

    #[test]
    fn str_multiplication_and_concat() {
        assert_eq!(
            repr(&binop(&s("ab"), BinOp::Mul, &i(3)).unwrap()).unwrap(),
            "'ababab'"
        );
        assert_eq!(
            repr(&binop(&s("ab"), BinOp::Mul, &i(0)).unwrap()).unwrap(),
            "''"
        );
        assert_eq!(
            repr(&binop(&i(-1), BinOp::Mul, &s("x")).unwrap()).unwrap(),
            "''"
        );
        assert_eq!(
            binop(&s("a"), BinOp::Add, &i(1)).unwrap_err().to_string(),
            "TypeError: can only concatenate str (not \"int\") to str"
        );
    }

    #[test]
    fn hash_contract() {
        // hash(1) == hash(1.0) == hash(True) == 1 (S-01-35)
        assert_eq!(hash(&i(1)).unwrap(), 1);
        assert_eq!(hash(&f(1.0)).unwrap(), 1);
        assert_eq!(hash(&Value::Bool(true)).unwrap(), 1);
        assert_eq!(hash(&i(0)).unwrap(), 0);
        assert_eq!(hash(&f(0.0)).unwrap(), 0);
        assert_eq!(hash(&f(-0.0)).unwrap(), 0);
        assert_eq!(hash(&Value::Bool(false)).unwrap(), 0);
        // hash(-1) == -2
        assert_eq!(hash(&i(-1)).unwrap(), -2);
        // hash(2**100) == 2**100 % (2**61 - 1) (S-01-36, probed)
        let p100 = Int::from_big(num_bigint::BigInt::from(2u8).pow(100));
        assert_eq!(hash(&Value::Int(p100)).unwrap(), 549755813888);
        // hash(-(10**100)) probed
        let n100 = Int::from_big(-num_bigint::BigInt::from(10u8).pow(100));
        assert_eq!(hash(&Value::Int(n100)).unwrap(), -910685213754167845);
        // hash(2**61 - 1) == 0
        let m = Int::from_big(num_bigint::BigInt::from((1u64 << 61) - 1));
        assert_eq!(hash(&Value::Int(m)).unwrap(), 0);
        // float integral hashing
        assert_eq!(hash(&f(2.0)).unwrap(), 2);
        assert_eq!(hash(&f(1.5)).unwrap(), 1152921504606846977);
        assert_eq!(hash(&f(f64::INFINITY)).unwrap(), 314159);
        assert_eq!(hash(&f(f64::NEG_INFINITY)).unwrap(), -314159);
        // tuple hash probed bytes
        let t = Value::Tuple(Gc::new(Tuple(vec![i(1), i(2)])));
        assert_eq!(hash(&t).unwrap(), -3550055125485641917);
        // unhashable texts
        assert_eq!(
            hash(&Value::List(crate::value::PyList::new()))
                .unwrap_err()
                .to_string(),
            "TypeError: unhashable type: 'list'"
        );
        // str hash pinned seed-0 (S-01-38: value unobservable; pinned for
        // reproducibility) — spot check our siphash vector
        assert_eq!(hash(&s("abc")).unwrap(), -4594863902769663758);
    }

    #[test]
    fn int_float_conversion_errors() {
        assert_eq!(
            float_to_int(f64::NAN).unwrap_err().to_string(),
            "ValueError: cannot convert float NaN to integer"
        );
        assert_eq!(
            float_to_int(f64::INFINITY).unwrap_err().to_string(),
            "OverflowError: cannot convert float infinity to integer"
        );
        assert_eq!(float_to_int(3.7).unwrap(), Int::Smi(3));
        assert_eq!(float_to_int(-3.7).unwrap(), Int::Smi(-3));
        assert_eq!(float_to_int(-0.0).unwrap(), Int::Smi(0));
        // int(1e300) exact digits (probed)
        let v = float_to_int(1e300).unwrap();
        assert_eq!(v.decimal_digits(), 301);
        assert!(v
            .to_string_raw()
            .starts_with("10000000000000000525047602552"));
    }
}

//! Python integers: i61 smi fast path + arbitrary-precision heap path.
//!
//! Semantics contract: S-01-1..S-01-14, C-12..C-14.
//!
//! - **No overflow anywhere** (C-13): smi arithmetic that leaves
//!   `[-2^60, 2^60)` promotes to `BigInt` (num-bigint); bigint arithmetic is exact,
//!   bounded only by memory. The `2^SMI_BITS ± k` boundary is invisible.
//! - **Floor division / modulo** (S-01-3/4): `//` truncates toward negative
//!   infinity, `%` takes the divisor's sign. num-bigint's `/` and `%` are
//!   *truncated*, so both are normalized here — documented, not inherited.
//! - **Shifts** (S-01-1): arithmetic, so `x << n` / `x >> n` on negatives
//!   keep the sign; huge shifts are memory-bound, not overflow-bound
//!   (C-14: `1 << 2^62` is a `MemoryError` — here the allocation simply
//!   fails/succeeds; no `OverflowError` is ever produced for shifts).
//! - **byte-exact decimal formatting** (S-01-40) with the C-14/A-2 digit
//!   limit for `str(int)` (default 4300, matching `sys.get_int_max_str_digits`).

use num_bigint::BigInt as Big;
use num_traits::{Signed, ToPrimitive, Zero, One};
use std::cmp::Ordering;

/// Documented conformance constant (C-13 boundary sweep reads THIS value):
/// the small-int fast path covers `[-2^59, 2^59)` — 61 bits of state, one
/// sign bit reserved in the discriminant layout. `2**59 - 1` is the largest
/// smi; `2**59` promotes to heap. Crossing the boundary is invisible.
pub const SMI_BITS: u32 = 60;

/// Largest smi: `2**59 - 1`.
pub const SMI_MAX: i64 = (1i64 << (SMI_BITS - 1)) - 1;
/// Smallest smi: `-2**59`.
pub const SMI_MIN: i64 = -(1i64 << (SMI_BITS - 1));

/// Default int↔str conversion digit limit (C-14 / A-2, `sys`
/// `get_int_max_str_digits` default). The RUNTIME value is adjustable via
/// `sys.set_int_max_str_digits` (02.02: the executor calls
/// [`set_int_str_digit_limit`]); this constant is the default/reset value.
pub const INT_STR_DIGIT_LIMIT: usize = 4300;

/// The live limit (process-global; v0 Python is single-threaded). Started at
/// the default, mutable only through the `sys` stub surface.
static INT_STR_DIGIT_LIMIT_LIVE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(INT_STR_DIGIT_LIMIT);

/// Read the live int↔str digit limit (sys.get_int_max_str_digits surface).
pub fn get_int_str_digit_limit() -> usize {
    INT_STR_DIGIT_LIMIT_LIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the live limit. `sys.set_int_max_str_digits(n)` validates `n >= 640`
/// at the sys surface; this setter takes the already-validated value.
pub fn set_int_str_digit_limit(n: usize) {
    INT_STR_DIGIT_LIMIT_LIVE.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// A Python `int`: either the inline small-int fast path or a heap bigint.
///
/// Clone is cheap in the smi case; the bigint arm is `Gc<Big>`-shaped (a Box
/// until 02.04 swaps in the MMTk handle — see crate docs).
#[derive(Debug, Clone, PartialEq)]
pub enum Int {
    /// Inline small integer (bools included via `Value::Bool`, not here).
    Smi(i64),
    /// Arbitrary-precision integer. Invariant: never holds a value within
    /// smi range (normalization on construction) so `eq` between arms is
    /// always false without cross-arm comparison.
    Big(Gc<Big>),
}

/// Heap handle placeholder: `Rc<T>` — SHARED ownership, because Python
/// semantics demand aliasing (`ys = xs; ys.append(4)` mutates `xs`,
/// S-01-19) and self-referential structures (`xs.append(xs)` → repr
/// `[1, [...]]`, S-01-44) are possible with plain builtins. A `Box` here
/// would deep-clone and break both. Mutable containers reach interior
/// mutability in 02.02's executor (`RefCell`/`Mutex` wrapper); 02.04 swaps
/// this alias for the MMTk handle type, whose sharing semantics match `Rc`.
pub type Gc<T> = std::rc::Rc<T>;

impl Int {
    /// Build from any i64, normalizing into the smi fast path when it fits.
    pub fn from_i64(v: i64) -> Int {
        if (SMI_MIN..=SMI_MAX).contains(&v) {
            Int::Smi(v)
        } else {
            Int::Big(Gc::new(Big::from(v)))
        }
    }

    pub fn from_big(b: Big) -> Int {
        let i = match b.to_i64() {
            Some(v) if (SMI_MIN..=SMI_MAX).contains(&v) => return Int::Smi(v),
            _ => b,
        };
        Int::Big(Gc::new(i))
    }

    /// True iff the value is exactly zero.
    pub fn is_zero(&self) -> bool {
        match self {
            Int::Smi(v) => *v == 0,
            Int::Big(b) => b.is_zero(),
        }
    }

    /// True iff the value is exactly one.
    pub fn is_one(&self) -> bool {
        match self {
            Int::Smi(v) => *v == 1,
            Int::Big(b) => **b == Big::one(),
        }
    }

    pub fn is_negative(&self) -> bool {
        match self {
            Int::Smi(v) => *v < 0,
            Int::Big(b) => b.is_negative(),
        }
    }

    /// Exact i64 view, when representable (used for smi math fast paths,
    /// index arithmetic, and the int→float quick path).
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Int::Smi(v) => Some(*v),
            Int::Big(b) => b.to_i64(),
        }
    }

    /// Borrowed bigint view for hashing/comparison fast paths.
    pub fn as_big(&self) -> Option<&Big> {
        match self {
            Int::Smi(_) => None,
            Int::Big(b) => Some(b),
        }
    }

    /// Correctly-rounded f64 conversion, or `None` when |value| exceeds the
    /// float range (`OverflowError: int too large to convert to float`,
    /// S-01-11/C-14 — the CALLER raises; `None` is the "too large" signal).
    ///
    /// Correctness: the decimal digits are re-parsed by Rust's f64 parser,
    /// which is correctly rounded (Eisel-Lemire) — matching CPython's
    /// round-half-even long→double conversion (verified vs 3.14: `float(2**53
    /// + 3)` → `9007199254740996.0`). A value whose first 40 significant
    /// digits place it below `2^-1075` (i.e. rounds to 0.0) parses as 0.0,
    /// which is what CPython produces for those too.
    pub fn to_f64(&self) -> Option<f64> {
        match self {
            Int::Smi(v) => Some(*v as f64),
            Int::Big(b) => {
                let s = b.to_string();
                let (neg, mag) = match s.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, s.as_str()),
                };
                // Guard the exponent so the *parse* never inf-overflow silently:
                // decimal digit count bounds the binary exponent; anything past
                // ~1e309 magnitudes is OverflowError territory (309 < 400 but
                // the exact threshold check is: |v| >= 2^1024 overflows, and
                // 10^d >= 2^1024 first at d = 309 — digit-count precheck keeps
                // parse from producing inf when it should raise).
                if mag.len() > 309 {
                    return None;
                }
                let v: f64 = mag.parse().ok()?;
                if v.is_infinite() {
                    return None;
                }
                Some(if neg { -v } else { v })
            }
        }
    }

    /// Is this int exactly representable in the f64 mantissa (|v| < 2^53)?
    /// Cheap guard for mixed int/float ops (S-01-7).
    pub fn fits_f64_exact(&self) -> bool {
        match self {
            Int::Smi(v) => v.abs() < (1i64 << 53),
            Int::Big(b) => b.bits() <= 53,
        }
    }

    /// `+` (C-12: exact).
    pub fn add(&self, other: &Int) -> Int {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => match a.checked_add(*b) {
                Some(v) if (SMI_MIN..=SMI_MAX).contains(&v) => Int::Smi(v),
                _ => Int::from_i64(*a).add_big(&Int::from_i64(*b)),
            },
            (a, b) => a.big_or_promote(b, |x, y| x + y),
        }
    }

    /// `-` (C-12: exact).
    pub fn sub(&self, other: &Int) -> Int {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => match a.checked_sub(*b) {
                Some(v) if (SMI_MIN..=SMI_MAX).contains(&v) => Int::Smi(v),
                _ => Int::from_i64(*a).sub_big(&Int::from_i64(*b)),
            },
            (a, b) => a.big_or_promote(b, |x, y| x - y),
        }
    }

    /// `*` (C-12: exact, `(2**200) * (2**200)` included).
    pub fn mul(&self, other: &Int) -> Int {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => match a.checked_mul(*b) {
                Some(v) if (SMI_MIN..=SMI_MAX).contains(&v) => Int::Smi(v),
                _ => Int::from_i64(*a).mul_big(&Int::from_i64(*b)),
            },
            (a, b) => a.big_or_promote(b, |x, y| x * y),
        }
    }

    /// Floor division `//` (S-01-3). Caller raises on zero divisor.
    pub fn floor_div(&self, other: &Int) -> Int {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => {
                // Overflow-free floor div: i64::MIN / -1 is the only
                // overflowing pair, and the result (-2^63) leaves smi range
                // anyway — route through big.
                match (a.checked_div_euclid(*b), a.checked_rem_euclid(*b)) {
                    (Some(q), Some(r)) => {
                        // div_euclid's remainder is in [0, |b|): that IS floor
                        // division when b > 0; when b < 0 the euclid quotient
                        // equals trunc, and floor subtracts one iff inexact.
                        let floored = if *b < 0 && r != 0 { q - 1 } else { q };
                        if (SMI_MIN..=SMI_MAX).contains(&floored) {
                            return Int::Smi(floored);
                        }
                        Int::from_i64(*a).floor_div_big(&Int::from_i64(*b))
                    }
                    _ => Int::from_i64(*a).floor_div_big(&Int::from_i64(*b)),
                }
            }
            (a, b) => a.big_or_promote(b, |x, y| {
                let (q, r) = divmod_big(x, y);
                let _ = r;
                q
            }),
        }
    }

    /// Modulo `%` (S-01-4: sign of divisor). Caller raises on zero divisor.
    pub fn rem(&self, other: &Int) -> Int {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => {
                // Python rem: r = a - floor(a/b)*b — sign of b.
                let (q, r) = match (a.checked_div_euclid(*b), a.checked_rem_euclid(*b)) {
                    (Some(q), Some(r)) => (q, r),
                    _ => return Int::from_i64(*a).rem_big(&Int::from_i64(*b)),
                };
                let floored_q = if *b < 0 && r != 0 { q - 1 } else { q };
                let py_r = a - floored_q * b;
                if (SMI_MIN..=SMI_MAX).contains(&py_r) {
                    Int::Smi(py_r)
                } else {
                    Int::from_i64(*a).rem_big(&Int::from_i64(*b))
                }
            }
            (a, b) => a.big_or_promote(b, |x, y| {
                let (_, r) = divmod_big(x, y);
                r
            }),
        }
    }

    /// `(a // b, a % b)` in one pass (S-01-5). Caller raises on zero divisor.
    pub fn divmod(&self, other: &Int) -> (Int, Int) {
        (self.floor_div(other), self.rem(other))
    }

    /// Integer power `a ** b` for b >= 0 (exact, S-01-2); b < 0 is the
    /// float path — the CALLER converts (`1 / (a**|b|)` semantics), so this
    /// returns `None` for negative exponents.
    pub fn pow(&self, exp: &Int) -> Option<Int> {
        let e = exp.to_i64()?;
        if e < 0 {
            return None;
        }
        match (self, exp) {
            (Int::Smi(_), _) => {
                // Square-and-multiply on bigs, promoting as needed; checked
                // smi pow first for the common tiny case.
                if let (Int::Smi(base), Some(e32)) = (self, u32::try_from(e).ok()) {
                    if let Some(v) = base.checked_pow(e32) {
                        if (SMI_MIN..=SMI_MAX).contains(&v) {
                            return Some(Int::Smi(v));
                        }
                    }
                }
                let b = self.to_big();
                Some(Int::from_big(b.pow(e as u32)))
            }
            _ => {
                let b = self.to_big();
                Some(Int::from_big(b.pow(e as u32)))
            }
        }
    }

    /// Left shift `a << b` (S-01-1, arithmetic). `b < 0` is a `ValueError`
    /// ("negative shift count") the CALLER raises; here b is pre-validated.
    pub fn shl(&self, bits: u64) -> Int {
        match self {
            // Smi shift: i64 shifts overflow past 62 bits silently (wrapping),
            // so only trust it when the result provably fits: a << b fits smi
            // iff a fits in 60-b bits signed. Anything else routes to big.
            Int::Smi(a) if bits < 60 && a.abs() < (1i64 << (SMI_BITS - 1 - bits as u32)) => {
                Int::Smi(a << bits)
            }
            _ => Int::from_big(self.to_big() << bits),
        }
    }

    /// Right shift `a >> b` (arithmetic; `-5 >> 1 == -3`).
    pub fn shr(&self, bits: u64) -> Int {
        match self {
            Int::Smi(a) if bits < 60 => {
                let v = *a >> bits;
                if (SMI_MIN..=SMI_MAX).contains(&v) {
                    Int::Smi(v)
                } else {
                    // bits >= 60 on a smi: result is 0 or -1 (sign fill).
                    Int::Smi(if *a < 0 { -1 } else { 0 })
                }
            }
            _ => Int::from_big(self.to_big() >> bits),
        }
    }

    /// Bitwise AND/OR/XOR: two's-complement semantics on arbitrary precision
    /// (num-bigint implements exactly CPython's rules for negatives).
    pub fn bitand(&self, other: &Int) -> Int {
        self.big_or_promote(other, |x, y| x & y)
    }
    pub fn bitor(&self, other: &Int) -> Int {
        self.big_or_promote(other, |x, y| x | y)
    }
    pub fn bitxor(&self, other: &Int) -> Int {
        self.big_or_promote(other, |x, y| x ^ y)
    }

    /// `~a == -a - 1` (S-01-1).
    pub fn invert(&self) -> Int {
        match self {
            Int::Smi(a) => {
                let v = !*a;
                if (SMI_MIN..=SMI_MAX).contains(&v) {
                    Int::Smi(v)
                } else {
                    Int::from_i64(v)
                }
            }
            Int::Big(b) => Int::from_big(!(**b).clone()),
        }
    }

    pub fn neg(&self) -> Int {
        match self {
            Int::Smi(a) => {
                let v = -*a;
                if (SMI_MIN..=SMI_MAX).contains(&v) {
                    Int::Smi(v)
                } else {
                    Int::from_i64(v) // -SMI_MIN is exactly -2^60, i64-fits
                }
            }
            Int::Big(b) => Int::from_big(-(**b).clone()),
        }
    }

    pub fn abs(&self) -> Int {
        match self {
            Int::Smi(a) => Int::Smi(a.abs()),
            Int::Big(b) => Int::from_big(b.abs()),
        }
    }

    /// Python ordering (exact across widths).
    #[must_use]
    pub fn cmp_int(&self, other: &Int) -> Ordering {
        match (self, other) {
            (Int::Smi(a), Int::Smi(b)) => a.cmp(b),
            (a, b) => a.to_big().cmp(&b.to_big()),
        }
    }

    /// Python `==` (exact across widths; `2**60 == 2**60` regardless of arm).
    pub fn eq_int(&self, other: &Int) -> bool {
        self.cmp_int(other) == Ordering::Equal
    }

    /// Number of decimal digits of |self| (for the C-14/A-2 limit checks).
    pub fn decimal_digits(&self) -> usize {
        match self {
            Int::Smi(v) => {
                let a = v.unsigned_abs();
                if a == 0 {
                    1
                } else {
                    a.ilog10() as usize + 1
                }
            }
            Int::Big(b) => {
                let s = b.to_string();
                let mag = s.strip_prefix('-').unwrap_or(&s);
                mag.len()
            }
        }
    }

    /// `str(int)` (S-01-40) with the C-14/A-2 digit limit applied.
    /// `Ok(bytes)` or `Err(the ValueError)`.
    pub fn to_py_string(&self) -> Result<String, crate::exc::PyException> {
        let s = self.to_string_raw();
        let digits = s.strip_prefix('-').unwrap_or(&s).len();
        let limit = get_int_str_digit_limit();
        if digits > limit {
            return Err(crate::exc::PyException::int_str_limit(limit));
        }
        Ok(s)
    }

    /// Raw decimal rendering with NO digit limit (internal: hashing, reprs
    /// of containers go through the limit only at the `str()` surface per
    /// C-14 — repr(int) IS str(int), so container reprs of >4300-digit ints
    /// also raise; that check lives in `ops::repr`, not here).
    pub fn to_string_raw(&self) -> String {
        match self {
            Int::Smi(v) => v.to_string(),
            Int::Big(b) => b.to_string(),
        }
    }

    /// Parse a base-10 int literal with surrounding whitespace and optional
    /// sign (S-01-13). Underscores are allowed between digits (Python literal
    /// syntax); leading/trailing/doubled underscores are invalid.
    /// Returns `Err(())` for invalid input — the caller wraps in
    /// `invalid_int_literal`.
    pub fn parse_base10(text: &str) -> Result<Int, ()> {
        let t = text.trim_matches(|c: char| {
            c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\x0b' || c == '\x0c'
        });
        let (neg, rest) = match t.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        if rest.is_empty() {
            return Err(());
        }
        // Underscores allowed between digits (Python literal syntax carries
        // into int()); a leading/trailing/doubled underscore is invalid.
        let cleaned: String = rest.chars().filter(|&c| c != '_').collect();
        let underscores_ok = !rest.starts_with('_')
            && !rest.ends_with('_')
            && !rest.contains("__")
            && rest.chars().filter(|&c| c == '_').count()
                == rest.chars().count() - cleaned.chars().count();
        if !underscores_ok {
            return Err(());
        }
        if cleaned.is_empty() || !cleaned.bytes().all(|b| b.is_ascii_digit()) {
            return Err(());
        }
        // int(str) digit limit (C-14/A-2): caller checks BEFORE calling by
        // counting input digits vs INT_STR_DIGIT_LIMIT — this fn stays pure.
        match cleaned.parse::<Big>() {
            Ok(b) => {
                let b = if neg { -b } else { b };
                Ok(Int::from_big(b))
            }
            Err(_) => Err(()),
        }
    }

    /// Parse with an explicit base 2..=36 (S-01-13: `int("ff", 16)` == 255).
    pub fn parse_base(text: &str, base: u32) -> Result<Int, ()> {
        if !(2..=36).contains(&base) {
            return Err(());
        }
        let t = text.trim_matches(|c: char| {
            c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\x0b' || c == '\x0c'
        });
        let (neg, rest) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        // Optional 0x/0o/0b prefix when it matches the base (CPython allows
        // it only for base 16/8/2 respectively, base 0 = infer).
        let rest = match (base, rest.get(0..2)) {
            (16, Some(p)) if p.eq_ignore_ascii_case("0x") => &rest[2..],
            (8, Some(p)) if p.eq_ignore_ascii_case("0o") => &rest[2..],
            (2, Some(p)) if p.eq_ignore_ascii_case("0b") => &rest[2..],
            _ => rest,
        };
        if rest.is_empty() {
            return Err(());
        }
        // Underscores allowed between digits (Python literal syntax carries
        // into int()); a leading/trailing/doubled underscore is invalid.
        let cleaned: String = rest.chars().filter(|&c| c != '_').collect();
        let underscores_ok = !rest.starts_with('_')
            && !rest.ends_with('_')
            && !rest.contains("__")
            && rest.chars().filter(|&c| c == '_').count()
                == rest.chars().count() - cleaned.chars().count();
        if !underscores_ok {
            return Err(());
        }
        match Big::parse_bytes(cleaned.as_bytes(), base) {
            Some(b) => {
                let b = if neg { -b } else { b };
                Ok(Int::from_big(b))
            }
            None => Err(()),
        }
    }

    // -- private helpers ---------------------------------------------------

    fn to_big(&self) -> Big {
        match self {
            Int::Smi(v) => Big::from(*v),
            Int::Big(b) => (**b).clone(),
        }
    }

    /// Run `f` on bigint views, normalizing back (promoting smi args first).
    fn big_or_promote(&self, other: &Int, f: impl Fn(&Big, &Big) -> Big) -> Int {
        match (self, other) {
            (Int::Big(a), Int::Big(b)) => Int::from_big(f(a, b)),
            (a, b) => {
                let (x, y) = (a.to_big(), b.to_big());
                Int::from_big(f(&x, &y))
            }
        }
    }

    fn add_big(&self, other: &Int) -> Int {
        self.big_or_promote(other, |a, b| a + b)
    }
    fn sub_big(&self, other: &Int) -> Int {
        self.big_or_promote(other, |a, b| a - b)
    }
    fn mul_big(&self, other: &Int) -> Int {
        self.big_or_promote(other, |a, b| a * b)
    }
    fn floor_div_big(&self, other: &Int) -> Int {
        self.big_or_promote(other, |a, b| divmod_big(a, b).0)
    }
    fn rem_big(&self, other: &Int) -> Int {
        self.big_or_promote(other, |a, b| divmod_big(a, b).1)
    }
}

/// num-integer's `Integer::div_rem` is FLOORED (Python-style); num-bigint's
/// `/` and `%` operators are TRUNCATED (C-style). Normalize the truncated
/// ops to Python semantics (S-01-3/4): adjust both quotient and remainder
/// when the signs disagree and the remainder is nonzero.
fn divmod_big(a: &Big, b: &Big) -> (Big, Big) {
    let mut q = a / b; // truncated
    let mut r = a % b; // sign of a
    if !r.is_zero() && (a.is_negative() != b.is_negative()) {
        q = q - Big::from(1);
        r = r + b; // remainder takes the divisor's sign
    }
    (q, r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big(v: i64) -> Int {
        Int::from_i64(v)
    }

    #[test]
    fn smi_boundary_invisible() {
        // C-13 boundary sweep at B = SMI_BITS.
        let b = Int::from_big(Big::from(1i64) << (SMI_BITS - 1)); // 2**59
        for k in -2i64..=2i64 {
            let v = Int::from_big((&*b.as_big().unwrap()) + k);
            let plus1 = v.add(&Int::Smi(1));
            let expect = Int::from_big((Big::from(1i64) << (SMI_BITS - 1)) + k + 1);
            assert!(plus1.eq_int(&expect), "2**{SMI_BITS}+{k}+1");
        }
        // mul sweep
        for i in 0..4u64 {
            let a = Int::from_big((Big::from(1i64) << (SMI_BITS - 1)) - i);
            let c = Int::from_big((Big::from(1i64) << (SMI_BITS - 1)) + i);
            let m = a.mul(&c);
            let expect = Int::from_big(
                ((Big::from(1i64) << (SMI_BITS - 1)) - i as i64)
                    * ((Big::from(1i64) << (SMI_BITS - 1)) + i as i64),
            );
            assert!(m.eq_int(&expect));
        }
    }

    #[test]
    fn floor_div_mod_sign_matrix() {
        // S-01-3/4 full sign table.
        let cases = [
            (7, 2, 3, 1),
            (-7, 2, -4, 1),
            (7, -2, -4, -1),
            (-7, -2, 3, -1),
        ];
        for (a, b, q, r) in cases {
            assert_eq!(big(a).floor_div(&big(b)), big(q), "{a}//{b}");
            assert_eq!(big(a).rem(&big(b)), big(r), "{a}%{b}");
        }
        // invariant a == (a//b)*b + a%b
        for (a, b) in [(1267i64, 37i64), (-1267, 37), (1267, -37), (-1267, -37)] {
            let (q, r) = big(a).divmod(&big(b));
            assert!(big(a).eq_int(&q.mul(&big(b)).add(&r)));
        }
        // bigint arm
        let a = Int::parse_base10("-1267650600228229401496703205376").unwrap();
        let b = big(37);
        let (q, r) = a.divmod(&b);
        assert!(a.eq_int(&q.mul(&b).add(&r)));
    }

    #[test]
    fn huge_arith_exact() {
        // (2**200) * (2**200)
        let p200 = Int::from_big(Big::from(2i64).pow(200));
        let p400 = p200.mul(&p200);
        assert_eq!(p400.decimal_digits(), 121);
        // known digit check: 2**400 starts with 258224987808690858965591...
        assert!(p400
            .to_string_raw()
            .starts_with("2582249878086908589655919172003"));
        // shifts
        let p100 = Int::from_big(Big::from(2i64).pow(100));
        assert!(Int::Smi(1).shl(100).eq_int(&p100));
        // (-10**25) >> 25 = -298023223876953125 (arithmetic, probed)
        assert!(Int::parse_base10("-10000000000000000000000000")
            .unwrap()
            .shr(25)
            .eq_int(&Int::parse_base10("-298023223876953125").unwrap()));
        // big-arm sign fill: (-10**25) >> 60 == -8673618 (probed)
        assert!(Int::parse_base10("-10000000000000000000000000")
            .unwrap()
            .shr(60)
            .eq_int(&Int::Smi(-8673618)));
    }

    #[test]
    fn digit_limit_texts() {
        // int("1"*5000) error text has the count; str(10**5000) does not.
        let huge = "1".repeat(5000);
        let parsed = Int::parse_base10(&huge).unwrap();
        assert_eq!(
            parsed.to_py_string().unwrap_err().to_string(),
            "ValueError: Exceeds the limit (4300 digits) for integer string conversion; use sys.set_int_max_str_digits() to increase the limit"
        );
        assert_eq!(
            crate::exc::PyException::str_int_limit(4300, 5000).to_string(),
            "ValueError: Exceeds the limit (4300 digits) for integer string conversion: value has 5000 digits; use sys.set_int_max_str_digits() to increase the limit"
        );
    }

    #[test]
    fn to_f64_matches_cpython_rounding() {
        // float(2**53 + 3) == 9007199254740996.0 (round-half-even, probed).
        let v = Int::parse_base10("9007199254740995").unwrap();
        assert_eq!(v.to_f64(), Some(9007199254740996.0));
        // overflow -> None (caller raises OverflowError)
        let big_v = Int::from_big(Big::from(10u32).pow(400));
        assert_eq!(big_v.to_f64(), None);
    }
}

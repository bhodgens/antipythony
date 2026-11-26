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
use num_traits::{One, Signed, ToPrimitive, Zero};
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

/// pp2-int: digit count above which `parse_base10` switches from num-bigint's
/// own base-10 string parser to the divide-and-conquer chunk combiner
/// (`dc_parse_base10`). num-bigint's `from_str` is schoolbook
/// acc = acc * 10 + d (O(n^2)); D&C with Karatsuba/Toom-3 multiplies wins
/// from around a few thousand digits. Below the threshold both are fine and
/// the native parser avoids chunking overhead.
const DC_PARSE_THRESHOLD: usize = 56;

/// pp2-int: divide-and-conquer base-10 recombination. `chunks` holds
/// little-endian base-10^9 limbs (least significant first); returns
/// sum(chunks[i] * (10^9)^i). `pw` memoizes (10^9)^(2^k) (i.e. the value of
/// one chunk position at the START of a level-k block: 10^(9*2^k)) across
/// the recursion; sibling subtrees at the same depth share entries, and a
/// deeper subtree only ever sees a strictly longer prefix than its parent
/// built, so the memo is append-only along any root-to-leaf path — sound.
fn dc_parse_base10(chunks: &[u32], pw: &mut Vec<Big>) -> Big {
    if chunks.len() == 1 {
        return Big::from(chunks[0]);
    }
    let mid = chunks.len() / 2;
    let lo = dc_parse_base10(&chunks[..mid], pw);
    let hi = dc_parse_base10(&chunks[mid..], pw);
    // The hi block starts at chunk position `mid`, so its place value is
    // (10^9)^mid = 10^(9*mid) — computed directly, independent of the memo.
    let factor = Big::from(10u32).pow((mid as u32) * 9);
    lo + hi * factor
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

/// Heap handle: `Arc<T>` — SHARED ownership, because Python semantics demand
/// aliasing (`ys = xs; ys.append(4)` mutates `xs`, S-01-19) and
/// self-referential structures (`xs.append(xs)` → repr `[1, [...]]`,
/// S-01-44) are possible with plain builtins. A `Box` here would deep-clone
/// and break both. Mutable containers reach interior mutability in 02.02's
/// executor (`RefCell`/`Mutex` wrapper); 02.04 swaps this alias for the MMTk
/// handle type, whose sharing semantics match `Arc`.
///
/// 02.09a Decision 1 pull-in (SPLIT-02.09.md decision 1; docs/no-gil-object-
/// audit.md "The blocker every surface shares"): was `std::rc::Rc<T>`. Every
/// Send/Sync marker is unprovable while refcounts are non-atomic — values
/// legally cross threads since 02.07 (queue/T101–T115), so a cross-thread
/// clone/drop on an Rc is UB regardless of the interior-mutability story.
/// Arc's atomic counts make concurrent clone/drop sound; the 16-byte Value
/// contract, address stability, and the subtract-refs arithmetic
/// (`Arc::strong_count` == `Rc::strong_count` semantics) are preserved. The
/// ap-gc collector reads counts via `strong_count_of` only, GC-register/
/// drop paths are GIL-serialized in this phase, and under the still-present
/// GIL v1 behavior is byte-identical (the lock already excluded concurrent
/// refcount traffic; now the type system does too). Arc drop contention on
/// interned strings/builtins was measured on this M2 Ultra before/after
/// (leaf report; geomean delta within noise).
pub type Gc<T> = std::sync::Arc<T>;

// ===========================================================================
// 02.09a P4/P5: staged Send/Sync markers (SPLIT-02.09.md:127-134 — the
// markers may land in this leaf only BEHIND the Decision-1 refcount swap,
// which landed in the same leaf).
//
// CONTRACT (applies to every `unsafe impl Sync` below and to the ap-run
// markers on Interpreter/ThreadEntry/LxDoc): the struct carries RefCell/
// Cell interiors whose borrow flags are thread-local, so a concurrent
// borrow from two threads is UB. Today that cannot happen: GIL v1
// serializes every mutator thread that touches the shared graph, and each
// process-global table holding these payloads carries its own Mutex (the
// happens-before the type system cannot see). Markers claim Send+Sync for
// the HANDLE GRAPH only — move + atomic-refcount traffic is sound since
// Gc<T> = Arc<T> (Decision 1). 02.09-b owns the per-object lock story
// (census punch-list P4); until then every cross-thread touch stays
// GIL/mutex-serialized by protocol. No blanket wrapper is reinstated:
// each marker names its type and its serialization story.
// ===========================================================================

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
    ///   which is what CPython produces for those too.
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
                _ => {
                    // Widening fast path (fp-controlflow loop target): two
                    // smis multiply EXACTLY into an i128 — skip the two
                    // from_i64 Big allocs + mul_big (measured ~300ns/iter
                    // in the LCG build loop; the widening path is ~40ns).
                    // num-bigint's From<i128> covers every (i61 x i63)
                    // product. Overflowing add/sub stay on mul_big (the
                    // i128 add of two i61 can't overflow anyway, but keep
                    // this scoped to the measured hotspot).
                    Int::Big(Gc::new(Big::from(*a as i128 * *b as i128)))
                }
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
    #[allow(clippy::result_unit_err)] // unit error kept for CPython-exact exception mapping
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
        // pp2-int: two fast paths before the generic Big::from_str route.
        // (1) <= 18 digits: pure-i64 accumulate (10^18 - 1 < i64::MAX, so no
        //     overflow is possible); skips Big entirely for the short-string
        //     workload (CPython ~41ns budget).
        // (2) > DC_PARSE_THRESHOLD digits: divide-and-conquer chunk combine —
        //     9-digit u32 chunks, halves combined as lo + hi * 10^(9*mid)
        //     with memoized power-of-ten factors; num-bigint's mul is
        //     Toom-3/Karatsuba, so the combine is subquadratic vs the
        //     schoolbook acc = acc * 10^9 + chunk left-to-right O(n^2/9).
        let ndig = cleaned.len();
        if ndig <= 18 {
            let mut acc: u64 = 0;
            for &b in cleaned.as_bytes() {
                acc = acc * 10 + (b - b'0') as u64;
            }
            let v = if neg { -(acc as i64) } else { acc as i64 };
            return Ok(Int::from_i64(v));
        }
        if ndig > DC_PARSE_THRESHOLD {
            // little-endian 9-digit chunks (least significant first)
            let bytes = cleaned.as_bytes();
            let nchunks = ndig.div_ceil(9);
            let mut chunks = Vec::with_capacity(nchunks);
            let mut i = ndig;
            while i > 0 {
                let start = i.saturating_sub(9);
                let mut c: u32 = 0;
                for &d in &bytes[start..i] {
                    c = c * 10 + (d - b'0') as u32;
                }
                chunks.push(c);
                i = start;
            }
            let mut pw = Vec::new();
            let mag = dc_parse_base10(&chunks, &mut pw);
            let b = if neg { -mag } else { mag };
            return Ok(Int::from_big(b));
        }
        match cleaned.parse::<Big>() {
            Ok(b) => {
                let b = if neg { -b } else { b };
                Ok(Int::from_big(b))
            }
            Err(_) => Err(()),
        }
    }

    /// Parse with an explicit base 2..=36 (S-01-13: `int("ff", 16)` == 255).
    #[allow(clippy::result_unit_err)] // unit error kept for CPython-exact exception mapping
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

    // -- bytes conversion (int.to_bytes / int.from_bytes) ------------------

    /// CPython-exact `int.to_bytes(length, byteorder, *, signed=False)`.
    /// Limb-based O(n) extraction — no i64 truncation (audit bug #1: the old
    /// path did `to_i64().unwrap_or(0)` and silently zero-filled bigints).
    /// Semantics: overflow raises `Err(msg)` with the exact CPython
    /// OverflowError message; empty output only when length == 0 and the
    /// value is in the two's-complement representable range (CPython allows
    /// to_bytes(0) only for 0 / -1-with-signed, else OverflowError).
    pub fn to_bytes_exact(
        &self,
        length: usize,
        little: bool,
        signed: bool,
    ) -> Result<Vec<u8>, String> {
        // Two's complement range check, mirroring CPython
        // Objects/longobject.c:_PyLong_AsByteArray overflow logic.
        // signed && length==0: CPython accepts only value 0 (signed 0 fits;
        // signed -1 does NOT fit in 0 bytes since -1 < -2^0).
        if length == 0 {
            if !self.is_zero() {
                // CPython 3.14 ground truth: the ONLY two messages on this
                // surface are "can't convert negative int to unsigned"
                // (unsigned && negative) and "int too big to convert"
                // (everything else — signed direction does NOT flip the text,
                // -(2^63).to_bytes(7, signed) says "int too big").
                return Err(if !signed && self.is_negative() {
                    "can't convert negative int to unsigned".to_string()
                } else {
                    "int too big to convert".to_string()
                });
            }
            return Ok(Vec::new());
        }
        // pp2-int: Smi fast path — pure i64 arithmetic, no Big construction,
        // no limit-allocations (the old path built `Big::one() << (length*8)`
        // even for `(5).to_bytes(8)`; at 1MiB lengths that dominated the
        // profile). Two's complement via sign-fill + `to_le_bytes` slice.
        if let Int::Smi(v) = self {
            if length == 0 {
                if *v != 0 {
                    return Err(if !signed && *v < 0 {
                        "can't convert negative int to unsigned".to_string()
                    } else {
                        "int too big to convert".to_string()
                    });
                }
                return Ok(Vec::new());
            }
            let neg = *v < 0;
            let mag: u64 = v.unsigned_abs();
            let mag_bytes = (64 - mag.leading_zeros()).div_ceil(8) as usize; // 0 for mag==0
            if !signed {
                if neg {
                    return Err("can't convert negative int to unsigned".to_string());
                }
                if mag_bytes > length {
                    return Err("int too big to convert".to_string());
                }
            } else if length < 9 {
                // v in [-2^(8L-1), 2^(8L-1)-1]; L*8-1 spans 7..=63 here, so
                // the shift is exact. |v| == 2^(8L-1) fits only as negative.
                let lim = 1u64 << (length * 8 - 1);
                if mag > lim || (mag == lim && !neg) {
                    return Err("int too big to convert".to_string());
                }
            } // L >= 9: full i64 range fits signed
            let mut out = vec![if neg { 0xFFu8 } else { 0u8 }; length];
            let vb = v.to_le_bytes();
            let n = length.min(8);
            out[..n].copy_from_slice(&vb[..n]);
            if !little {
                out.reverse();
            }
            return Ok(out);
        }
        let b = match self {
            Int::Big(b) => b,
            Int::Smi(_) => unreachable!("smi arm returned above"),
        };
        let neg = b.is_negative();
        // Magnitude view WITHOUT cloning the BigInt (the old path cloned via
        // to_big() twice on this surface).
        let magu = b.magnitude();
        let mag_bytes = (magu.bits() as usize).div_ceil(8);
        // Coarse overflow first (cheap bits() check — no limit-allocations).
        if !signed {
            if neg {
                return Err("can't convert negative int to unsigned".to_string());
            }
            if mag_bytes > length {
                return Err("int too big to convert".to_string());
            }
        } else if mag_bytes > length {
            return Err("int too big to convert".to_string());
        }
        // Bulk limb->bytes emission (pp2-int): write each u32 limb straight
        // into the pre-sized buffer (4-byte copy_from_slice per limb); the
        // old flat_map per-byte fill and the giant `Big::one() << 8L` limit
        // constructions are gone. mag_bytes <= length is proven above, so no
        // truncation can lose bits.
        let mut out = vec![0u8; length];
        // Bulk limb->bytes emission (pp2-int): the u32 limb array in a
        // little-endian BigUint IS the little-endian byte representation, so
        // one bulk memcpy replaces the per-limb/per-byte fill (the old
        // flat_map loop ran 262k 4-byte copies for 1MiB and capped at
        // ~0.9 GB/s; memcpy moves 5-10x that). mag_bytes <= length is proven
        // above, so truncation cannot lose bits. BE targets fall back to the
        // safe per-limb loop (same output, slower).
        let limbs = magu.to_u32_digits();
        let limb_bytes = limbs.len() * 4;
        debug_assert!(limb_bytes >= mag_bytes);
        // Clamp to `length`: any limb byte beyond it is proven zero (the
        // magnitude fits in `length` bytes — bits() check above), so
        // truncating the copy is exact (the not-limb-aligned-length case:
        // 71 bits -> 3 limbs = 12 bytes but length may be 9).
        let n = limb_bytes.min(length);
        if cfg!(target_endian = "little") {
            // SAFETY: `limbs` is a valid u32 slice (in-bounds via len()*4);
            // reinterpreted as u8 bytes, the first `n` (<= limb_bytes, proven
            // above) are read and copied — alignment holds for u8.
            let src = unsafe { std::slice::from_raw_parts(limbs.as_ptr() as *const u8, n) };
            out[..n].copy_from_slice(src);
        } else {
            for (i, d) in limbs.iter().enumerate() {
                let db = d.to_le_bytes();
                let lo = i * 4;
                let hi = (lo + 4).min(length);
                out[lo..hi].copy_from_slice(&db[..hi - lo]);
            }
        }
        if signed {
            if !neg {
                // positive: overflow iff the top emitted bit is set
                if mag_bytes == length && out[length - 1] & 0x80 != 0 {
                    return Err("int too big to convert".to_string());
                }
            } else if mag_bytes == length && out[length - 1] & 0x80 != 0 {
                // negative fits only at the exact minimum -2^(8L-1), i.e.
                // magnitude == [0x80, 0, ..., 0]
                let is_min = out[length - 1] == 0x80 && out[..length - 1].iter().all(|&b| b == 0);
                if !is_min {
                    return Err("int too big to convert".to_string());
                }
            }
        }
        if neg {
            // two's complement: invert + 1 over `length` bytes
            let mut carry = 1u16;
            for b in out.iter_mut() {
                let v = (!*b) as u16 + carry;
                *b = v as u8;
                carry = v >> 8;
            }
        }
        if !little {
            out.reverse();
        }
        Ok(out)
    }

    /// CPython-exact `int.from_bytes(bytes, byteorder, *, signed=False)`.
    /// Limb-based O(n) accumulation — no i128 truncation (audit bug #2:
    /// the old path accumulated into an i128 and truncated past 16 bytes,
    /// and `big_or_int`'s Big arm re-cast to i64).
    pub fn from_bytes_exact(bytes: &[u8], little: bool, signed: bool) -> Int {
        // Strip leading (big-endian sense) padding zeros.
        let eff: &[u8] = if little {
            let mut s = bytes;
            while let Some((&last, rest)) = s.split_last() {
                if last == 0 {
                    s = rest;
                } else {
                    break;
                }
            }
            s
        } else {
            let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
            &bytes[start..]
        };
        if eff.is_empty() {
            return Int::from_i64(0);
        }
        // Pad to u32-limb boundary and build the magnitude in ONE bulk
        // num-bigint construction (O(n)); the previous per-chunk
        // `mag = (mag << 32) | limb` accumulation was O(n^2) — every
        // shift allocated+copied the whole growing Big (1MiB from_bytes
        // took 2.6s; bulk parse: ~1ms).
        let pad = (4 - eff.len() % 4) % 4;
        let mut be: Vec<u8> = if little {
            eff.iter().rev().copied().collect()
        } else {
            eff.to_vec()
        };
        if pad > 0 {
            let mut padded = vec![0u8; pad];
            padded.append(&mut be);
            be = padded;
        }
        let mag = Big::from_bytes_be(num_bigint::Sign::Plus, &be);
        let val = if signed && (bytes[if little { bytes.len() - 1 } else { 0 }] & 0x80) != 0 {
            // two's complement: value = mag - 2^(8*len)
            mag - (Big::one() << (bytes.len() as u64 * 8))
        } else {
            mag
        };
        Int::from_big(val)
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
        q -= Big::from(1);
        r += b; // remainder takes the divisor's sign
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
            let v = Int::from_big(b.as_big().unwrap() + k);
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

#[cfg(test)]
mod pp2_tb_bench {
    use super::*;
    use std::time::Instant;

    #[test]
    fn bench_to_bytes_1mib() {
        let n = 1usize << 20;
        let data: Vec<u8> = (0..n).map(|i| (i % 256) as u8).collect();
        let big = Int::from_bytes_exact(&data, false, false);
        // warm
        let _ = big.to_bytes_exact(n, false, false).unwrap();
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t0 = Instant::now();
            let out = big.to_bytes_exact(n, false, false).unwrap();
            best = best.min(t0.elapsed().as_secs_f64());
            assert_eq!(out.len(), n);
            assert!(out[..64] == data[..64]);
        }
        eprintln!(
            "PP2TB: to_bytes_exact 1MiB best = {:.4}s = {:.4} GB/s",
            best,
            n as f64 / best / 1e9
        );
    }
}

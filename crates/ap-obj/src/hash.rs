//! Python hashing, byte-exact where the spec pins it (S-01-35..S-01-39).
//!
//! - `hash(int)` = `v mod (2^61 - 1)`, negatives negated, `-1 → -2`
//!   (S-01-36; modulus 2305843009213693951).
//! - `hash(float)` = CPython's frexp/28-bit-lane folding of the mantissa
//!   into the same modulus (S-01-37); integral floats hash as their int,
//!   `hash(inf) == 314159`, `hash(-0.0) == 0`. NaN is UNSPECIFIED — we
//!   return a fixed constant (documented; never asserted by corpus).
//! - `hash(str)` / `hash(bytes)`: siphash13 with the zero seed key layout
//!   (PYTHONHASHSEED=0). Salted per process in CPython (S-01-38 UNSPECIFIED);
//!   pinning to 0 makes in-process and cross-process values deterministic.
//!   Empty string/bytes hash to 0 (CPython special case, seed-independent).
//! - `hash(bool)` = hash of the int value (S-01-35: hash(True) == 1).
//! - `hash(tuple)` = CPython 3.14's xxHash-based fold (PSF tupleobject.c
//!   transcription; probed byte-exact).
//! - `hash(frozenset)` = CPython's order-independent shuffle-bits XOR fold
//!   (PSF setobject.c transcription; probed byte-exact, S-01-26).
//! - `hash(list/dict/set)` raises `TypeError: unhashable type: '...'`
//!   (S-01-39).
//!
//! The non-salted choices are safe because C-26/S-01-25 make set iteration
//! order and string hash VALUES unobservable in conforming programs — the
//! invariant that survives is `a == b ⇒ hash(a) == hash(b)` (S-01-35).

use crate::exc::PyException;
use crate::int::{Gc, Int};
use crate::value::Value;
use num_bigint::BigUint;
use num_traits::ToPrimitive;

/// `sys.hash_info.modulus` = 2^61 - 1 (S-01-36).
pub const HASH_MODULUS: u64 = (1u64 << 61) - 1;
/// `sys.hash_info.inf` = 314159 (S-01-37).
pub const HASH_INF: i64 = 314159;
/// Our pinned value for NaN (UNSPECIFIED per S-01-37; never corpus-asserted).
pub const HASH_NAN: i64 = 0;
/// The 64-bit siphash13 zero-key layout: `PYTHONHASHSEED=0`.
const SIP_K0: u64 = 0;
const SIP_K1: u64 = 0;

/// Python `hash()` for any v1 value. `Err` for unhashable types (list, dict,
/// set) with the exact S-01-39 text.
pub fn py_hash(v: &Value) -> Result<i64, PyException> {
    match v {
        Value::None => Ok(crate::NONE_HASH),
        Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
        Value::Int(i) => hash_int(i),
        Value::Float(f) => Ok(hash_f64(*f)),
        Value::Str(sv) => Ok(sv.py_hash()),
        Value::Bytes(b) => Ok(hash_bytes(b)),
        Value::Tuple(t) => {
            let lanes: Vec<i64> = t.iter().map(py_hash).collect::<Result<Vec<_>, _>>()?;
            Ok(hash_tuple_lanes(lanes, t.len()))
        }
        Value::List(_) => Err(PyException::unhashable("list")),
        Value::Dict(_) => Err(PyException::unhashable("dict")),
        Value::Set(_) => Err(PyException::unhashable("set")),
        Value::Frozen(s) => {
            // CPython's frozenset hash: order-independent shuffle-bits fold
            // (setobject.c transcription; already probed byte-exact in the
            // 02.01 differential suite).
            let hashes = s.element_hashes()?;
            Ok(hash_frozenset_lanes(&hashes))
        }
        Value::Ellipsis => Ok(crate::ELLIPSIS_HASH),
        // Generic objects hash by identity (default object.__hash__):
        // pointer-based, process-lifetime stable, never -1.
        // Exception: weakref tokens hash BY TARGET (CPython ref semantics —
        // refs to the same object dedupe in sets/dicts; WeakSet/WKD keys).
        Value::Obj(o) => {
            // A-4 item 1: bytearray is UNHASHABLE in CPython (mutable buffer)
            // — hash(bytearray(b'a')) raises TypeError exactly like list/dict.
            if o.kind == crate::object::ObjKind::Opaque
                && o.name == "bytearray"
                && crate::dict::bytearray_data(o).is_some()
            {
                return Err(PyException::unhashable("bytearray"));
            }
            if let Some(t) = crate::value::weakref_target_addr(o) {
                return Ok(hash_i64(t as i64));
            }
            // W111: int-payload instances (IntEnum members built through the
            // int.__new__(cls, v) derived-instance shape) hash by PAYLOAD —
            // CPython: hash(member) == hash(int value); dict keys keyed by
            // the numeric value (http.client.responses[200]).
            if o.kind == crate::object::ObjKind::UserInstance {
                if let Some(crate::Value::Int(n)) = o
                    .attrs
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&crate::Value::Str(crate::PyString::new("__ap_int__")))
                {
                    return hash_int(&n);
                }
            }
            let h = crate::value::addr_of(o) as i64;
            Ok(if h == -1 { -2 } else { h })
        }
    }
}

/// `hash(int)` (S-01-36): `v mod (2^61-1)`, sign preserved, `-1 → -2`.
pub fn hash_int(v: &Int) -> Result<i64, PyException> {
    match v.to_i64() {
        Some(x) => Ok(hash_i64(x)),
        None => {
            let b = v.as_big().ok_or_else(|| PyException::unhashable("int"))?;
            let mag = b.magnitude();
            let m = biguint_mod_hashmod(mag);
            let mut h = m as i64;
            if v.is_negative() {
                h = -h;
            }
            Ok(if h == -1 { -2 } else { h })
        }
    }
}

/// i64 fast path: same modulus arithmetic on non-negative residue.
pub fn hash_i64(x: i64) -> i64 {
    let m = (x.unsigned_abs() % HASH_MODULUS) as i64;
    let h = if x < 0 { -m } else { m };
    if h == -1 {
        -2
    } else {
        h
    }
}

/// Reduce a bigint magnitude modulo 2^61-1. The modulus is prime and < 2^61,
/// so plain BigUint modulo works and is exact.
fn biguint_mod_hashmod(mag: &BigUint) -> u64 {
    let modulus = num_bigint::BigUint::from(HASH_MODULUS);
    (mag % &modulus).to_u64().unwrap_or(0)
}

/// `hash(float)` (S-01-37): CPython's `_Py_HashDouble` (pyhash.c 3.14.0).
/// Integral values hash as their int; the mantissa is folded 28 bits at a
/// time into the 61-bit modulus; exponent rotation at the end.
pub fn hash_f64(v: f64) -> i64 {
    if !v.is_finite() {
        if v.is_infinite() {
            return if v > 0.0 { HASH_INF } else { -HASH_INF };
        }
        return HASH_NAN;
    }
    // Integral values hash as their exact int value, whatever the magnitude
    // (S-01-37: `hash(2.0) == 2`; `hash(2**53 + 0.0)` == 9007199254740992).
    let truncated = v.trunc();
    if truncated == v && v.abs() < 9.007_199_254_740_992e15 * 8.0 {
        // |v| < 2^56: exact i64 view exists for the integral part up to 2^63.
        if let Some(i) = truncated.to_i64() {
            return hash_i64(i);
        }
    }
    // General case: 28-bit-lane fold (CPython pyhash.c _Py_HashDouble).
    let (mut m, mut e) = frexp(v);
    let sign: i64 = if m < 0.0 {
        m = -m;
        -1
    } else {
        1
    };
    let mut x: u64 = 0;
    while m != 0.0 {
        x = ((x << 28) & HASH_MODULUS) | (x >> (61 - 28));
        m *= 268_435_456.0; // 2^28
        e -= 28;
        let y = m as u64; // integer part
        m -= y as f64;
        x += y;
        if x >= HASH_MODULUS {
            x -= HASH_MODULUS;
        }
    }
    // fold the exponent in: e mod 61 (negative-aware, C-style)
    let e = if e >= 0 {
        (e % 61) as u32
    } else {
        (61 - 1 - ((-1 - e) % 61)) as u32
    };
    x = ((x << e) & HASH_MODULUS) | (x >> (61 - e));
    let h = (x as i64).wrapping_mul(sign);
    if h == -1 {
        -2
    } else {
        h
    }
}

/// C `frexp`: (mantissa in [0.5, 1), exponent). Rust has this in libm as
/// `f64::frexp` under std? It's not stable std — hand-roll it.
fn frexp(v: f64) -> (f64, i32) {
    if v == 0.0 {
        return (v, 0);
    }
    let bits = v.to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    if raw_exp == 0 {
        // subnormal: scale up by 2^52 and recompute
        let scaled = v * (1u64 << 52) as f64;
        let (m, e) = frexp(scaled);
        return (m, e - 52);
    }
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, raw_exp - 1022)
}

/// `hash(str)` / `hash(bytes)`: siphash13 with zero keys; empty input -> 0
/// (CPython `_Py_HashBytes` special case, seed-independent — probed).
pub fn hash_bytes(data: &[u8]) -> i64 {
    if data.is_empty() {
        return 0;
    }
    let r = siphash13(SIP_K0, SIP_K1, data);
    r as i64
}

/// CPython 3.14 tuple hash (tupleobject.c): xxHash64-style fold.
/// Probed byte-exact: `hash((1,2)) == -3550055125485641917`.
pub fn hash_tuple_lanes(lanes: impl IntoIterator<Item = i64>, len: usize) -> i64 {
    const P1: u64 = 11_400_714_785_074_694_791;
    const P2: u64 = 14_029_467_366_897_019_727;
    const P5: u64 = 2_870_177_450_012_600_261;
    let mut acc: u64 = P5;
    for lane in lanes {
        acc = acc.wrapping_add((lane as u64).wrapping_mul(P2));
        acc = acc.rotate_left(31);
        acc = acc.wrapping_mul(P1);
    }
    // add input length, mangled to keep hash(())'s historical value
    acc = acc.wrapping_add(len as u64 ^ (P5 ^ 3527539));
    if acc == u64::MAX {
        acc = 1_546_275_796;
    }
    acc as i64
}

/// CPython 3.14 frozenset hash (setobject.c): order-independent shuffle-bit
/// XOR fold. Probed byte-exact: `hash(frozenset({1,2})) == -1826646154956904602`.
pub fn hash_frozenset_lanes(elem_hashes: &[i64]) -> i64 {
    fn shuffle_bits(h: u64) -> u64 {
        ((h ^ 89869747) ^ (h << 16)).wrapping_mul(3644798167)
    }
    let mut hash: u64 = 0;
    for &eh in elem_hashes {
        hash ^= shuffle_bits(eh as u64);
    }
    // factor in the entry count
    hash ^= ((elem_hashes.len() as u64) + 1).wrapping_mul(1927868237);
    // disperse nested-frozenset patterns
    hash ^= (hash >> 11) ^ (hash >> 25);
    hash = hash.wrapping_mul(69069).wrapping_add(907133923);
    if hash == u64::MAX {
        hash = 590_923_713;
    }
    hash as i64
}

/// siphash13 (CPython `Python/pyhash.c` 3.14.0, PSF license — byte-exact
/// transcription; verified against live 3.14 with seed 0 and seed 1 keys).
pub fn siphash13(k0: u64, k1: u64, data: &[u8]) -> u64 {
    fn rot(v: u64, c: u32) -> u64 {
        v.rotate_left(c)
    }
    #[inline]
    fn single_round(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
        // HALF_ROUND(v0,v1,v2,v3,13,16)
        *v0 = v0.wrapping_add(*v1);
        *v2 = v2.wrapping_add(*v3);
        *v1 = rot(*v1, 13) ^ *v0;
        *v3 = rot(*v3, 16) ^ *v2;
        *v0 = rot(*v0, 32);
        // HALF_ROUND(v2,v1,v0,v3,17,21)
        *v2 = v2.wrapping_add(*v1);
        *v0 = v0.wrapping_add(*v3);
        *v1 = rot(*v1, 17) ^ *v2;
        *v3 = rot(*v3, 21) ^ *v0;
        *v2 = rot(*v2, 32);
    }
    let b = (data.len() as u64) << 56;
    let mut v0 = k0 ^ 0x736f_6d65_7073_6575;
    let mut v1 = k1 ^ 0x646f_7261_6e64_6f6d;
    let mut v2 = k0 ^ 0x6c79_6765_6e65_7261;
    let mut v3 = k1 ^ 0x7465_6462_7974_6573;
    let mut chunks = data.chunks_exact(8);
    for chunk in &mut chunks {
        let mut mi = [0u8; 8];
        mi.copy_from_slice(chunk);
        let mi = u64::from_le_bytes(mi);
        v3 ^= mi;
        single_round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= mi;
    }
    let mut tail = b;
    for (i, &byte) in chunks.remainder().iter().enumerate() {
        tail |= (byte as u64) << (8 * i);
    }
    v3 ^= tail;
    single_round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= tail;
    v2 ^= 0xff;
    single_round(&mut v0, &mut v1, &mut v2, &mut v3);
    single_round(&mut v0, &mut v1, &mut v2, &mut v3);
    single_round(&mut v0, &mut v1, &mut v2, &mut v3);
    (v0 ^ v1) ^ (v2 ^ v3)
}

// Keep the Gc import used even if the bigint arm changes shape later.
#[allow(unused)]
fn _gc_marker(_: &Gc<num_bigint::BigInt>) {}

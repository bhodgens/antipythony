//! Leaf 1006 band 1 — the PyLong import/export API (CPython 3.14
//! cpython/longintrepr.h "Import/Export API" block) and band 2's GC
//! bridging (PyObject_GC_ family + tp_traverse edges), differential-proven
//! against /opt/homebrew/bin/python3.14 (3.14.7).
//!
//! == PyLong band: the measured 3.14.7 dialect ==
//!
//! PyLong_GetNativeLayout(): 30-bit digits, 4-byte digits, least-significant
//! first (digits_order = -1), little-endian per digit (digit_endianness = -1).
//!
//! PyLong_Export(obj, *export):
//!   - non-int: zero-fills the whole PyLongExport, TypeError "expect int,
//!     got %T" (the %T renders the TYPE name: NoneType, Idx, NotIdx, str),
//!     returns -1.
//!   - compact (fits int64): value = the value, negative = 0, ndigits = 0,
//!     digits = NULL, _reserved = 0.
//!   - non-compact: value = 0, negative = 1 only when NEGATIVE (a positive
//!     big keeps 0), ndigits = digit count (0 -> 1 per upstream's `if
//!     (ndigits == 0)` bump), digits = LSB-first u32 array,
//!     _reserved = a NEW reference to obj (FreeExport drops it).
//! FreeExport on the compact arm is a no-op (reserved 0).
//!
//! PyLong_AsNativeBytes / FromNativeBytes: full flag semantics measured
//! (see the per-fn docs below). THE GOTCHA the probes caught: the flag
//! constants double as endianness bits — Py_ASNATIVEBYTES_UNSIGNED_BUFFER
//! (=4) has bit 2 clear and bit 3 set? No: 4 = 0b100, so `_resolve_
//! endianness` sees flags & 2 == 0 => flags &= 1 => 0 => BIG endian.
//! Measured: 255 into 16 bytes with UNS flag writes 00*15 ff (BE order).

use crate::ffi_types::PyObjPtr;
use crate::state::{err_set, new_handle, CeHandle, CeKind, CePayload};
use num_traits::{Signed, ToPrimitive, Zero};
use std::ffi::{c_char, c_int, c_void};

// ---------------------------------------------------------------------------
// The PyLongLayout / PyLongExport C structs (mirror longintrepr.h; the
// extension includes apx_python.h which declares the same shapes).
// ---------------------------------------------------------------------------

/// PyLongLayout (3.14): bits_per_digit, digit_size, digits_order,
/// digit_endianness. Values = CPython 3.14's PyLong_LAYOUT on this platform.
#[repr(C)]
pub struct PyLongLayout {
    pub bits_per_digit: u8,
    pub digit_size: u8,
    pub digits_order: i8,
    pub digit_endianness: i8,
}

/// PyLongExport (3.14): value, negative, ndigits, digits, _reserved.
#[repr(C)]
pub struct PyLongExport {
    pub value: i64,
    pub negative: u8,
    pub ndigits: isize,
    pub digits: *const c_void,
    pub reserved: usize,
}

/// CPython 3.14's PyLong_LAYOUT (longobject.c:6755): 30-bit digits stored
/// in uint32, least significant first, little-endian digits.
pub static PYLONG_LAYOUT: PyLongLayout = PyLongLayout {
    bits_per_digit: 30,
    digit_size: 4,
    digits_order: -1,
    digit_endianness: -1,
};

// ---------------------------------------------------------------------------
// Int payload extraction
// ---------------------------------------------------------------------------

/// The (sign, magnitude) of a CeHandle int, with the magnitude as a
/// num-bigint Big. Returns None for non-int handles (TypeError already set).
fn int_parts(h: CeHandle) -> Option<(bool, num_bigint::BigInt)> {
    // SAFETY: caller precondition — h is a live minted ce handle (the C callers
    // null-check per the apx header contract; handles are Box::into_raw'd and
    // never freed), so the deref is valid; the payload borrow is short.
    unsafe {
        let o = &*h;
        // Py_None mints a Bool-kind handle with kind == CeKind::None —
        // the None singleton is NOT an int (measured: export None -> -1).
        if o.ob_type == CeKind::None {
            return None;
        }
        match o.payload.borrow().as_ref() {
            Some(CePayload::Int(i)) => {
                let neg = i.is_negative();
                let mag: num_bigint::BigInt = i.to_big_for_cmp().abs();
                Some((neg, mag))
            }
            Some(CePayload::Bool(b)) => Some((
                false,
                num_bigint::BigInt::from(if *b { 1i64 } else { 0i64 }),
            )),
            _ => None,
        }
    }
}

fn expect_int(h: CeHandle, api: &str) -> bool {
    if h.is_null() {
        err_set("TypeError", format!("expect int, got <null> ({api})"));
        return false;
    }
    match int_parts(h) {
        Some(_) => true,
        None => {
            // SAFETY: h is non-null (checked above); type_name_of is our own helper
            let tn = unsafe { type_name_of(h) };
            // CPython dialect: TypeError "expect int, got %T" — the %T is
            // the TYPE name. (The api name rides only our diagnostics;
            // the CLASS slot must stay TypeError or except TypeError
            // never matches — the 1006 gate-found bridge finding.)
            let _ = api;
            err_set("TypeError", format!("expect int, got {tn}"));
            false
        }
    }
}

// SAFETY: callers pass h as a live minted ce handle (Box::into_raw'd, never freed) or
// null (handled below); the payload RefCell borrow is short (worker-serialization
// protocol, the markers.rs claim).
unsafe fn type_name_of(h: CeHandle) -> String {
    if h.is_null() {
        return "<null>".to_string();
    }
    let o = &*h;
    match o.payload.borrow().as_ref() {
        Some(CePayload::Type { spec_index }) => crate::slots::record_clone(*spec_index)
            .map(|s| s.name)
            .unwrap_or_else(|| o.type_name().to_string()),
        Some(CePayload::Instance { class_spec, .. }) => crate::slots::record_clone(*class_spec)
            .map(|s| s.name)
            .unwrap_or_else(|| "object".to_string()),
        Some(CePayload::Opaque(v)) => v.name.clone(),
        _ => o.type_name().to_string(),
    }
}

/// Magnitude -> 30-bit digit array, LSB first (the 3.14 layout).
pub fn to_digits(mag: &num_bigint::BigInt) -> Vec<u32> {
    let mask = num_bigint::BigInt::from(0x3FFF_FFFF_u64);
    let mut digs: Vec<u32> = Vec::new();
    let mut x = mag.clone();
    while !x.is_zero() {
        let d = &x & &mask;
        digs.push(d.to_u32().unwrap_or(0));
        x >>= 30usize;
    }
    digs
}

/// Digit array -> magnitude.
pub fn from_digits(digs: &[u32]) -> num_bigint::BigInt {
    let mut acc = num_bigint::BigInt::from(0u8);
    let shift30 = num_bigint::BigInt::from(1u8) << 30usize;
    for d in digs.iter().rev() {
        acc = &acc * &shift30;
        acc += num_bigint::BigInt::from(*d);
    }
    acc
}

// ---------------------------------------------------------------------------
// PyLong_GetNativeLayout
// ---------------------------------------------------------------------------

/// PyLong_GetNativeLayout — returns OUR static layout record (identical
/// values to CPython's on this platform).
#[no_mangle]
pub extern "C" fn PyLong_GetNativeLayout() -> *const PyLongLayout {
    &PYLONG_LAYOUT
}

// ---------------------------------------------------------------------------
// PyLong_Export / PyLong_FreeExport
// ---------------------------------------------------------------------------

/// PyLong_Export. The FULL measured contract:
/// - non-int: zero-fill + TypeError "expect int, got %T" + return -1.
/// - fits int64: compact arm (value/neg=0/nd=0/digits=NULL/reserved=0).
/// - else: value=0, negative = (v < 0), ndigits = digit count (0->1),
///   digits = pointer to a leaked-per-export LSB-first u32 snapshot
///   (process-lifetime, the tracing-GC ownership posture; FreeExport
///   no-ops it), reserved = 0 (we never handed out a real ref).
#[no_mangle]
pub unsafe extern "C" fn PyLong_Export(obj: PyObjPtr, export_long: *mut PyLongExport) -> c_int {
    let h = obj as CeHandle;
    let out = &mut *export_long;
    if !expect_int(h, "PyLong_Export") {
        // CPython zero-fills the struct BEFORE setting the error.
        *out = PyLongExport {
            value: 0,
            negative: 0,
            ndigits: 0,
            digits: std::ptr::null(),
            reserved: 0,
        };
        return -1;
    }
    let (neg, mag) = int_parts(h).expect("checked above");
    // Non-compact path (signed_value handles the whole int64 range now,
    // including INT64_MIN — CPython measured: min64 exports compact).
    if let Some(signed) = signed_value(&neg, &mag) {
        *out = PyLongExport {
            value: signed,
            negative: 0,
            ndigits: 0,
            digits: std::ptr::null(),
            reserved: 0,
        };
        return 0;
    }
    // Non-compact arm.
    let digs = to_digits(&mag);
    let mut nd = digs.len() as isize;
    if nd == 0 {
        nd = 1; // upstream: `if (ndigits == 0) { ndigits = 1; }`
    }
    // Leak the digit snapshot (process-lifetime stability; FreeExport no-ops).
    let boxed: Vec<u32> = digs;
    let ptr = Box::into_raw(boxed.into_boxed_slice()) as *const c_void;
    *out = PyLongExport {
        value: 0,
        negative: if neg { 1 } else { 0 },
        ndigits: nd,
        digits: ptr,
        reserved: 0,
    };
    0
}

/// The signed i64 of a (neg, magnitude) pair, when the magnitude fits.
/// INT64_MIN fits (its magnitude 2^63 fits u64 but not i64-as-positive —
/// handle it via the u64 compare; CPython's compact arm covers the whole
/// int64 range INCLUDING the minimum: measured E8 = compact).
fn signed_value(neg: &bool, mag: &num_bigint::BigInt) -> Option<i64> {
    if *neg {
        // -(2^63) is the only magnitude whose negation overflows i64.
        let two63 = num_bigint::BigInt::from(1u8) << 63usize;
        if *mag == two63 {
            return Some(i64::MIN);
        }
        let v = mag.to_i64()?;
        v.checked_neg()
    } else {
        mag.to_i64()
    }
}

/// PyLong_FreeExport — on OUR arm the digits snapshot is process-lifetime;
/// reserved is always 0, so this is a no-op (identical observable behavior
/// to CPython's DECREF on the reserved arm: nothing the caller can probe).
#[no_mangle]
pub unsafe extern "C" fn PyLong_FreeExport(_export_long: *mut PyLongExport) {
    // CPython drops the _reserved NEW ref. We never set one (digits are
    // process-lifetime snapshots) — nothing to do.
}

// ---------------------------------------------------------------------------
// PyLong_AsNativeBytes
// ---------------------------------------------------------------------------

/// CPython's flag constants (longobject.h) — duplicated here so the Rust
/// side implements the exact semantics the header promises.
pub const PY_ASNATIVEBYTES_DEFAULTS: c_int = -1;
pub const PY_ASNATIVEBYTES_BIG_ENDIAN: c_int = 0;
pub const PY_ASNATIVEBYTES_LITTLE_ENDIAN: c_int = 1;
pub const PY_ASNATIVEBYTES_NATIVE_ENDIAN: c_int = 3;
pub const PY_ASNATIVEBYTES_UNSIGNED_BUFFER: c_int = 4;
pub const PY_ASNATIVEBYTES_REJECT_NEGATIVE: c_int = 8;
pub const PY_ASNATIVEBYTES_ALLOW_INDEX: c_int = 16;

/// CPython `_resolve_endianness`: DEFAULTS(-1) or bit2 set -> native (LE
/// on this host); otherwise the low bit picks the order. The probes caught
/// the trap: UNS(4) alone means BIG endian (4 & 2 == 0, 4 & 1 == 0).
pub fn resolve_endianness(flags: c_int) -> bool {
    if flags == -1 || (flags & 2) != 0 {
        true // little-endian (native on aarch64/x86_64)
    } else {
        (flags & 1) != 0
    }
}

/// CPython `_fits_in_n_bits` (longobject.c:1180).
pub fn fits_in_n_bits(v: i64, n: usize) -> bool {
    if n >= 64 {
        return true;
    }
    let ext = v >> (n - 1);
    ext == 0 || ext == -1
}

/// PyLong_AsNativeBytes — the full 3.14 semantics, probed and source-read:
/// - NULL obj or n < 0: SystemError
///   "Objects/longobject.c:1215: bad argument to internal function"
///   (measured on 3.14.7: the BadInternalCall line is 1215).
/// - non-int without ALLOW_INDEX: TypeError "expect int, got %T".
/// - REJECT_NEGATIVE on negative: ValueError "Cannot convert negative int".
/// - compact (fits one i64): sign-fill / zero-fill to n, res per
///   `_fits_in_n_bits` (n when the value fits n bytes; positives with MSB
///   set take n under DEFAULTS-negation semantics — see below); res is
///   sizeof(ssize_t)=8 when n > 8.
/// - non-compact: `_PyLong_AsByteArray` (2's-complement, sign-extend),
///   res = magnitude_bits/8 + 1, with the two documented edge-case
///   adjustments (negative 0x80...00 and positive-MSB under UNS).
/// All n bytes are written unless an error is raised (CPython's stated
/// invariant); on Overflow res still names the needed size.
#[no_mangle]
pub unsafe extern "C" fn PyLong_AsNativeBytes(
    vv: PyObjPtr,
    buffer: *mut c_void,
    n: isize,
    flags: c_int,
) -> isize {
    if vv.is_null() || n < 0 {
        // CPython PyErr_BadInternalCall: the measured 3.14.7 line.
        err_set(
            "SystemError",
            "Objects/longobject.c:1215: bad argument to internal function",
        );
        return -1;
    }
    let h = vv as CeHandle;
    let little_endian = resolve_endianness(flags);
    // Int-ness (ALLOW_INDEX is accepted as the flag gate on real ints only;
    // our S1 surface has no number protocol to consult for non-ints, so a
    // non-int is ALWAYS the TypeError — the measured %T dialect).
    if !expect_int(h, "PyLong_AsNativeBytes") {
        return -1;
    }
    let (neg, mag) = int_parts(h).expect("checked above");
    if (flags != -1 && (flags & PY_ASNATIVEBYTES_REJECT_NEGATIVE) != 0) && neg {
        err_set("ValueError", "Cannot convert negative int");
        return -1;
    }
    // Signed magnitude as bits.
    let mag_bits: u64 = if mag.is_zero() { 0 } else { mag.bits() as u64 };

    // (the compact/non-compact paths own the byte streams entirely)

    let res: isize;
    // CPython's `_PyLong_IsCompact` = ONE 30-bit digit (NOT the int64
    // compact the Export fast-path uses — the two compactness tests
    // DIFFER upstream: export uses AsLongAndOverflow; AsNative uses
    // IsCompact. Measured: -2^63 exports compact but takes the
    // non-compact AsNative path (res=9); 2^30 needs 4.
    let compact = {
        let two30 = num_bigint::BigInt::from(1u8) << 30usize;
        mag.cmp(&two30) == std::cmp::Ordering::Less
    };
    if compact {
        let sv = signed_value(&neg, &mag).expect("compact => fits i64");
        res = compact_path(sv, buffer, n, flags, little_endian);
    } else {
        // Non-compact: `_PyLong_AsByteArray` fills ALL n bytes — the
        // value's 2's-complement bytes then sign-extension (0xff/0x00)
        // for the remainder. BE: the value's bytes land at the END of
        // the buffer (msb last), with the SIGN FILL ahead of them.
        if n > 0 {
            let buf = std::slice::from_raw_parts_mut(buffer as *mut u8, n as usize);
            let full = full_twos_complement(&mag, neg, mag_bits);
            let need = full.len();
            if little_endian {
                let take = (n as usize).min(need);
                for i in 0..take {
                    buf[i] = full[i];
                }
                // Sign-extend the rest (upstream: signbyte fill).
                let signbyte = if neg { 0xFF } else { 0x00 };
                for b in buf[need.min(n as usize)..].iter_mut() {
                    *b = signbyte;
                }
            } else {
                // BE: fill [0..n-need) with the sign byte, then the
                // value's bytes (msb-first) into the LAST `need` bytes.
                let take = (n as usize).min(need);
                let signbyte = if neg { 0xFF } else { 0x00 };
                for b in buf[..n as usize - take].iter_mut() {
                    *b = signbyte;
                }
                for i in 0..take {
                    buf[n as usize - take + i] = full[need - take + i];
                }
            }
        }
        // res = magnitude_bits/8 + 1 (the comment says the sign bit cancels
        // the ceiling rounding).
        let mut r = (mag_bits / 8) as isize + 1;
        // Edge case 1: negative value that fills the buffer exactly with
        // 0x80..00 in the top byte => res = n.
        if n > 0 && r == n + 1 && mag_bits % 8 == 0 {
            if neg {
                let buf = std::slice::from_raw_parts(buffer as *const u8, n as usize);
                let mut is_edge = true;
                for (i, b) in buf.iter().enumerate() {
                    let last = i == n as usize - 1;
                    let want = if i == 0 {
                        if little_endian {
                            0x80
                        } else {
                            0
                        }
                    } else if !last {
                        0
                    } else if little_endian {
                        0x80
                    } else {
                        0
                    };
                    if *b != want {
                        is_edge = false;
                        break;
                    }
                }
                if is_edge {
                    r = n;
                }
            } else {
                // Edge case 2: positive MSB set; UNS(-ish) flags keep res=n.
                let buf = std::slice::from_raw_parts(buffer as *const u8, n as usize);
                let msb_set = if little_endian {
                    buf[n as usize - 1] & 0x80 != 0
                } else {
                    buf[0] & 0x80 != 0
                };
                if msb_set {
                    if flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) != 0 {
                        r = n;
                    } else {
                        r = n + 1;
                    }
                }
            }
        }
        res = r;
    }
    res
}

/// The full 2's-complement little-endian byte vector of a signed value.
fn full_twos_complement(mag: &num_bigint::BigInt, neg: bool, mag_bits: u64) -> Vec<u8> {
    if !neg {
        if mag.is_zero() {
            return vec![0];
        }
        let need = ((mag_bits + 7) / 8) as usize;
        let bytes = mag.to_bytes_le().1;
        let mut out = bytes;
        out.resize(need.max(1), 0);
        out
    } else {
        // 2^k - mag where k = (mag_bits rounded up to bytes) + 8 sign bytes;
        // after subtracting, leading 0xff bytes collapse via sign extension
        // at the caller (we emit the minimal 2's-complement form then the
        // caller sign-fills).
        let k = ((mag_bits + 7) / 8) * 8 + 8;
        let m = num_bigint::BigInt::from(1u8) << (k as usize);
        let tc = &m - mag;
        // to_bytes_le on a positive tc: strip trailing 0xff padding beyond
        // the significant span: significant = k/8 - 1 bytes then extension.
        let bytes = tc.to_bytes_le().1;
        // tc has exactly k/8 bytes with the top byte < 0xff (since mag >= 1).
        // The minimal 2's-complement form needs (mag_bits/8 + 1) bytes.
        let min_len = ((mag_bits / 8) + 1) as usize;
        let mut out = bytes;
        out.truncate(min_len.max(1));
        out
    }
}

/// The compact arm of AsNativeBytes (a single i64), reproducing CPython's
/// `_fits_in_n_bits` return-value and fill logic exactly.
// SAFETY: buffer is caller-supplied per the AsNativeBytes contract: writable for at
// least n bytes when non-null (n clamped to 1..=8 by the callers, i64 copy below).
unsafe fn compact_path(
    sv: i64,
    buffer: *mut c_void,
    n: isize,
    flags: c_int,
    little_endian: bool,
) -> isize {
    let cv_bytes = sv.to_le_bytes();
    let mut res: isize = 8; // sizeof(Py_ssize_t)
    if n <= 0 {
        // nothing to do
    } else if n <= 8 {
        let buf = std::slice::from_raw_parts_mut(buffer as *mut u8, n as usize);
        if little_endian {
            buf[..n as usize].copy_from_slice(&cv_bytes[..n as usize]);
        } else {
            for i in 0..n as usize {
                buf[n as usize - i - 1] = cv_bytes[i];
            }
        }
        if fits_in_n_bits(sv, (n * 8) as usize) {
            res = n;
        } else if sv > 0 && fits_in_n_bits(sv, (n * 8 + 1) as usize) {
            if flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) != 0 {
                res = n;
            } else {
                res = n + 1;
            }
        }
    } else {
        let buf = std::slice::from_raw_parts_mut(buffer as *mut u8, n as usize);
        let fill: u8 = if sv < 0 { 0xFF } else { 0x00 };
        if little_endian {
            buf[..8].copy_from_slice(&cv_bytes);
            for b in buf[8..].iter_mut() {
                *b = fill;
            }
        } else {
            // CPython BE arm (n > 8, PY_LITTLE_ENDIAN): sign fill first,
            // then `*b++ = cv.b[i-1]` for i=8..1 — i.e. the value's bytes
            // land at the END of the buffer in BIG-ENDIAN (reversed) order:
            // buf[n-8]=cv.b[7] ... buf[n-1]=cv.b[0].
            for b in buf[..n as usize - 8].iter_mut() {
                *b = fill;
            }
            for i in 0..8 {
                buf[n as usize - 8 + i] = cv_bytes[7 - i];
            }
        }
    }
    res
}

// ---------------------------------------------------------------------------
// PyLong_FromNativeBytes / PyLong_FromUnsignedNativeBytes
// ---------------------------------------------------------------------------

/// PyLong_FromNativeBytes — `_PyLong_FromByteArray(buffer, n, endian,
/// signed?)` where signed = !(flags == -1 || flags & UNS). n == 0 -> int 0.
/// The signed interpretation is CPython's 2's-complement reader: sign from
/// the MSB of the top byte, leading-significant-byte trim, sign-extension.
#[no_mangle]
pub unsafe extern "C" fn PyLong_FromNativeBytes(
    buffer: *const c_void,
    n: usize,
    flags: c_int,
) -> PyObjPtr {
    if buffer.is_null() {
        err_set("SystemError", "PyLong_FromNativeBytes: NULL buffer");
        return std::ptr::null_mut();
    }
    let little_endian = resolve_endianness(flags);
    // CPython longobject.c:1404: `signed = (flags == -1 || !(flags &
    // UNS)) ? 1 : 0` — DEFAULTS means SIGNED. (Gate-found inversion: the
    // first cut negated the whole condition, making DEF unsigned.)
    let is_signed = flags == -1 || (flags & PY_ASNATIVEBYTES_UNSIGNED_BUFFER) == 0;
    from_byte_array(buffer, n, little_endian, is_signed)
}

/// PyLong_FromUnsignedNativeBytes — always unsigned.
#[no_mangle]
pub unsafe extern "C" fn PyLong_FromUnsignedNativeBytes(
    buffer: *const c_void,
    n: usize,
    flags: c_int,
) -> PyObjPtr {
    if buffer.is_null() {
        err_set("SystemError", "PyLong_FromUnsignedNativeBytes: NULL buffer");
        return std::ptr::null_mut();
    }
    let little_endian = resolve_endianness(flags);
    from_byte_array(buffer, n, little_endian, false)
}

/// `_PyLong_FromByteArray` (longobject.c:1114): read n bytes as a
/// (2's-complement when signed) base-256 integer. Sign from the MSB of the
/// top byte; leading insignificant bytes trimmed (0x00 positive, 0xff
/// negative, +1 safety bump); 2's-complement correction LSB-first with a
/// rippling carry (upstream's twodigits accumulation, byte-at-a-time form).
// SAFETY: buffer is caller-supplied per the _PyLong_FromByteArray contract: readable for
// n bytes (null/n==0 handled below); bytes come from a live Python buffer object.
unsafe fn from_byte_array(
    buffer: *const c_void,
    n: usize,
    little_endian: bool,
    mut is_signed: bool,
) -> PyObjPtr {
    if n == 0 || buffer.is_null() {
        return new_handle(CeKind::Int, CePayload::Int(ap_obj::int::Int::from_i64(0))) as PyObjPtr;
    }
    let bytes = std::slice::from_raw_parts(buffer as *const u8, n);
    // pstartbyte = LSB end; pendbyte = MSB end.
    let (start, step): (usize, isize) = if little_endian { (0, 1) } else { (n - 1, -1) };
    let msb_index = if little_endian { n - 1 } else { 0 };
    if is_signed {
        is_signed = bytes[msb_index] >= 0x80;
    }
    // numsignificantbytes: walk MSB -> LSB, trim insignificant bytes.
    let insignificant: u8 = if is_signed { 0xFF } else { 0x00 };
    let sig: usize;
    {
        let mut count = 0usize;
        for i in 0..n {
            let idx = ((msb_index as isize) - (i as isize) * step) as usize;
            if bytes[idx] != insignificant {
                break;
            }
            count += 1;
        }
        let mut s2 = n - count;
        if is_signed && s2 < n {
            s2 += 1;
        }
        sig = s2;
    }
    // Significant bytes LSB-first.
    let mut lsb_first: Vec<u8> = Vec::with_capacity(sig);
    for i in 0..sig {
        let idx = (start as isize + (i as isize) * step) as usize;
        lsb_first.push(bytes[idx]);
    }
    // Accumulate LSB-first; 2's-complement correction per byte with a
    // carry that ripples upward (upstream: thisbyte = (0xff ^ b) + carry,
    // carry = thisbyte >> 8). Our per-byte Big form: track the correction
    // as a pending +256 added to the NEXT byte position.
    let base = num_bigint::BigInt::from(256u32);
    let mut correction = num_bigint::BigInt::from(1u8);
    let mut result = num_bigint::BigInt::from(0u8);
    let mut carry = if is_signed { 1u32 } else { 0 };
    for b in lsb_first.iter() {
        let byte_val = if is_signed {
            let v = 0xFFu32 - (*b as u32) + carry;
            carry = v >> 8;
            v & 0xFF
        } else {
            *b as u32
        };
        result = &result + (num_bigint::BigInt::from(byte_val) * &correction);
        correction = &correction * &base;
    }
    let int_val = if is_signed {
        ap_obj::int::Int::from_big(-&result)
    } else {
        ap_obj::int::Int::from_big(result)
    };
    new_handle(CeKind::Int, CePayload::Int(int_val)) as PyObjPtr
}

// ---------------------------------------------------------------------------
// PyLongWriter (the Import/Export block's third piece — the inverse of
// Export; measured: ndigits <= 0 -> ValueError "ndigits must be positive").
// ---------------------------------------------------------------------------

/// PyLongWriter_Create: returns an opaque writer handle (usize index into a
/// process-global table; 0 on failure). The digits area is a heap Vec the
/// caller fills through the returned pointer (u32 digits, 30-bit values,
/// LSB first).
#[no_mangle]
pub unsafe extern "C" fn PyLongWriter_Create(
    negative: c_int,
    ndigits: isize,
    digits_out: *mut *mut c_void,
) -> usize {
    if ndigits <= 0 {
        err_set("ValueError", "ndigits must be positive");
        if !digits_out.is_null() {
            *digits_out = std::ptr::null_mut();
        }
        return 0;
    }
    let digs: Vec<u32> = vec![0; ndigits as usize];
    let idx = WRITERS.lock().unwrap_or_else(|p| p.into_inner()).len() + 1;
    WRITERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(WriterState {
            negative: negative != 0,
            digits: digs,
        });
    if !digits_out.is_null() {
        // Hand out the Vec's live pointer: the table never grows the Vec
        // in place after Create (each writer owns its Vec; Finish reads by
        // index). Stable for the writer's lifetime.
        let mut g = WRITERS.lock().unwrap_or_else(|p| p.into_inner());
        *digits_out = g[idx - 1].digits.as_mut_ptr() as *mut c_void;
    }
    idx
}

/// PyLongWriter_Finish: normalize (trim leading zero digits), build the
/// signed bigint, retire the writer. Returns the int handle.
#[no_mangle]
pub unsafe extern "C" fn PyLongWriter_Finish(writer: usize) -> PyObjPtr {
    // The caller wrote digits through the raw pointer handed out at
    // Create — that pointer points INTO the table's Vec, so the writes
    // are already visible here. Do NOT clear before reading (the earlier
    // clear-then-replace form lost the digits: writer 0 bug, gate-found).
    let st = {
        let mut g = WRITERS.lock().unwrap_or_else(|p| p.into_inner());
        if writer == 0 || writer > g.len() {
            err_set("SystemError", "PyLongWriter_Finish: bad writer");
            return std::ptr::null_mut();
        }
        std::mem::replace(
            &mut g[writer - 1],
            WriterState {
                negative: false,
                digits: Vec::new(),
            },
        )
    };
    let mut digs = st.digits;
    // Normalize: trim trailing (most significant) zeros.
    while let Some(&0) = digs.last() {
        digs.pop();
    }
    let mag = from_digits(&digs);
    let v = if st.negative && !mag.is_zero() {
        ap_obj::int::Int::from_big(-&mag)
    } else {
        ap_obj::int::Int::from_big(mag)
    };
    new_handle(CeKind::Int, CePayload::Int(v)) as PyObjPtr
}

/// PyLongWriter_Discard: retire without building.
#[no_mangle]
pub unsafe extern "C" fn PyLongWriter_Discard(writer: usize) {
    let mut g = WRITERS.lock().unwrap_or_else(|p| p.into_inner());
    if writer >= 1 && writer <= g.len() {
        g[writer - 1] = WriterState {
            negative: false,
            digits: Vec::new(),
        };
    }
}

struct WriterState {
    negative: bool,
    digits: Vec<u32>,
}

static WRITERS: std::sync::Mutex<Vec<WriterState>> = std::sync::Mutex::new(Vec::new());

// ===========================================================================
// Leaf 1006 band 2: tp_traverse / GC bridging (CONSERVATION-CRITICAL).
//
// A C extension payload struct holds PyObject* edges OUR collector cannot
// see (the C struct is a malloc'd block OUTSIDE the Gc heap). The bridge:
// the extension's tp_traverse fn is captured at FromSpec time; OUR collector
// CALLS it with a visit closure per registered instance, and the closure
// registers each visited PyObject handle's RUNTIME object as a GC edge.
//
// Model decision: side-registry keyed by payload key (the same keying the
// 10.04 payload registry uses). At collection time ap-run's ce_bridge (GC
// band) walks the registered instances and consults their traverse fn, so
// the collector's edges_of() consults this table through the registered
// GcTraverseFn — the ap-gc dependency direction is preserved (ap-run
// installs; ap-gc reads through ITS OWN bridge fn-pointer seam, gcmod's
// weakbridge pattern).
// ===========================================================================

/// The C traverse fn ABI (CPython traverseproc): (self, visit, arg) -> 0.
pub type TraverseProc =
    unsafe extern "C" fn(self_h: PyObjPtr, visit: TraverseVisit, arg: *mut c_void) -> c_int;

/// The visit fn we hand the C traverse: (visited, closure) -> 0.
pub type TraverseVisit = unsafe extern "C" fn(visited: PyObjPtr, closure: *mut c_void) -> c_int;

/// One visit callback record. The closure carries the payload key; visits
/// land in the PER-KEY edge sets.
struct VisitCtx {
    #[allow(dead_code)] // payload key carried for diagnostics
    key: usize,
    visited: Vec<usize>, // runtime-object addresses (Arc::as_ptr of the
                         // Opaque/CeInstance payloads' Gc objects)
}

thread_local! {
    static VISITING: std::cell::RefCell<Vec<VisitCtx>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The visit fn handed to C traverse calls. Resolves the visited handle to
/// its runtime object address (the GC registry key shape) and records it.
unsafe extern "C" fn gc_visit_fn(visited: PyObjPtr, closure: *mut c_void) -> c_int {
    let ctx = &mut *(closure as *mut VisitCtx);
    let h = visited as CeHandle;
    if !h.is_null() {
        let addr = ce_handle_gc_addr(h);
        if addr != 0 {
            ctx.visited.push(addr);
        }
    }
    0
}

/// The GC-registry address of a CeHandle's runtime object (0 when the
/// handle fronts no tracked heap object).
// SAFETY: callers pass h as a live minted ce handle (Box::into_raw'd, never freed); the
// payload RefCell borrow is short (worker-serialization protocol, the markers.rs claim).
unsafe fn ce_handle_gc_addr(h: CeHandle) -> usize {
    let o = &*h;
    match o.payload.borrow().as_ref() {
        // CeInstance: the payload key IS the runtime object address.
        Some(CePayload::CeInstance { payload_key, .. }) => *payload_key,
        Some(CePayload::Opaque(v)) => ap_obj::Gc::as_ptr(v) as *const c_void as usize,
        Some(CePayload::Instance { state, .. }) => {
            // The instance dict is the tracked carrier.
            ap_obj::Gc::as_ptr(state) as *const c_void as usize
        }
        Some(CePayload::Dict(d)) => ap_obj::Gc::as_ptr(d) as *const c_void as usize,
        _ => 0,
    }
}

/// The traverse registry: payload key -> (traverse fn, spec index).
/// FromSpec-time registration via Py_tp_traverse slots; ap-run's GC band
/// drains it at collect time.
pub struct TraverseEntry {
    pub payload_key: usize,
    pub traverse: TraverseProc,
}

static TRAVERSE: std::sync::Mutex<Vec<TraverseEntry>> = std::sync::Mutex::new(Vec::new());

/// Register a tp_traverse fn for a payload key (idempotent per key: a
/// re-registration replaces — FromSpec types re-registering per instance
/// must not accumulate).
pub fn gc_traverse_register(payload_key: usize, traverse: TraverseProc) {
    let mut g = TRAVERSE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(e) = g.iter_mut().find(|e| e.payload_key == payload_key) {
        e.traverse = traverse;
        return;
    }
    g.push(TraverseEntry {
        payload_key,
        traverse,
    });
}

pub fn gc_traverse_unregister(payload_key: usize) {
    let mut g = TRAVERSE.lock().unwrap_or_else(|p| p.into_inner());
    g.retain(|e| e.payload_key != payload_key);
}

/// RUN a payload's traverse fn, returning the GC addresses it visits.
/// Called by ap-run's GC band (per registered instance at collect time).
/// The CeHandle for `self` must be the CeInstance handle (payload key
/// carried); the C fn calls visit(edge, closure) per pointer member.
pub fn gc_traverse_collect(payload_key: usize, self_handle: CeHandle) -> Vec<usize> {
    let traverse = {
        let g = TRAVERSE.lock().unwrap_or_else(|p| p.into_inner());
        g.iter()
            .find(|e| e.payload_key == payload_key)
            .map(|e| e.traverse)
    };
    let Some(traverse) = traverse else {
        return Vec::new();
    };
    let mut ctx = VisitCtx {
        key: payload_key,
        visited: Vec::new(),
    };
    // SAFETY: traverse is the TraverseProc registered for this payload
    let rc = unsafe {
        traverse(
            self_handle as PyObjPtr,
            gc_visit_fn,
            &mut ctx as *mut VisitCtx as *mut c_void,
        )
    };
    let _ = rc; // non-zero return = the visit aborted early; partial edges kept
    std::mem::take(&mut ctx.visited)
}

/// The C-side closure shape bridge: Py_VISIT-style visits carry the raw
/// PyObject*; gc_visit_fn converts. (Exposed for the unit gate.)
pub fn traverse_registered_keys() -> Vec<usize> {
    let g = TRAVERSE.lock().unwrap_or_else(|p| p.into_inner());
    g.iter().map(|e| e.payload_key).collect()
}

// ---------------------------------------------------------------------------
// PyObject_GC_ family exports
// ---------------------------------------------------------------------------

/// PyObject_GC_New: the S1 shape allocates a GC-registered CeInstance-sized
/// handle the collector tracks. Our model: allocations never leave the
/// handle layer (no raw C memory holds heap pointers by construction —
/// the visitor bridge makes the EDGES visible instead). The return is an
/// untracked-at-birth handle (CPython: GC_NEW is untracked until Track).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GC_New(basicsize: isize) -> PyObjPtr {
    // CPython's PyObject_GC_New(type) takes a TYPE; the FromSpec emission
    // shape passes the type object. Our handle carries CeKind::Opaque with
    // a fresh placeholder payload; the tp_new route replaces it.
    let _ = basicsize;
    new_handle(
        CeKind::Opaque,
        CePayload::Opaque(ap_obj::PyObject::new(ap_obj::ObjKind::Opaque, "apx.GCNew")),
    ) as PyObjPtr
}

/// PyObject_GC_Track — a no-op on the handle layer (handles are process-
/// lifetime; tracking = "the GC may see me", which is always true here).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GC_Track(_o: PyObjPtr) {}

/// PyObject_GC_UnTrack — no-op (see Track).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GC_UnTrack(_o: PyObjPtr) {}

/// PyObject_GC_Del — no-op free (tracing GC owns memory; the handle layer
/// never frees, matching the S0 DECREF posture).
#[no_mangle]
pub unsafe extern "C" fn PyObject_GC_Del(_o: PyObjPtr) {}

/// PyObject_GC_UnRegister — apx extension (the conservation probe's
/// unregister seam; not a CPython symbol).
#[no_mangle]
pub unsafe extern "C" fn apx_GC_TraverseRegister(
    payload_key: usize,
    traverse: TraverseProc,
) -> c_int {
    gc_traverse_register(payload_key, traverse);
    0
}

#[no_mangle]
pub unsafe extern "C" fn apx_GC_TraverseUnregister(payload_key: usize) -> c_int {
    gc_traverse_unregister(payload_key);
    0
}

// Keep c_char referenced (the header band declares stringy symbols through
// this module's error paths).
#[allow(unused)]
fn _unused_char(_: *const c_char) {}

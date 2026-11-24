//! simd_encode.rs — NEON kernels for the base64 fast paths
//! (leaf 01-base64-simd, plans/primitive-perf/01-encode-simd).
//!
//! Port of CPython PR #162443 (gh-124951, Modules/binascii_neon.h):
//! TBL encode over the 64-byte table, Muła/Lemire nibble decode for the
//! standard alphabet. Both kernels consume a PREFIX of the input made of
//! COMPLETE groups only (encode: 3-byte groups; decode: 4-char groups of
//! valid standard-alphabet characters) and return how many input units
//! they consumed. The tail, padding (`=`), invalid characters and ALL
//! error reporting stay on the scalar loops in builtins_modules.rs —
//! that is the byte-exactness contract (NEON never decides errors).
//!
//! Below NEON_MIN_ENCODE / NEON_MIN_DECODE the scalar loops alone are
//! faster, so the kernels return 0 there (PR constants 24 / 32).
//!
//! Our wiring only ever reaches these kernels with the STANDARD alphabet:
//! `base64.b64decode(altchars=)` / `urlsafe_*` translate to `+/` first and
//! binascii 3.14 has no `alphabet=` kwarg, so the PR's 256-byte custom
//! reverse-table decode path is structurally unreachable here (documented
//! in 01-base64-report.md). Encode uses TBL over the standard table, which
//! is the same instruction shape the PR uses for arbitrary alphabets.
//!
//! Non-aarch64 builds fall back to "consume nothing" (scalar-only), per
//! the leaf spec: `#[cfg(target_arch = "aarch64")]` + scalar fallback.

/// Shorter inputs are faster with the scalar loops alone (PR #162443).
pub const NEON_MIN_ENCODE: usize = 24;
/// Shorter inputs are faster with the scalar loops alone (PR #162443).
pub const NEON_MIN_DECODE: usize = 32;

/// NEON encode driver: `input.len() >= 24`, `out` empty with sufficient
/// capacity. Returns bytes written (= n/3*4 where n = input.len()/3*3).
///
/// # Safety
/// Caller guarantees the capacity preconditions of `encode_prefix`.
#[cfg(target_arch = "aarch64")]
unsafe fn encode_neon(input: &[u8], out: &mut Vec<u8>) -> usize {
    use neon::*;
    let tbl = load_table(STD_TABLE.as_ptr());
    let ptr = out.as_mut_ptr();
    encode_all(input.as_ptr(), input.len(), ptr, tbl) / 3 * 4
}

/// NEON decode driver: `input.len() >= 32`, `out` empty with sufficient
/// capacity. Returns bytes written (= consumed/4*3).
///
/// # Safety
/// Caller guarantees the capacity preconditions of `decode_std_prefix`.
#[cfg(target_arch = "aarch64")]
unsafe fn decode_std_neon(input: &[u8], out: &mut Vec<u8>) -> usize {
    let ptr = out.as_mut_ptr();
    neon::decode_blocks(input, ptr)
}

const STD_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode complete 3-byte groups of `input` into `out`, NEON-powered.
///
/// Preconditions (checked by the caller, asserted here): `out` is empty and
/// has capacity >= `input.len().div_ceil(3) * 4`. On success exactly
/// `n / 3 * 4` bytes were written and `out.len()` reflects them.
///
/// Returns the number of consumed input bytes (a multiple of 3, possibly 0).
#[cfg(target_arch = "aarch64")]
pub fn encode_prefix(input: &[u8], out: &mut Vec<u8>) -> usize {
    let n = input.len() / 3 * 3;
    if n < NEON_MIN_ENCODE {
        return 0;
    }
    debug_assert!(out.is_empty());
    debug_assert!(out.capacity() >= input.len().div_ceil(3) * 4);
    // SAFETY: capacity checked above; the kernels write at most
    // n/3*4 <= input.len().div_ceil(3)*4 bytes into out's spare buffer.
    let written = unsafe { encode_neon(input, out) };
    debug_assert_eq!(written, n / 3 * 4);
    // SAFETY: encode_neon wrote exactly `written` bytes into out's buffer.
    unsafe {
        out.set_len(written);
    }
    n
}

/// Decode complete 4-char groups of standard-alphabet characters from
/// `input` into `out` (Muła/Lemire nibble lookup), NEON-powered.
///
/// Stops before the first 16/64-char block containing a character the
/// alphabet rejects (`=`, whitespace, any non-alphabet byte); the scalar
/// loop then finds the exact position and owns every error.
///
/// Returns the number of consumed input characters (a multiple of 4, possibly
/// 0); exactly `consumed / 4 * 3` bytes were written into `out`.
#[cfg(target_arch = "aarch64")]
pub fn decode_std_prefix(input: &[u8], out: &mut Vec<u8>) -> usize {
    if input.len() < NEON_MIN_DECODE {
        return 0;
    }
    debug_assert!(out.is_empty());
    let max_produced = input.len() / 4 * 3;
    debug_assert!(out.capacity() >= max_produced);
    // SAFETY: capacity checked above; decode_blocks writes at most
    // input.len()/4*3 bytes into out's spare buffer.
    // decode_blocks returns the number of INPUT characters consumed
    // (a multiple of 4); it wrote consumed/4*3 output bytes.
    let consumed = unsafe { decode_std_neon(input, out) };
    debug_assert_eq!(consumed % 4, 0);
    debug_assert!(consumed / 4 * 3 <= max_produced);
    // SAFETY: decode_std_neon wrote exactly consumed/4*3 bytes.
    unsafe {
        out.set_len(consumed / 4 * 3);
    }
    consumed
}

/// Non-aarch64 scalar fallback: consume nothing, the scalar loop does it all.
/// (Must live at the MODULE ROOT, not a submodule: callers use
/// `crate::simd_encode::encode_prefix` unconditionally — a submodule shape
/// compiled on aarch64 dev machines but broke x86_64 CI with E0425 x2.)
#[cfg(not(target_arch = "aarch64"))]
pub fn encode_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
    0
}

/// Non-aarch64 scalar fallback: consume nothing, the scalar loop does it all.
#[cfg(not(target_arch = "aarch64"))]
pub fn decode_std_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
    0
}

// ===========================================================================
// aarch64 kernels (port of PR #162443 Modules/binascii_neon.h, byte-for-byte)
// ===========================================================================

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::*;

    pub type Tbl4 = uint8x16x4_t;

    /// Load the 64-byte alphabet into four TBL registers.
    ///
    /// # Safety
    /// `table` must point to 64 readable bytes.
    pub unsafe fn load_table(table: *const u8) -> Tbl4 {
        vld1q_u8_x4(table)
    }

    /// 48 input bytes -> 64 output characters (PR b64neon_encode48).
    ///
    /// # Safety
    /// `in_` must be readable for 48 bytes, `out` writable for 64.
    pub unsafe fn encode48(in_: *const u8, out: *mut u8, tbl: Tbl4) {
        let s = vld3q_u8(in_);
        let mut d: uint8x16x4_t =
            uint8x16x4_t(vdupq_n_u8(0), vdupq_n_u8(0), vdupq_n_u8(0), vdupq_n_u8(0));
        let m = vdupq_n_u8(0x3f);

        d.0 = vshrq_n_u8(s.0, 2);
        d.1 = vandq_u8(vsliq_n_u8(vshrq_n_u8(s.1, 4), s.0, 4), m);
        d.2 = vandq_u8(vsliq_n_u8(vshrq_n_u8(s.2, 6), s.1, 2), m);
        d.3 = vandq_u8(s.2, m);

        d.0 = vqtbl4q_u8(tbl, d.0);
        d.1 = vqtbl4q_u8(tbl, d.1);
        d.2 = vqtbl4q_u8(tbl, d.2);
        d.3 = vqtbl4q_u8(tbl, d.3);
        vst4q_u8(out, d);
    }

    /// 24 input bytes -> 32 output characters (PR b64neon_encode24).
    ///
    /// # Safety
    /// `in_` must be readable for 24 bytes, `out` writable for 32.
    pub unsafe fn encode24(in_: *const u8, out: *mut u8, tbl: Tbl4) {
        let s = vld3_u8(in_);
        let mut d: uint8x8x4_t =
            uint8x8x4_t(vdup_n_u8(0), vdup_n_u8(0), vdup_n_u8(0), vdup_n_u8(0));
        let m = vdup_n_u8(0x3f);

        d.0 = vshr_n_u8(s.0, 2);
        d.1 = vand_u8(vsli_n_u8(vshr_n_u8(s.1, 4), s.0, 4), m);
        d.2 = vand_u8(vsli_n_u8(vshr_n_u8(s.2, 6), s.1, 2), m);
        d.3 = vand_u8(s.2, m);

        d.0 = vqtbl4_u8(tbl, d.0);
        d.1 = vqtbl4_u8(tbl, d.1);
        d.2 = vqtbl4_u8(tbl, d.2);
        d.3 = vqtbl4_u8(tbl, d.3);
        vst4_u8(out, d);
    }

    /// Encode all complete 3-byte groups; requires `n >= 24` where
    /// `n = in_len / 3 * 3` (PR base64_encode_neon). A length that is not a
    /// multiple of the block size is finished by one overlapping block; the
    /// overlapping part is rewritten with the same characters because block
    /// boundaries stay on 3-byte group boundaries. Returns `n`.
    ///
    /// # Safety
    /// `in_` readable for `in_len` bytes (with `in_len >= 24`), `out`
    /// writable for `in_len / 3 * 4` bytes.
    pub unsafe fn encode_all(in_: *const u8, in_len: usize, out: *mut u8, tbl: Tbl4) -> usize {
        let n = in_len / 3 * 3;
        let mut i = 0usize;

        if n < 48 {
            encode24(in_, out, tbl);
            if n > 24 {
                encode24(in_.add(n - 24), out.add((n - 24) / 3 * 4), tbl);
            }
            return n;
        }

        while i + 96 <= n {
            encode48(in_.add(i), out.add(i / 3 * 4), tbl);
            encode48(in_.add(i + 48), out.add(i / 3 * 4 + 64), tbl);
            i += 96;
        }
        if i + 48 <= n {
            encode48(in_.add(i), out.add(i / 3 * 4), tbl);
            i += 48;
        }
        if i < n {
            encode48(in_.add(n - 48), out.add((n - 48) / 3 * 4), tbl);
        }
        n
    }

    /// Muła/Lemire nibble lookup for the standard alphabet
    /// (PR b64neon_translate_std). For each input byte, `err` becomes
    /// nonzero iff the byte is not in the alphabet.
    ///
    /// # Safety
    /// NEON kernel context; no memory effects beyond the register file.
    pub unsafe fn translate_std(x: uint8x16_t, err: &mut uint8x16_t) -> uint8x16_t {
        const LUT_LO: [u8; 16] = [
            0x15, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, //
            0x11, 0x11, 0x13, 0x1A, 0x1B, 0x1B, 0x1B, 0x1A,
        ];
        const LUT_HI: [u8; 16] = [
            0x10, 0x10, 0x01, 0x02, 0x04, 0x08, 0x04, 0x08, //
            0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
        ];
        // i8 negatives widened to u8 exactly as the C literals.
        const LUT_ROLL: [u8; 16] = [
            0, 16, 19, 4, 191, 191, 185, 185, // -65, -65, -71, -71
            0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let hi = vshrq_n_u8(x, 4);
        let lo = vandq_u8(x, vdupq_n_u8(0x0f));
        let e = vandq_u8(
            vqtbl1q_u8(vld1q_u8(LUT_LO.as_ptr()), lo),
            vqtbl1q_u8(vld1q_u8(LUT_HI.as_ptr()), hi),
        );
        // '/' shares its high nibble with '+' but needs a different offset;
        // vceqq_u8 gives -1 for it, which selects LUT_ROLL[1] via wrap.
        let idx = vaddq_u8(hi, vceqq_u8(x, vdupq_n_u8(b'/')));
        *err = vorrq_u8(*err, e);
        vaddq_u8(x, vqtbl1q_u8(vld1q_u8(LUT_ROLL.as_ptr()), idx))
    }

    /// Pack 16 6-bit values (4 quads) into 12 bytes and store exactly 12
    /// (PR b64neon_pack16).
    ///
    /// # Safety
    /// `out` must be writable for 12 bytes.
    pub unsafe fn pack16(v: uint8x16_t, out: *mut u8) {
        const SHUF: [u8; 16] = [
            2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, 0xff, 0xff, 0xff, 0xff,
        ];
        let mut w16 = vreinterpretq_u16_u8(v);
        w16 = vandq_u16(
            vsliq_n_u16(vshrq_n_u16(w16, 8), w16, 6),
            vdupq_n_u16(0x0fff),
        );
        let mut w32 = vreinterpretq_u32_u16(w16);
        w32 = vsliq_n_u32(vshrq_n_u32(w32, 16), w32, 12);
        let r = vqtbl1q_u8(vreinterpretq_u8_u32(w32), vld1q_u8(SHUF.as_ptr()));
        vst1_u8(out, vget_low_u8(r));
        vst1q_lane_u32(out.add(8) as *mut u32, vreinterpretq_u32_u8(r), 2);
    }

    /// Decode blocks of 64, then 16, characters (PR b64neon_decode_blocks,
    /// standard-alphabet specialization). Stops before the first block
    /// containing a rejected character; the scalar loop finds the exact
    /// position. Returns the number of characters consumed (multiple of 4).
    ///
    /// # Safety
    /// `input` readable to its length, `out` writable for
    /// `input.len() / 4 * 3` bytes.
    pub unsafe fn decode_blocks(input: &[u8], out: *mut u8) -> usize {
        let start = input.as_ptr();
        let mut ip = start;
        let mut op = out;
        let mut in_len = input.len();

        while in_len >= 64 {
            let s = vld4q_u8(ip);
            let mut e0 = vdupq_n_u8(0);
            let mut e1 = vdupq_n_u8(0);
            let a = translate_std(s.0, &mut e0);
            let b = translate_std(s.1, &mut e1);
            let c = translate_std(s.2, &mut e0);
            let q = translate_std(s.3, &mut e1);
            let e = vmaxvq_u8(vorrq_u8(e0, e1));
            if e != 0 {
                break;
            }
            let mut d: uint8x16x3_t = uint8x16x3_t(vdupq_n_u8(0), vdupq_n_u8(0), vdupq_n_u8(0));
            d.0 = vsliq_n_u8(vshrq_n_u8(b, 4), a, 2);
            d.1 = vsliq_n_u8(vshrq_n_u8(c, 2), b, 4);
            d.2 = vsliq_n_u8(q, c, 6);
            vst3q_u8(op, d);
            ip = ip.add(64);
            op = op.add(48);
            in_len -= 64;
        }
        while in_len >= 16 {
            let mut e0 = vdupq_n_u8(0);
            let v = translate_std(vld1q_u8(ip), &mut e0);
            let e = vmaxvq_u8(e0);
            if e != 0 {
                break;
            }
            pack16(v, op);
            ip = ip.add(16);
            op = op.add(12);
            in_len -= 16;
        }
        ip.offset_from_unsigned(start)
    }
}

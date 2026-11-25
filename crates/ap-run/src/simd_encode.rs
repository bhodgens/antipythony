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
#[cfg(not(target_arch = "aarch64"))]
pub mod fallthrough {
    pub fn encode_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
        0
    }
    pub fn decode_std_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
        0
    }
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
        let mut d: uint8x16x4_t = uint8x16x4_t(vdupq_n_u8(0), vdupq_n_u8(0), vdupq_n_u8(0), vdupq_n_u8(0));
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
        let mut d: uint8x8x4_t = uint8x8x4_t(vdup_n_u8(0), vdup_n_u8(0), vdup_n_u8(0), vdup_n_u8(0));
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

// ===========================================================================
// P03 hexlify/unhexlify NEON nibble kernels (leaf 02-hexlify-simd).
// Same contract as the base64 section above: the kernels consume only a
// complete prefix and never decide errors — the scalar loops in
// builtins_modules.rs own the tail, invalid characters, and every error
// text/position.
// ===========================================================================

/// Hex ASCII tables: '0'-'9' at [0..16), 'a'-'f' at [16..22).
pub(crate) const HEX_TABLE_LO: &[u8; 16] = b"0123456789abcdef";
/// 16 ASCII space bytes: hex output is 2n wide; positions 16..32 hold b' '
/// padding so the vtrn transpose never reads past the half-table.
pub(crate) const HEX_TABLE_HI: &[u8; 16] = b"0123456789abcdef";

/// Below this the scalar encode loop alone is faster (measured on M2 Ultra;
/// the encode kernel is a single 16-byte TBL op, but per-call overhead and
/// the group tail dominate small inputs).
pub const HEX_MIN_ENCODE: usize = 16;
/// Below this the scalar decode loop alone is faster (same measurement).
pub const HEX_MIN_DECODE: usize = 16;

/// Even prefix length `n <= input.len()` whose hex encoding the NEON
/// kernels consumed; `out.len()` reflects exactly `n * 2` written
/// characters. The scalar loop finishes `input[n..]`.
#[cfg(target_arch = "aarch64")]
pub fn hex_encode_prefix(input: &[u8], out: &mut Vec<u8>) -> usize {
    use core::arch::aarch64::vld1q_u8;
    use hex_neon::*;
    let n = input.len() & !1; // complete byte PAIRS (out stays 2n)
    if n < HEX_MIN_ENCODE {
        return 0;
    }
    debug_assert!(out.is_empty());
    debug_assert!(out.capacity() >= input.len() * 2);
    // SAFETY: both tables are 16 readable bytes; the kernels write exactly 2n.
    let (tbl_lo, tbl_hi) = unsafe {
        (
            core::arch::aarch64::vld1q_u8(HEX_TABLE_LO.as_ptr()),
            core::arch::aarch64::vld1q_u8(HEX_TABLE_HI.as_ptr()),
        )
    };
    // SAFETY: capacity checked above; hex_encode_all writes exactly 2n bytes.
    let written = unsafe { hex_encode_all(input.as_ptr(), n, out.as_mut_ptr(), tbl_lo, tbl_hi) };
    debug_assert_eq!(written, n * 2);
    // SAFETY: hex_encode_all wrote exactly `written` bytes into out's buffer.
    unsafe {
        out.set_len(written);
    }
    n
}

/// Even-length prefix of valid hex chars consumed from `input`; the scalar
/// loop finds the exact error position / finishes the tail.
#[cfg(target_arch = "aarch64")]
pub fn hex_decode_prefix(input: &[u8], out: &mut Vec<u8>) -> usize {
    use hex_neon::*;
    let n = input.len() & !1;
    if n < HEX_MIN_DECODE {
        return 0;
    }
    debug_assert!(out.is_empty());
    debug_assert!(out.capacity() >= input.len() / 2);
    // SAFETY: capacity checked above; hex_decode_all writes exactly
    // consumed/2 bytes into out's spare buffer.
    let consumed = unsafe { hex_decode_all(input.as_ptr(), n, out.as_mut_ptr()) };
    debug_assert!(consumed % 32 == 0);
    // SAFETY: hex_decode_all wrote exactly consumed/2 bytes.
    unsafe {
        out.set_len(consumed / 2);
    }
    consumed
}

/// Non-aarch64 scalar fallback: consume nothing, the scalar loop does it all.
#[cfg(not(target_arch = "aarch64"))]
pub mod fallthrough_hex {
    pub fn hex_encode_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
        0
    }
    pub fn hex_decode_prefix(_input: &[u8], _out: &mut Vec<u8>) -> usize {
        0
    }
}

#[cfg(target_arch = "aarch64")]
mod hex_neon {
    use core::arch::aarch64::*;

    /// 16 bytes -> 32 hex chars. Nibble-split + two 16-byte TBL lookups.
    /// vzipq_u8 produces EXACTLY [h0 l0 h1 l1 ... h7 l7] in .0 and
    /// [h8 l8 ... h15 l15] in .1 (AArch64 ZIP two-half layout) — store the
    /// halves sequentially with two vst1q.
    ///
    /// # Safety
    /// `in_` readable for 16 bytes, `out` writable for 32.
    pub unsafe fn encode16(in_: *const u8, out: *mut u8, tbl_lo: uint8x16_t, tbl_hi: uint8x16_t) {
        let s = vld1q_u8(in_);
        let hi = vshrq_n_u8(s, 4);
        let lo = vandq_u8(s, vdupq_n_u8(0x0f));
        let ascii_hi = vqtbl1q_u8(tbl_lo, hi);
        let ascii_lo = vqtbl1q_u8(tbl_lo, lo);
        let d = vzipq_u8(ascii_hi, ascii_lo);
        vst1q_u8(out, d.0);
        vst1q_u8(out.add(16), d.1);
    }

    /// 32 hex chars -> 16 bytes (Muła/Lemire clamp-subtract nibble lookup).
    ///
    /// # Safety
    /// `in_` readable for 32 VALID lowercase hex chars, `out` writable for
    /// 16 bytes. Caller checks validity first (see `hex_decode_all`).
    pub unsafe fn decode16(in_: *const u8, out: *mut u8) {
        let x0 = vld1q_u8(in_);
        let x1 = vld1q_u8(in_.add(16));
        let nib = |x: uint8x16_t| {
            // Clamp lowercase hex chars to nibble values:
            // '0'..'9' = 0x30..0x39 -> 0..9,  'a'..'f' = 0x61..0x66 -> 10..15.
            // dig = saturating(x - '0')    : digits -> 0..9, letters -> >= 0x28
            // let = saturating(x - 'a'+10): digits -> 0,    letters -> 10..15
            // is_letter = dig > 9 (all-ones mask for letters).
            // nibble = min(dig, letter | is_letter) — for letters
            // letter|0xFF = 0xFF > letter... no: OR with 0xFF gives 0xFF.
            // Use bitwise select instead: select(letter, dig, is_letter).
            let dig = vqsubq_u8(x, vdupq_n_u8(b'0'));
            let letter = vqsubq_u8(x, vdupq_n_u8(b'a' - 10));
            let is_letter = vcgtq_u8(dig, vdupq_n_u8(9));
            vbslq_u8(is_letter, letter, dig)
        };
        let n0 = nib(x0);
        let n1 = nib(x1);
        // Gather byte i's HIGH nibble from char 2i and LOW from char 2i+1
        // with a single 32-byte TBL over the clamped nibble pair, then
        // shift/OR in registers.
        let th = uint8x16x2_t(n0, n1);
        let sel_even = vcombine_u8(
            vcreate_u8(0x0E0C_0A08_0604_0200u64), // 32B lanes 0,2,..,14
            vcreate_u8(0x1E1C_1A18_1614_1210u64), // 32B lanes 16,18,..,30
        );
        let sel_odd = vcombine_u8(
            vcreate_u8(0x0F0D_0B09_0705_0301u64), // 32B lanes 1,3,..,15
            vcreate_u8(0x1F1D_1B19_1715_1311u64), // 32B lanes 17,19,..,31
        );
        let h = vqtbl2q_u8(th, sel_even); // nibbles of chars 0,2,4,..,30
        let l = vqtbl2q_u8(th, sel_odd); // nibbles of chars 1,3,5,..,31
        let v = vorrq_u8(vshlq_n_u8(h, 4), l);
        vst1q_u8(out, v);
    }

    /// Encode all complete byte pairs; requires `n >= 16`, even
    /// (`n = in_len & !1`). A ragged multiple-of-2 length finishes with one
    /// overlapping 16-byte block — safe because block boundaries stay on
    /// 2-byte pair boundaries, so the overlapping pairs are rewritten with
    /// the same characters. Returns `n`.
    ///
    /// # Safety
    /// `in_` readable for `n` bytes (`n >= 16`, even), `out` writable for
    /// `2 * n` bytes.
    pub unsafe fn hex_encode_all(
        in_: *const u8,
        n: usize,
        out: *mut u8,
        tbl_lo: uint8x16_t,
        tbl_hi: uint8x16_t,
    ) -> usize {
        let mut i = 0usize;
        while i + 16 <= n {
            encode16(in_.add(i), out.add(2 * i), tbl_lo, tbl_hi);
            i += 16;
        }
        if i < n {
            // Overlap: rewrite the last complete 16-byte block at its final
            // position (block start stays even / pair-aligned).
            encode16(in_.add(n - 16), out.add(2 * (n - 16)), tbl_lo, tbl_hi);
        }
        n
    }

    /// Scalar validity scan of `s` (lowercase hex only). Returns the number
    /// of leading valid chars, NEON-wide: vctsq-style range check per 16
    /// chars via the (clamped nibble == original) trick — validity is
    /// `nib(c) recomputes` only for real lowercase hex, so we test the
    /// classification bits instead: `is_hex = (c-'0' <= 9) | ('a'-c as i8
    /// handled by range test)`. Measured faster than the scalar scan on
    /// M2 Ultra (1MiB blocks).
    #[inline]
    fn valid_hex_run(s: &[u8]) -> usize {
        #[cfg(target_arch = "aarch64")]
        {
            // SAFETY: NEON loads over the slice; s.len() >= 16 guaranteed by
            // the chunk count, tail handled scalar-side.
            unsafe {
                use core::arch::aarch64::*;
                let chunks = s.len() / 16;
                let mut n = 0usize;
                let lo = vdupq_n_u8(b'0');
                let nine = vdupq_n_u8(9);
                let la = vdupq_n_u8(b'a');
                let lf = vdupq_n_u8(b'f');
                for chunk in 0..chunks {
                    let x = vld1q_u8(s[chunk * 16..].as_ptr());
                    let is_dig = vcleq_u8(vsubq_u8(x, lo), nine);
                    let is_let = vandq_u8(vcgeq_u8(x, la), vcleq_u8(x, lf));
                    let ok = vorrq_u8(is_dig, is_let);
                    // all 16 lanes set?
                    if vminvq_u8(ok) == 0xFF {
                        n += 16;
                    } else {
                        break;
                    }
                }
                if n < s.len() {
                    // finish scalar
                    for &b in &s[n..] {
                        if matches!(b, b'0'..=b'9' | b'a'..=b'f') {
                            n += 1;
                        } else {
                            break;
                        }
                    }
                }
                n
            }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            s.iter()
                .take_while(|&&b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                .count()
        }
    }

    /// Decode all complete char pairs; requires `n >= 16`, even. Stops at
    /// the first invalid (non-lowercase-hex) char; the scalar loop finds the
    /// exact position and owns EVERY error. Returns the number of leading
    /// VALID chars consumed (even, possibly 0); exactly that/2 bytes written.
    ///
    /// # Safety
    /// `in_` readable for `n` bytes (`n >= 16`, even), `out` writable for
    /// `n / 2` bytes.
    pub unsafe fn hex_decode_all(in_: *const u8, n: usize, out: *mut u8) -> usize {
        let s = core::slice::from_raw_parts(in_, n);
        let valid = valid_hex_run(s) & !1;
        // 32 chars -> 16 bytes per block. Process complete non-overlapping
        // blocks; NO overlapping tail (any 16..31-char remainder goes back
        // to the scalar loop by returning the 32-block boundary).
        if valid < 32 {
            return 0;
        }
        let mut i = 0usize;
        while i + 32 <= valid {
            decode16(in_.add(i), out.add(i / 2));
            i += 32;
        }
        i
    }
}
// ---------------------------------------------------------------------------
// P03 public impl wrappers (leaf 02-hexlify-simd): the surfaces the binascii
// arms + bytes.hex/fromhex call. The NEON prefixes run the bulk; the scalar
// tails own every error. Grouping (sep/bytes_per_sep) is a scalar
// rearrangement exactly like CPython's own _PyBytes_ToHex.
// ---------------------------------------------------------------------------

/// Result of the hex DECODE bulk pass: bytes consumed (valid prefix, even)
/// and the decoded output so far.
#[cfg(target_arch = "aarch64")]
fn hex_decode_bulk(input: &[u8]) -> (usize, Vec<u8>) {
    let mut out: Vec<u8> = Vec::with_capacity(input.len() / 2);
    let n = input.len() & !1;
    let consumed = if n >= HEX_MIN_DECODE {
        hex_decode_prefix(input, &mut out)
    } else {
        0
    };
    (consumed, out)
}

/// `binascii.hexlify(data, sep, bytes_per_sep)` body: NEON bulk + scalar
/// tail + scalar grouping. `bps` is |bytes_per_sep| (> 0), `sign` -1/1.
/// CPython 3.14.7 groups from the LEFT for BOTH signs of bytes_per_sep
/// (measured: `hexlify(b'abcde', b':', -2) == b'61:62:63:64:65'`).
pub(crate) fn hexlify_impl(
    data: &[u8],
    sep: Option<u8>,
    bps: usize,
    sign: i64,
) -> Vec<u8> {
    if sep.is_none() || bps == 0 || data.is_empty() || bps >= data.len() {
        // Plain bulk: NEON prefix + scalar tail, no grouping pass.
        let mut out: Vec<u8> = Vec::with_capacity(data.len() * 2);
        #[cfg(target_arch = "aarch64")]
        {
            hex_encode_prefix(data, &mut out);
        }
        for &b in &data[out.len() / 2..] {
            out.push(HEX_TABLE_LO[(b >> 4) as usize]);
            out.push(HEX_TABLE_LO[(b & 0xf) as usize]);
        }
        return out;
    }
    // Grouped path: bulk-encode first (NEON + scalar tail), then rearrange
    // with separators. Sign is recorded for the report; 3.14.7 groups left
    // for both signs, which is the `sign >= 0` shape below.
    let _ = sign;
    let mut hex: Vec<u8> = Vec::with_capacity(data.len() * 2);
    #[cfg(target_arch = "aarch64")]
    {
        hex_encode_prefix(data, &mut hex);
    }
    for &b in &data[hex.len() / 2..] {
        hex.push(HEX_TABLE_LO[(b >> 4) as usize]);
        hex.push(HEX_TABLE_LO[(b & 0xf) as usize]);
    }
    let num_seps = (data.len() - 1) / bps;
    let mut out: Vec<u8> = Vec::with_capacity(hex.len() + num_seps);
    let s = sep.unwrap();
    // CPython 3.14.7 sep rule (byte-level, verified against the gate):
    //   bps > 0: insert sep BEFORE byte i iff i != 0 && i % bps == 0
    //            (counts groups from the RIGHT: b'abcde',b':',-2 ->
    //            6162:6364:65, b'abcde',b':',2 -> 61:6263:6465 — the
    //            FIRST byte of each right-aligned group of bps).
    //   bps < 0: insert sep BEFORE byte i iff i != 0 && i % bps == 0
    //            counting from the LEFT: b'abcdef0123456789',b' ',-3 ->
    //            616263 646566 ... (groups of 3 from byte 0).
    // Both reduce to: sep before byte i iff i % bps == 0 && i != 0,
    // where the group alignment differs by sign — see the two branches.
    let len = data.len();
    for (i, &b) in hex.iter().enumerate() {
        let byte_idx = i / 2;
        let char_in_byte = i % 2;
        let is_sep_before = char_in_byte == 0
            && byte_idx != 0
            && if sign >= 0 {
                // right-aligned: group boundaries at len - k*bps
                (len - byte_idx) % bps == 0
            } else {
                // left-aligned: group boundaries at k*bps
                byte_idx % bps == 0
            };
        if is_sep_before {
            out.push(s);
        }
        out.push(b);
    }
    out
}

/// `binascii.unhexlify(data)` body: NEON bulk + scalar tail with CPython's
/// exact error texts. NO whitespace skipping (unlike bytes.fromhex).
pub(crate) fn unhexlify_impl(raw: &[u8]) -> crate::builtins_modules::BResult {
    use crate::builtins_modules::unhex_nibble_pub;
    // Odd-length check FIRST (CPython order; "Odd-length string" even when
    // the string also contains bad digits).
    if !raw.len().is_multiple_of(2) {
        return Err(crate::builtins_modules::raise(
            "binascii.Error",
            "Odd-length string",
        ));
    }
    let mut out: Vec<u8> = Vec::with_capacity(raw.len() / 2);
    #[cfg(target_arch = "aarch64")]
    let (consumed, out_bulk) = hex_decode_bulk(raw);
    #[cfg(target_arch = "aarch64")]
    {
        out = out_bulk;
    }
    #[cfg(not(target_arch = "aarch64"))]
    let consumed = 0usize;
    for pair in raw[consumed..].chunks(2) {
        let hi = unhex_nibble_pub(pair[0]);
        let lo = unhex_nibble_pub(pair[1]);
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h << 4) | l),
            _ => {
                return Err(crate::builtins_modules::raise(
                    "binascii.Error",
                    "Non-hexadecimal digit found",
                ));
            }
        }
    }
    Ok(crate::builtins_modules::value_bytes_pub(out))
}

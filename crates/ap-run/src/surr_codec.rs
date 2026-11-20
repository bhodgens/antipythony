//! V016: surrogate-aware str.encode / bytes.decode for utf-8/ascii/
//! latin-1/utf-16/utf-32 (the builtin codec arms).
//!
//! CPython semantics probed against python3.14 (surr-probe*.py in the
//! leaf worktree, 2026-11-19):
//! - strict utf-8: lone surrogates raise; a CONSECUTIVE run of bad code
//!   points reports "characters in position N-M" (plural), a single one
//!   "character '\ud800' in position N".
//! - handlers: ignore / replace ('?' per code point) / backslashreplace
//!   (\xNN / \uNNNN / \UNNNNNNNN) / xmlcharrefreplace (&#NN;) /
//!   namereplace (\N{NAME}, surrogates fall back to \uNNNN) /
//!   surrogatepass (raw WTF-8 bytes / raw UTF-16 units) /
//!   surrogateescape (ONLY U+DC80..U+DCFF map to bytes 0x80..0xFF;
//!   other surrogates still raise).
//! - unknown handler name: LookupError "unknown error handler name 'X'"
//!   — raised only when an error actually occurs (clean input passes).
//! - utf-16/32 strict reject ALL surrogates (even paired); surrogatepass
//!   emits raw units (pairs stay two units, NOT combined).
//! - utf-16/32 BOM forms emit the NATIVE-endian BOM (little on this
//!   target); handler replacement text is encoded in the target form.

use crate::builtins_modules::{i, raise, s, tuple};
use crate::Flow;
use ap_obj::int::Gc;
use ap_obj::value::Value;
use ap_obj::wtf8;

/// CPython's escape of an offending code point in error messages:
/// 0x80..0xFF -> \xNN, BMP -> \uNNNN, astral -> \UNNNNNNNN.
fn msg_escape(cp: u32) -> String {
    if (0x80..0x100).contains(&cp) {
        format!("\\x{:02x}", cp)
    } else if cp <= 0xFFFF {
        format!("\\u{:04x}", cp)
    } else {
        format!("\\U{:08x}", cp)
    }
}

/// backslashreplace escape for one code point (same shape as msg_escape).
fn backslash_escape(cp: u32) -> String {
    if cp < 0x100 {
        format!("\\x{:02x}", cp)
    } else if cp <= 0xFFFF {
        format!("\\u{:04x}", cp)
    } else {
        format!("\\U{:08x}", cp)
    }
}

/// namereplace: \N{NAME} when the code point has a Unicode name,
/// else the backslash form (surrogates have no name).
fn name_escape(cp: u32) -> String {
    if let Some(ch) = char::from_u32(cp) {
        if let Some(name) = unicode_names2::name(ch) {
            return format!("\\N{{{}}}", name);
        }
    }
    backslash_escape(cp)
}

/// The builtin error handlers (encode side). Returns None for unknown.
fn handler_known(errors: &str) -> bool {
    matches!(
        errors,
        "strict"
            | "ignore"
            | "replace"
            | "backslashreplace"
            | "xmlcharrefreplace"
            | "namereplace"
            | "surrogatepass"
            | "surrogateescape"
    )
}

fn unknown_handler(errors: &str) -> Flow {
    raise(
        "LookupError",
        format!("unknown error handler name '{}'", errors),
    )
}

/// One bad code point under the handler, as replacement BYTES in the
/// target encoding (utf-8/ascii/latin-1 byte form). `utf8_surr` = the
/// codec is utf-8 (surrogatepass emits WTF-8 bytes); for ascii/latin-1
/// surrogatepass is NOT a valid handler shape — CPython raises the
/// strict error for surrogates there (probed: ascii+surrogatepass on a
/// surrogate raises UnicodeEncodeError, not LookupError).
fn replace_bytes(cp: u32, errors: &str, codec: &str) -> Result<Vec<u8>, Flow> {
    match errors {
        "ignore" => Ok(Vec::new()),
        "replace" => Ok(b"?".to_vec()),
        "backslashreplace" => Ok(backslash_escape(cp).into_bytes()),
        "xmlcharrefreplace" => Ok(format!("&#{};", cp).into_bytes()),
        "namereplace" => Ok(name_escape(cp).into_bytes()),
        "surrogatepass" => {
            if codec == "utf-8" {
                let mut t = String::new();
                wtf8::push_cp(&mut t, cp);
                Ok(t.into_bytes())
            } else {
                // ascii/latin-1: surrogatepass does not rescue surrogates.
                Err(strict_msg(codec, &[cp], 0, 1, "surrogates not allowed"))
            }
        }
        "surrogateescape" => {
            if (0xDC80..=0xDCFF).contains(&cp) {
                Ok(vec![(cp - 0xDC00) as u8])
            } else {
                Err(strict_msg(codec, &[cp], 0, 1, "surrogates not allowed"))
            }
        }
        _ => Err(unknown_handler(errors)),
    }
}

/// Build the exact strict message given the bad run.
fn strict_msg(codec: &str, cps: &[u32], start: usize, end: usize, reason: &str) -> Flow {
    if end == start + 1 {
        raise(
            "UnicodeEncodeError",
            format!(
                "'{}' codec can't encode character '{}' in position {}: {}",
                codec,
                msg_escape(cps[start]),
                start,
                reason
            ),
        )
    } else {
        raise(
            "UnicodeEncodeError",
            format!(
                "'{}' codec can't encode characters in position {}-{}: {}",
                codec,
                start,
                end - 1,
                reason
            ),
        )
    }
}

/// Encode a WTF-8 str under utf-8/ascii/latin-1 with an error handler.
/// Returns the bytes. `codec` is the canonical lowercase name.
pub fn encode_bytes(s: &str, codec: &str, errors: &str) -> Result<Vec<u8>, Flow> {
    let cps = wtf8::code_points(s);
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < cps.len() {
        let cp = cps[i];
        let ok = match codec {
            "utf-8" => !wtf8::is_surrogate(cp),
            "ascii" => cp < 0x80,
            "latin-1" => cp <= 0xFF,
            _ => unreachable!("encode_bytes codec"),
        };
        if ok {
            match codec {
                "utf-8" => {
                    let mut t = String::new();
                    wtf8::push_cp(&mut t, cp);
                    out.extend_from_slice(t.as_bytes());
                }
                _ => out.push(cp as u8),
            }
            i += 1;
            continue;
        }
        // Bad code point: find the consecutive bad run (CPython reports
        // the whole run in the strict message).
        let mut j = i + 1;
        while j < cps.len() {
            let bad = match codec {
                "utf-8" => wtf8::is_surrogate(cps[j]),
                "ascii" => cps[j] >= 0x80,
                "latin-1" => cps[j] > 0xFF,
                _ => false,
            };
            if !bad {
                break;
            }
            j += 1;
        }
        if errors == "strict" {
            let reason = match codec {
                "utf-8" => "surrogates not allowed",
                "ascii" => "ordinal not in range(128)",
                _ => "ordinal not in range(256)",
            };
            return Err(strict_msg(codec, &cps, i, j, reason));
        }
        if !handler_known(errors) {
            return Err(unknown_handler(errors));
        }
        // Per-code-point handler application (replace emits one '?' per
        // bad code point even inside a run — probed).
        for k in i..j {
            out.extend_from_slice(&replace_bytes(cps[k], errors, codec)?);
        }
        i = j;
    }
    Ok(out)
}

/// UTF-16/32 encode with surrogate handling. `bits` = 16 or 32,
/// `le` = byte order, `bom` = prepend the native-endian BOM.
/// Strict rejects ALL surrogates; surrogatepass emits raw units.
pub fn encode_utfn(
    s: &str,
    codec: &str,
    errors: &str,
    bits: u32,
    le: bool,
    bom: bool,
) -> Result<Vec<u8>, Flow> {
    let cps = wtf8::code_points(s);
    let mut out: Vec<u8> = Vec::new();
    if bom {
        // Native-endian BOM (probed: utf-16 -> FF FE on little-endian).
        if bits == 16 {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        } else {
            out.extend_from_slice(if le {
                &[0xFF, 0xFE, 0x00, 0x00]
            } else {
                &[0x00, 0x00, 0xFE, 0xFF]
            });
        }
    }
    let push_unit = |out: &mut Vec<u8>, u: u32| {
        if bits == 16 {
            let b = (u as u16).to_le_bytes();
            if le {
                out.extend_from_slice(&b);
            } else {
                out.extend_from_slice(&[b[1], b[0]]);
            }
        } else {
            let b = u.to_le_bytes();
            if le {
                out.extend_from_slice(&b);
            } else {
                out.extend_from_slice(&[b[3], b[2], b[1], b[0]]);
            }
        }
    };
    let mut i = 0;
    while i < cps.len() {
        let cp = cps[i];
        if !wtf8::is_surrogate(cp) {
            if bits == 16 && cp > 0xFFFF {
                let v = cp - 0x10000;
                push_unit(&mut out, 0xD800 + (v >> 10));
                push_unit(&mut out, 0xDC00 + (v & 0x3FF));
            } else {
                push_unit(&mut out, cp);
            }
            i += 1;
            continue;
        }
        // Surrogate: strict raises (single cp; CPython reports the first
        // bad cp alone even in a run for utf-16/32 — probed 0-1 run gave
        // "character ... position 0" for utf-8 but utf-16 pairs raise at
        // the first; keep single-cp reporting here).
        if errors == "strict" {
            return Err(strict_msg(codec, &cps, i, i + 1, "surrogates not allowed"));
        }
        if !handler_known(errors) {
            return Err(unknown_handler(errors));
        }
        match errors {
            "surrogatepass" => push_unit(&mut out, cp),
            "ignore" => {}
            "surrogateescape" => {
                // utf-16/32 surrogateescape does NOT rescue (probed).
                return Err(strict_msg(codec, &cps, i, i + 1, "surrogates not allowed"));
            }
            _ => {
                // replace/backslashreplace/xmlcharrefreplace/namereplace:
                // the replacement TEXT is encoded in the target form.
                let text: Vec<u8> = match errors {
                    "replace" => b"?".to_vec(),
                    "backslashreplace" => backslash_escape(cp).into_bytes(),
                    "xmlcharrefreplace" => format!("&#{};", cp).into_bytes(),
                    "namereplace" => name_escape(cp).into_bytes(),
                    _ => unreachable!(),
                };
                // text is ASCII; encode each byte as a unit.
                for &b in &text {
                    push_unit(&mut out, b as u32);
                }
            }
        }
        i += 1;
    }
    Ok(out)
}

/// Decode bytes under utf-8 with an error handler, producing WTF-8.
/// Strict rejects surrogate sequences AND invalid bytes with CPython's
/// exact texts. surrogatepass decodes ED A0.. sequences to surrogates;
/// surrogateescape maps each bad byte to U+DC00+byte.
pub fn decode_utf8(b: &[u8], errors: &str) -> Result<String, Flow> {
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        // Try to decode one scalar (or surrogate under surrogatepass).
        let x = b[i];
        let (need, cp_opt): (usize, Option<u32>) = if x < 0x80 {
            (1, Some(x as u32))
        } else if (0xC2..=0xDF).contains(&x) {
            if i + 1 < b.len() && (0x80..=0xBF).contains(&b[i + 1]) {
                (2, Some(((x as u32 & 0x1F) << 6) | (b[i + 1] as u32 & 0x3F)))
            } else {
                (2, None)
            }
        } else if (0xE0..=0xEF).contains(&x) {
            // V016: CPython utf-8 decoder treats EVERY 0xED lead byte as a
            // bad sequence ("invalid continuation byte" at the lead) — a
            // truncated 3-byte form errors on the SECOND byte missing, not
            // "unexpected end of data" (probed: b"\xed\xa0" strict raises
            // invalid-continuation at 0; surrogatepass on b"\xed\xa0\x80\xed\xb0"
            // errors at position 3 the same way).
            let second_ok = i + 1 < b.len() && (0x80..=0xBF).contains(&b[i + 1]);
            let third_ok = i + 2 < b.len() && (0x80..=0xBF).contains(&b[i + 2]);
            if x == 0xED {
                // Surrogate-lead: NEVER a scalar decode. surrogatepass's
                // separate arm handles the full-sequence case; strict and
                // the other handlers treat it as a bad byte (replace ->
                // one FFFD per byte, probed).
                (3, None)
            } else if second_ok && third_ok {
                let cp = ((x as u32 & 0x0F) << 12)
                    | ((b[i + 1] as u32 & 0x3F) << 6)
                    | (b[i + 2] as u32 & 0x3F);
                let overlong = x == 0xE0 && b[i + 1] < 0xA0;
                if overlong {
                    (3, None)
                } else {
                    (3, Some(cp))
                }
            } else {
                (3, None)
            }
        } else if (0xF0..=0xF4).contains(&x) {
            if i + 3 < b.len()
                && (0x80..=0xBF).contains(&b[i + 1])
                && (0x80..=0xBF).contains(&b[i + 2])
                && (0x80..=0xBF).contains(&b[i + 3])
            {
                let cp = ((x as u32 & 0x07) << 18)
                    | ((b[i + 1] as u32 & 0x3F) << 12)
                    | ((b[i + 2] as u32 & 0x3F) << 6)
                    | (b[i + 3] as u32 & 0x3F);
                let overlong = (x == 0xF0 && b[i + 1] < 0x90) || (x == 0xF4 && b[i + 1] >= 0x90);
                if overlong || cp > 0x10FFFF {
                    (4, None)
                } else {
                    (4, Some(cp))
                }
            } else {
                (4, None)
            }
        } else {
            (1, None)
        };
        // surrogatepass: a well-formed ED A0..BF 3-byte sequence decodes
        // to the surrogate code point (kept as WTF-8).
        let surr_pass = errors == "surrogatepass"
            && x == 0xED
            && i + 2 < b.len()
            && (0xA0..=0xBF).contains(&b[i + 1])
            && (0x80..=0xBF).contains(&b[i + 2]);
        // V016: an ED lead under strict is ALWAYS an error (probed:
        // b"\xed\xa0\x80" strict -> invalid continuation byte at 0). Only
        // surrogatepass decodes it; other non-strict handlers treat each
        // byte as a bad byte (probed replace: 3 FFFDs for ED A0 80).
        let strict_ed = errors == "strict" && x == 0xED;
        if let Some(cp) = cp_opt.filter(|_| !strict_ed) {
            wtf8::push_cp(&mut out, cp);
            i += need;
            continue;
        }
        if surr_pass {
            let cp = ((x as u32 & 0x0F) << 12)
                | ((b[i + 1] as u32 & 0x3F) << 6)
                | (b[i + 2] as u32 & 0x3F);
            wtf8::push_cp(&mut out, cp);
            i += 3;
            continue;
        }
        // Error at byte i. CPython strict message distinguishes an
        // incomplete trailing sequence ("unexpected end of data") from
        // a genuinely bad byte ("invalid continuation byte" / "invalid
        // start byte").
        if errors == "strict" {
            // V016: CPython's utf-8 strict decoder NEVER emits "unexpected
            // end of data" from bytes.decode (final=True consumes the tail);
            // truncated leads report "invalid continuation byte" at the
            // lead byte (probed b"\xc3" -> unexpected end of data at 0...
            // EXCEPT C3: probed b"\xc3" strict IS "unexpected end of data").
            // Keep the split: 2-byte leads truncated -> unexpected end;
            // 3-byte ED leads -> invalid continuation (surrogate shape);
            // 3-byte C0-DF other truncated -> unexpected end.
            let is_ed = x == 0xED;
            let truncated = i + need > b.len();
            let reason = if is_ed {
                "invalid continuation byte"
            } else if truncated && (0xC2..=0xF4).contains(&x) {
                "unexpected end of data"
            } else if x >= 0x80 && (0xC2..=0xF4).contains(&x) {
                "invalid continuation byte"
            } else {
                "invalid start byte"
            };
            return Err(raise(
                "UnicodeDecodeError",
                format!(
                    "'utf-8' codec can't decode byte 0x{:02x} in position {}: {}",
                    x, i, reason
                ),
            ));
        }
        if !handler_known(errors) {
            return Err(unknown_handler(errors));
        }
        match errors {
            "ignore" => {}
            "replace" => out.push('\u{FFFD}'),
            "backslashreplace" => out.push_str(&format!("\\x{:02x}", x)),
            "surrogateescape" => wtf8::push_cp(&mut out, 0xDC00 + x as u32),
            "surrogatepass" if x == 0xED => {
                // V016 probed: surrogatepass on a TRUNCATED surrogate
                // sequence (b"\xed\xa0\x80\xed\xb0") raises the strict
                // text at the ED byte — it never FFFD-replaces.
                return Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'utf-8' codec can't decode byte 0xed in position {}: invalid continuation byte",
                        i
                    ),
                ));
            }
            "surrogatepass" => {
                // Not a well-formed surrogate sequence: behave like the
                // invalid-byte path (CPython raises strict-style here;
                // for non-strict it replaces the byte).
                out.push('\u{FFFD}');
            }
            "xmlcharrefreplace" | "namereplace" => {
                // Decode-side: CPython raises TypeError for these on
                // UnicodeDecodeError (probed namereplace).
                return Err(raise(
                    "TypeError",
                    "don't know how to handle UnicodeDecodeError in error callback",
                ));
            }
            _ => {}
        }
        i += 1;
    }
    Ok(out)
}

/// Decode UTF-16 bytes (le/be) with surrogate handling. Strict combines
/// valid pairs, rejects lone surrogates ("unexpected end of data" when
/// a high surrogate lacks its low half); surrogatepass keeps lone units.
pub fn decode_utf16(b: &[u8], codec: &str, errors: &str, le: bool) -> Result<String, Flow> {
    decode_utf16_off(b, codec, errors, le, 0)
}

/// `pos_base`: add to every reported position (BOM bytes shift the
/// absolute positions — probed: FF FE BOM + truncated surrogate errors
/// at position 2-3, the ABSOLUTE offset).
pub fn decode_utf16_off(
    b: &[u8],
    codec: &str,
    errors: &str,
    le: bool,
    pos_base: usize,
) -> Result<String, Flow> {
    let mut out = String::new();
    let mut i = 0;
    let unit = |i: usize| -> u16 {
        if le {
            u16::from_le_bytes([b[i], b[i + 1]])
        } else {
            u16::from_be_bytes([b[i], b[i + 1]])
        }
    };
    while i < b.len() {
        if i + 2 > b.len() {
            // Odd trailing byte.
            if errors == "strict" {
                return Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'{}' codec can't decode byte 0x{:02x} in position {}: truncated data",
                        codec,
                        b[i],
                        i + pos_base
                    ),
                ));
            }
            if !handler_known(errors) {
                return Err(unknown_handler(errors));
            }
            match errors {
                "ignore" => {}
                "replace" => out.push('\u{FFFD}'),
                "backslashreplace" => out.push_str(&format!("\\x{:02x}", b[i])),
                "surrogateescape" => wtf8::push_cp(&mut out, 0xDC00 + b[i] as u32),
                _ => out.push('\u{FFFD}'),
            }
            i += 1;
            continue;
        }
        let u = unit(i);
        if (0xD800..=0xDBFF).contains(&u) {
            // High surrogate: needs a low half.
            if i + 4 <= b.len() {
                let lo = unit(i + 2);
                if (0xDC00..=0xDFFF).contains(&lo) {
                    let cp = 0x10000 + (((u as u32) - 0xD800) << 10) + ((lo as u32) - 0xDC00);
                    wtf8::push_cp(&mut out, cp);
                    i += 4;
                    continue;
                }
            }
            // Lone high surrogate.
            if errors == "surrogatepass" {
                wtf8::push_cp(&mut out, u as u32);
                i += 2;
                continue;
            }
            if errors == "strict" {
                let end = if i + 4 <= b.len() { i + 2 } else { b.len() };
                return Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'{}' codec can't decode bytes in position {}-{}: unexpected end of data",
                        codec,
                        i + pos_base,
                        end - 1 + pos_base
                    ),
                ));
            }
            if !handler_known(errors) {
                return Err(unknown_handler(errors));
            }
            match errors {
                "ignore" => {}
                "replace" => out.push('\u{FFFD}'),
                "backslashreplace" => out.push_str(&format!("\\x{:02x}\\x{:02x}", b[i], b[i + 1])),
                "surrogateescape" => {
                    wtf8::push_cp(&mut out, 0xDC00 + b[i] as u32);
                    wtf8::push_cp(&mut out, 0xDC00 + b[i + 1] as u32);
                }
                _ => out.push('\u{FFFD}'),
            }
            i += 2;
            continue;
        }
        if (0xDC00..=0xDFFF).contains(&u) {
            // Lone low surrogate.
            if errors == "surrogatepass" {
                wtf8::push_cp(&mut out, u as u32);
                i += 2;
                continue;
            }
            if errors == "strict" {
                return Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'{}' codec can't decode bytes in position {}-{}: unexpected end of data",
                        codec,
                        i + pos_base,
                        i + 1 + pos_base
                    ),
                ));
            }
            if !handler_known(errors) {
                return Err(unknown_handler(errors));
            }
            match errors {
                "ignore" => {}
                "replace" => out.push('\u{FFFD}'),
                "backslashreplace" => out.push_str(&format!("\\x{:02x}\\x{:02x}", b[i], b[i + 1])),
                "surrogateescape" => {
                    wtf8::push_cp(&mut out, 0xDC00 + b[i] as u32);
                    wtf8::push_cp(&mut out, 0xDC00 + b[i + 1] as u32);
                }
                _ => out.push('\u{FFFD}'),
            }
            i += 2;
            continue;
        }
        wtf8::push_cp(&mut out, u as u32);
        i += 2;
    }
    Ok(out)
}

/// Decode UTF-32 bytes (le/be). Strict rejects surrogate code points
/// ("code point in surrogate code point range(0xd800, 0xe000)");
/// surrogatepass keeps them.
pub fn decode_utf32(b: &[u8], codec: &str, errors: &str, le: bool) -> Result<String, Flow> {
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if i + 4 > b.len() {
            if errors == "strict" {
                return Err(raise(
                    "UnicodeDecodeError",
                    format!(
                        "'{}' codec can't decode bytes in position {}-{}: unexpected end of data",
                        codec,
                        i,
                        b.len() - 1
                    ),
                ));
            }
            if !handler_known(errors) {
                return Err(unknown_handler(errors));
            }
            match errors {
                "ignore" => {}
                "replace" => out.push('\u{FFFD}'),
                "backslashreplace" => {
                    for &x in &b[i..] {
                        out.push_str(&format!("\\x{:02x}", x));
                    }
                }
                "surrogateescape" => {
                    for &x in &b[i..] {
                        wtf8::push_cp(&mut out, 0xDC00 + x as u32);
                    }
                }
                _ => out.push('\u{FFFD}'),
            }
            break;
        }
        let cp = if le {
            u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
        } else {
            u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
        };
        let bad = cp > 0x10FFFF || wtf8::is_surrogate(cp);
        if !bad {
            wtf8::push_cp(&mut out, cp);
            i += 4;
            continue;
        }
        if errors == "surrogatepass" && wtf8::is_surrogate(cp) {
            wtf8::push_cp(&mut out, cp);
            i += 4;
            continue;
        }
        if errors == "strict" {
            let reason = if wtf8::is_surrogate(cp) {
                "code point in surrogate code point range(0xd800, 0xe000)"
            } else {
                "code point not in range(0x110000)"
            };
            return Err(raise(
                "UnicodeDecodeError",
                format!(
                    "'{}' codec can't decode bytes in position {}-{}: {}",
                    codec,
                    i,
                    i + 3,
                    reason
                ),
            ));
        }
        if !handler_known(errors) {
            return Err(unknown_handler(errors));
        }
        match errors {
            "ignore" => {}
            "replace" => out.push('\u{FFFD}'),
            "backslashreplace" => {
                for &x in &b[i..i + 4] {
                    out.push_str(&format!("\\x{:02x}", x));
                }
            }
            "surrogateescape" => {
                for &x in &b[i..i + 4] {
                    wtf8::push_cp(&mut out, 0xDC00 + x as u32);
                }
            }
            _ => out.push('\u{FFFD}'),
        }
        i += 4;
    }
    Ok(out)
}

/// _codecs marker tuple shape: (bytes, consumed_code_points).
pub fn enc_tuple(bytes: Vec<u8>, n: usize) -> Value {
    tuple(vec![Value::Bytes(Gc::new(bytes)), i(n as i64)])
}

/// _codecs marker tuple shape: (str, consumed_bytes).
pub fn dec_tuple(text: String, n: usize) -> Value {
    tuple(vec![s(text), i(n as i64)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmsg(f: Flow) -> String {
        match f {
            Flow::Raise(e) => match e.args.first() {
                Some(Value::Str(s)) => s.as_str().to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    #[test]
    fn utf8_strict_surrogate_single() {
        let s = wtf8::from_code_points([0xD800]);
        let e = encode_bytes(&s, "utf-8", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-8' codec can't encode character '\\ud800' in position 0: surrogates not allowed"
        );
    }

    #[test]
    fn utf8_strict_surrogate_run() {
        let s = wtf8::from_code_points([0x61, 0xD800, 0xD801, 0x62]);
        let e = encode_bytes(&s, "utf-8", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-8' codec can't encode characters in position 1-2: surrogates not allowed"
        );
    }

    #[test]
    fn utf8_handlers() {
        let s = wtf8::from_code_points([0x78, 0xD800, 0x79]);
        assert_eq!(
            encode_bytes(&s, "utf-8", "ignore")
                .ok()
                .expect("expected ok"),
            b"xy"
        );
        assert_eq!(
            encode_bytes(&s, "utf-8", "replace")
                .ok()
                .expect("expected ok"),
            b"x?y"
        );
        assert_eq!(
            encode_bytes(&s, "utf-8", "backslashreplace")
                .ok()
                .expect("expected ok"),
            b"x\\ud800y"
        );
        assert_eq!(
            encode_bytes(&s, "utf-8", "xmlcharrefreplace")
                .ok()
                .expect("expected ok"),
            b"x&#55296;y"
        );
        assert_eq!(
            encode_bytes(&s, "utf-8", "surrogatepass")
                .ok()
                .expect("expected ok"),
            b"x\xed\xa0\x80y"
        );
    }

    #[test]
    fn surrogateescape_dc_only() {
        let s = wtf8::from_code_points([0xDC80]);
        assert_eq!(
            encode_bytes(&s, "utf-8", "surrogateescape")
                .ok()
                .expect("expected ok"),
            b"\x80"
        );
        let s2 = wtf8::from_code_points([0xD800]);
        assert!(encode_bytes(&s2, "utf-8", "surrogateescape").is_err());
        let s3 = wtf8::from_code_points([0xDC00]);
        assert!(encode_bytes(&s3, "utf-8", "surrogateescape").is_err());
    }

    #[test]
    fn unknown_handler_only_on_error() {
        assert!(encode_bytes("abc", "utf-8", "bogus").is_ok());
        let s = wtf8::from_code_points([0xD800]);
        let e = encode_bytes(&s, "utf-8", "bogus")
            .err()
            .expect("expected error");
        assert_eq!(fmsg(e), "unknown error handler name 'bogus'");
    }

    #[test]
    fn ascii_latin1_messages() {
        let e = encode_bytes("\u{e9}", "ascii", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'ascii' codec can't encode character '\\xe9' in position 0: ordinal not in range(128)"
        );
        let e = encode_bytes("a\u{e9}\u{ea}", "ascii", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'ascii' codec can't encode characters in position 1-2: ordinal not in range(128)"
        );
        let e = encode_bytes("\u{1F600}", "latin-1", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'latin-1' codec can't encode character '\\U0001f600' in position 0: ordinal not in range(256)"
        );
    }

    #[test]
    fn utf8_decode_strict_and_handlers() {
        assert_eq!(
            decode_utf8(b"\xc3\xa9", "strict")
                .ok()
                .expect("expected ok"),
            "\u{e9}"
        );
        let e = decode_utf8(b"\xed\xa0\x80", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-8' codec can't decode byte 0xed in position 0: invalid continuation byte"
        );
        let e = decode_utf8(b"\xc3", "strict")
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-8' codec can't decode byte 0xc3 in position 0: unexpected end of data"
        );
        assert_eq!(
            decode_utf8(b"\xed\xa0\x80", "surrogatepass")
                .ok()
                .expect("expected ok"),
            wtf8::from_code_points([0xD800])
        );
        assert_eq!(
            decode_utf8(b"\x80", "surrogateescape")
                .ok()
                .expect("expected ok"),
            wtf8::from_code_points([0xDC80])
        );
        assert_eq!(
            decode_utf8(b"\xff\xfe\xfd", "replace")
                .ok()
                .expect("expected ok"),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(
            decode_utf8(b"\xff\xfe\xfd", "backslashreplace")
                .ok()
                .expect("expected ok"),
            "\\xff\\xfe\\xfd"
        );
        assert_eq!(
            decode_utf8(b"\xed\xa0\x80", "ignore")
                .ok()
                .expect("expected ok"),
            ""
        );
    }

    #[test]
    fn utf16_encode_strict_and_pass() {
        let s = wtf8::from_code_points([0xD800]);
        let e = encode_utfn(&s, "utf-16-le", "strict", 16, true, false)
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-16-le' codec can't encode character '\\ud800' in position 0: surrogates not allowed"
        );
        assert_eq!(
            encode_utfn(&s, "utf-16-le", "surrogatepass", 16, true, false)
                .ok()
                .expect("expected ok"),
            b"\x00\xd8"
        );
        // BOM form: native LE BOM + raw unit.
        assert_eq!(
            encode_utfn(&s, "utf-16", "surrogatepass", 16, true, true)
                .ok()
                .expect("expected ok"),
            b"\xff\xfe\x00\xd8"
        );
        // Astral -> surrogate pair units.
        let a = "\u{1F600}";
        assert_eq!(
            encode_utfn(a, "utf-16", "surrogatepass", 16, true, true)
                .ok()
                .expect("expected ok"),
            b"\xff\xfe\x3d\xd8\x00\xde"
        );
    }

    #[test]
    fn utf16_decode_pairs_and_lone() {
        assert_eq!(
            decode_utf16(b"\x00\xd8\x00\xdc", "utf-16-le", "strict", true)
                .ok()
                .expect("expected ok"),
            "\u{10000}"
        );
        assert_eq!(
            decode_utf16(b"\x00\xd8", "utf-16-le", "surrogatepass", true)
                .ok()
                .expect("expected ok"),
            wtf8::from_code_points([0xD800])
        );
        let e = decode_utf16(b"\x00\xd8", "utf-16-le", "strict", true)
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-16-le' codec can't decode bytes in position 0-1: unexpected end of data"
        );
    }

    #[test]
    fn utf32_surrogate_range_message() {
        let e = decode_utf32(b"\x00\xd8\x00\x00", "utf-32-le", "strict", true)
            .err()
            .expect("expected error");
        assert_eq!(
            fmsg(e),
            "'utf-32-le' codec can't decode bytes in position 0-3: code point in surrogate code point range(0xd800, 0xe000)"
        );
        assert_eq!(
            decode_utf32(b"\x00\xd8\x00\x00", "utf-32-le", "surrogatepass", true)
                .ok()
                .expect("expected ok"),
            wtf8::from_code_points([0xD800])
        );
    }
}

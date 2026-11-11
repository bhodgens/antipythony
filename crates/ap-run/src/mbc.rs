//! Native multibyte CJK codecs (GitHub issue #13).
//!
//! Modules surfaced: `_multibytecodec`, `_codecs_jp`, `_codecs_kr`,
//! `_codecs_cn`, `_codecs_tw`, `_codecs_hk`, `_codecs_iso2022`.
//!
//! Byte-exactness contract vs CPython 3.14.0: the mapping tables are
//! GENERATED from the reference interpreter's own codec data
//! (tests/corpus/codecs_cjk/gen_tables.py + gen_gb18030_4.py); the
//! iso2022 escape designations were probed from the reference
//! (tests/corpus/codecs_cjk/probe_iso2022*.py). Differential triples are
//! pinned in tests/corpus/codecs_cjk/ground_truth_py314.json and exercised
//! by tests/t11_stdlib/test_cjk_codecs.py.
//!
//! Architecture: every codec's `getcodec(name)` returns a CodecInfo-shaped
//! namespace whose `encode`/`decode` are `_mbc.<name>.encode|decode`
//! markers; the vendored encodings/*.py wrappers bind them into their Codec
//! classes exactly like CPython (codec.encode/codec.decode). The marker
//! dispatch (call_mbc) routes to ONE stateless engine parameterized by the
//! codec's table set, so the seven native modules differ only in table
//! selection — mirroring how CPython's five C modules share
//! _multibytecodec's engine.
//!
//! Error handlers on encode: strict/replace/ignore/backslashreplace/
//! xmlcharrefreplace; on decode: strict/replace/ignore. Unknown handler
//! names raise LookupError like CPython (real registered handlers such as
//! w3lib's resolve through the codecs-module Python layer — this engine
//! raises `LookupError: unknown error handler` only when the handler is
//! neither builtin nor registered).

use crate::builtins_modules::{finish, i, marker, new_module, raise, s, str_arg, tuple, BResult};
use ap_obj::int::Gc;
use ap_obj::object::{ObjKind, PyObject};
use ap_obj::value::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

#[path = "mbc_gb18030_4.rs"]
mod mbc_gb18030_4;
#[path = "mbc_iso2022_spec.rs"]
mod mbc_iso2022_spec;
#[path = "mbc_tables.rs"]
mod mbc_tables;
#[path = "mbc_utf7.rs"]
pub(crate) mod mbc_utf7;

use mbc_gb18030_4::{Gb4Run, ASTRAL_ANCHOR_CP, ASTRAL_ANCHOR_SEQ, GB18030_4_BMP_RUNS};

/// CPython's escape for the offending char in codec error messages:
/// U+007F..U+00FF -> \xNN; other BMP non-ASCII -> \uNNNN; astral -> \UNNNNNNNN.
fn msg_char_escape(ch: char) -> String {
    let cp = ch as u32;
    if (0x80..0x100).contains(&cp) {
        format!("\\x{:02x}", cp)
    } else if cp <= 0xFFFF {
        format!("\\u{:04x}", cp)
    } else {
        format!("\\U{:08x}", cp)
    }
}

use mbc_iso2022_spec::{spec_for, Cs, Iso2022Spec, MultibyteKind};

// ===========================================================================
// Codec table registry
// ===========================================================================

/// The table set behind one codec name.
pub struct CodecTables {
    /// decode: byte sequence (hex key) -> Unicode scalar.
    pub dec: HashMap<&'static str, u32>,
    /// encode: char -> byte sequence.
    pub enc: HashMap<u32, &'static [u8]>,
    /// Longest key length (1..=3 bytes) — decode scan window.
    pub max_len: usize,
    /// gb18030-only: the 4-byte linear ranges.
    pub gb4: bool,
    /// HZ state machine (decoded via ~ escape sequences).
    pub hz: bool,
    /// ISO-2022 state machine with this designation table.
    pub iso: Option<&'static Iso2022Spec>,
    /// Kind name used in error messages (canonical codec name).
    pub canon: &'static str,
}

fn build_tables(raw: &'static [(&'static str, u32)], canon: &'static str) -> CodecTables {
    let mut dec = HashMap::with_capacity(raw.len());
    let mut enc = HashMap::with_capacity(raw.len());
    let mut max_len = 1usize;
    // Decode map: FIRST occurrence wins (CPython decode maps are unique);
    // encode map: LAST occurrence wins (built by forward iteration of the
    // decode map, mirroring CPython's encode-table construction where the
    // final mapping for a char wins).
    for (hex, cp) in raw {
        dec.entry(*hex).or_insert(*cp);
        let n = hex.len() / 2;
        if n > max_len {
            max_len = n;
        }
    }
    // Invert in REVERSE so the first writer (the last mapping in the table)
    // wins — CPython builds the forward map by scanning the decode map in
    // order with "don't overwrite existing" semantics for duplicates
    // arriving later; our tables are byte-sorted, so reversing reproduces
    // [allow(clippy::needless_range_loop)]
    for (hex, cp) in raw.iter().rev() {
        let mut bs: Vec<u8> = Vec::with_capacity(hex.len() / 2);
        let b = hex.as_bytes();
        let mut k = 0;
        while k + 1 < b.len() {
            let hi = (b[k] as char).to_digit(16).unwrap_or(0) as u8;
            let lo = (b[k + 1] as char).to_digit(16).unwrap_or(0) as u8;
            bs.push((hi << 4) | lo);
            k += 2;
        }
        let leaked: &'static [u8] = Box::leak(bs.into_boxed_slice());
        enc.entry(*cp).or_insert(leaked);
    }
    CodecTables {
        dec,
        enc,
        max_len,
        gb4: false,
        hz: false,
        iso: None,
        canon,
    }
}

#[allow(unused_macros)] // kept for parity with sibling codec fns; currently unreferenced
macro_rules! table_of {
    ($name:literal) => {
        &mbc_tables::$name
    };
}

fn cp932() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_CP932, "cp932"))
}

fn euc_jp() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_EUC_JP, "euc_jp"))
}

fn shift_jis() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_SHIFT_JIS, "shift_jis"))
}

fn shift_jisx0213() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_SHIFT_JISX0213, "shift_jisx0213"))
}

fn shift_jis_2004() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_SHIFT_JIS_2004, "shift_jis_2004"))
}

fn euc_jisx0213() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_EUC_JISX0213, "euc_jisx0213"))
}

fn euc_jis_2004() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_EUC_JIS_2004, "euc_jis_2004"))
}

fn euc_kr() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_EUC_KR, "euc_kr"))
}

fn cp949() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_CP949, "cp949"))
}

fn johab() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_JOHAB, "johab"))
}

fn gb2312() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_GB2312, "gb2312"))
}

fn gbk() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_GBK, "gbk"))
}

fn big5() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_BIG5, "big5"))
}

fn cp950() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_CP950, "cp950"))
}

fn big5hkscs() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| build_tables(mbc_tables::MAP_BIG5HKSCS, "big5hkscs"))
}

fn gb18030() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = build_tables(mbc_tables::MAP_GB18030_2, "gb18030");
        t.gb4 = true;
        t
    })
}

fn hz() -> &'static CodecTables {
    static T: OnceLock<CodecTables> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = build_tables(mbc_tables::MAP_GB2312, "hz");
        t.hz = true;
        t
    })
}

fn tables_for(name: &str) -> Option<&'static CodecTables> {
    match name {
        "cp932" => Some(cp932()),
        "euc_jp" => Some(euc_jp()),
        "shift_jis" => Some(shift_jis()),
        "shift_jisx0213" => Some(shift_jisx0213()),
        "shift_jis_2004" => Some(shift_jis_2004()),
        "euc_jisx0213" => Some(euc_jisx0213()),
        "euc_jis_2004" => Some(euc_jis_2004()),
        "euc_kr" => Some(euc_kr()),
        "cp949" => Some(cp949()),
        "johab" => Some(johab()),
        "gb2312" => Some(gb2312()),
        "gbk" => Some(gbk()),
        "gb18030" => Some(gb18030()),
        "hz" => Some(hz()),
        "big5" => Some(big5()),
        "cp950" => Some(cp950()),
        "big5hkscs" => Some(big5hkscs()),
        "iso2022_jp" | "iso2022_jp_1" | "iso2022_jp_2" | "iso2022_jp_3" | "iso2022_jp_ext"
        | "iso2022_jp_2004" | "iso2022_kr" => {
            let spec = spec_for(name)?;
            // iso2022 codecs decode through the state machine; their tables
            // come from the underlying charset (JIS0208 via euc_jp rows,
            // KSC5601 via euc_kr rows, GB2312 via gb2312 rows).
            let _base: &'static CodecTables = match spec.multibyte {
                MultibyteKind::JisX0208 => euc_jp(),
                MultibyteKind::JisX0213 => euc_jis_2004(),
                MultibyteKind::JisX0212 => euc_jp(),
                MultibyteKind::Ksc5601 => euc_kr(),
                MultibyteKind::Gb2312 => gb2312(),
            };
            static ISO_TABLES: OnceLock<HashMap<&'static str, CodecTables>> = OnceLock::new();
            let map = ISO_TABLES.get_or_init(|| {
                let mut m = HashMap::new();
                for nm in [
                    "iso2022_jp",
                    "iso2022_jp_1",
                    "iso2022_jp_2",
                    "iso2022_jp_3",
                    "iso2022_jp_ext",
                    "iso2022_jp_2004",
                    "iso2022_kr",
                ] {
                    let spec = spec_for(nm).expect("spec");
                    let base: &'static CodecTables = match spec.multibyte {
                        MultibyteKind::JisX0208 => euc_jp(),
                        MultibyteKind::JisX0213 => euc_jis_2004(),
                        MultibyteKind::JisX0212 => euc_jp(),
                        MultibyteKind::Ksc5601 => euc_kr(),
                        MultibyteKind::Gb2312 => gb2312(),
                    };
                    m.insert(
                        nm,
                        CodecTables {
                            dec: base.dec.clone(),
                            enc: base.enc.clone(),
                            max_len: base.max_len,
                            gb4: false,
                            hz: false,
                            iso: Some(spec),
                            canon: nm,
                        },
                    );
                }
                m
            });
            map.get(name)
        }
        _ => None,
    }
}

// ===========================================================================
// Encode engine
// ===========================================================================

pub enum EncError {
    /// (start_char_index, end_char_index_exclusive, char)
    Encode(usize, usize, char),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EncHandler {
    Strict,
    Replace,
    Ignore,
    BackslashReplace,
    XmlCharrefReplace,
    Other,
}

fn enc_handler_of(errors: &str) -> EncHandler {
    match errors {
        "strict" => EncHandler::Strict,
        "replace" => EncHandler::Replace,
        "ignore" => EncHandler::Ignore,
        "backslashreplace" => EncHandler::BackslashReplace,
        "xmlcharrefreplace" => EncHandler::XmlCharrefReplace,
        _ => EncHandler::Other,
    }
}

/// Encode `text` under `t`. Returns the bytes, or raises via `raise()`
/// with CPython's exact message for strict-mode failures.
pub fn mbc_encode(t: &CodecTables, text: &str, errors: &str) -> Result<Vec<u8>, crate::Flow> {
    let handler = enc_handler_of(errors);
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut known_handler = true;
    let mut bad: Option<(usize, usize, char)> = None;
    for (i, ch) in text.char_indices() {
        if let Some(bs) = t.enc.get(&(ch as u32)) {
            out.extend_from_slice(bs);
        } else if t.gb4 {
            // Issue #13: chars outside the 2-byte table fall to the 4-byte
            // runs engine (BMP runs + astral odometric anchor, both generated
            // from CPython 3.14). Unmapped cps still error/replace below.
            if let Some(seq) = gb4_encode_cp(ch as u32) {
                out.extend_from_slice(&seq);
            } else {
                match handler {
                    EncHandler::Strict => {
                        bad = Some((i, i + ch.len_utf8(), ch));
                        break;
                    }
                    EncHandler::Replace => {
                        // CPython multibytecodec replace emits ONE '?' per
                        // failing CHARACTER (not per byte).
                        out.push(b'?');
                    }
                    EncHandler::Ignore => {}
                    EncHandler::BackslashReplace => {
                        let s = if (ch as u32) < 0x100 {
                            format!("\\x{:02x}", ch as u32)
                        } else if (ch as u32) < 0x10000 {
                            format!("\\u{:04x}", ch as u32)
                        } else {
                            format!("\\U{:08x}", ch as u32)
                        };
                        out.extend_from_slice(s.as_bytes());
                    }
                    EncHandler::XmlCharrefReplace => {
                        let s = format!("&#{};", ch as u32);
                        out.extend_from_slice(s.as_bytes());
                    }
                    EncHandler::Other => {
                        known_handler = false;
                        bad = Some((i, i + ch.len_utf8(), ch));
                        break;
                    }
                }
            }
        } else {
            match handler {
                EncHandler::Strict => {
                    bad = Some((i, i + ch.len_utf8(), ch));
                    break;
                }
                EncHandler::Replace => {
                    // CPython multibytecodec replace emits ONE '?' per
                    // failing CHARACTER (not per byte).
                    out.push(b'?');
                }
                EncHandler::Ignore => {}
                EncHandler::BackslashReplace => {
                    let s = if (ch as u32) < 0x100 {
                        format!("\\x{:02x}", ch as u32)
                    } else if (ch as u32) < 0x10000 {
                        format!("\\u{:04x}", ch as u32)
                    } else {
                        format!("\\U{:08x}", ch as u32)
                    };
                    out.extend_from_slice(s.as_bytes());
                }
                EncHandler::XmlCharrefReplace => {
                    let s = format!("&#{};", ch as u32);
                    out.extend_from_slice(s.as_bytes());
                }
                EncHandler::Other => {
                    known_handler = false;
                    bad = Some((i, i + ch.len_utf8(), ch));
                    break;
                }
            }
        }
    }
    if let Some((start, _end, ch)) = bad {
        if !known_handler {
            return Err(raise(
                "LookupError",
                "unknown error handler: non-builtin handler".to_string(),
            ));
        }
        // CPython raises UnicodeEncodeError with the standard message text;
        // the exception CLASS is UnicodeEncodeError (subclass of
        // UnicodeError/ValueError wired in exc_hier).
        return Err(raise(
            "UnicodeEncodeError",
            format!(
                // CPython escape rule for the offending char in the message:
                // U+007F..U+00FF -> \\xNN; everything else non-ASCII -> \\uNNNN
                // (astral chars -> \\UNNNNNNNN).
                "'{}' codec can't encode character '{}' in position {}: illegal multibyte sequence",
                t.canon,
                msg_char_escape(ch),
                start
            ),
        ));
    }
    Ok(out)
}

// ===========================================================================
// Decode engine
// ===========================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecHandler {
    Strict,
    Replace,
    Ignore,
    Other,
}

fn dec_handler_of(errors: &str) -> DecHandler {
    match errors {
        "strict" => DecHandler::Strict,
        "replace" => DecHandler::Replace,
        "ignore" => DecHandler::Ignore,
        _ => DecHandler::Other,
    }
}

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
fn hex_decode(hex: &str) -> Vec<u8> {
    let b = hex.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut k = 0;
    while k + 1 < b.len() {
        let hi = (b[k] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (b[k + 1] as char).to_digit(16).unwrap_or(0) as u8;
        out.push((hi << 4) | lo);
        k += 2;
    }
    out
}

/// HZ decode: ~{ ... ~} switches between ASCII and GB2312 modes; ~~ -> ~.
fn hz_decode(t: &CodecTables, b: &[u8], errors: &str) -> Result<String, crate::Flow> {
    let handler = dec_handler_of(errors);
    let mut out = String::new();
    let mut gb = false;
    let mut i = 0usize;
    let n = b.len();
    while i < n {
        if b[i] == 0x7E {
            if i + 1 >= n {
                // trailing lone tilde: CPython errors "incomplete multibyte sequence"
                match handler {
                    DecHandler::Strict => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!("'hz' codec can't decode byte 0x7e in position {}: incomplete multibyte sequence", i),
                        ))
                    }
                    DecHandler::Replace => {
                        out.push('\u{fffd}');
                        i += 1;
                    }
                    DecHandler::Ignore => i += 1,
                    DecHandler::Other => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!("'hz' codec can't decode byte 0x7e in position {}: incomplete multibyte sequence", i),
                        ))
                    }
                }
                continue;
            }
            match b[i + 1] {
                0x7B => {
                    gb = true;
                    i += 2;
                }
                0x7D => {
                    gb = false;
                    i += 2;
                }
                0x7E => {
                    out.push('~');
                    i += 2;
                }
                0x7C => {
                    // newline escape: consume (emits nothing)
                    i += 2;
                }
                _ => {
                    // ~ followed by other char in GB mode: CPython raises
                    match handler {
                        DecHandler::Strict => {
                            return Err(raise(
                                "UnicodeDecodeError",
                                format!(
                                    "'hz' codec can't decode byte 0x{:02x} in position {}: illegal multibyte sequence",
                                    b[i + 1],
                                    i + 1
                                ),
                            ))
                        }
                        DecHandler::Replace => {
                            out.push('\u{fffd}');
                            i += 2;
                        }
                        DecHandler::Ignore => i += 2,
                        DecHandler::Other => {
                            return Err(raise(
                                "UnicodeDecodeError",
                                format!(
                                    "'hz' codec can't decode byte 0x{:02x} in position {}: illegal multibyte sequence",
                                    b[i + 1],
                                    i + 1
                                ),
                            ))
                        }
                    }
                }
            }
            continue;
        }
        if gb {
            if i + 1 >= n {
                match handler {
                    DecHandler::Strict => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!("'hz' codec can't decode byte 0x{:02x} in position {}: incomplete multibyte sequence", b[i], i),
                        ))
                    }
                    DecHandler::Replace => {
                        out.push('\u{fffd}');
                        i += 1;
                    }
                    DecHandler::Ignore => i += 1,
                    DecHandler::Other => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!("'hz' codec can't decode byte 0x{:02x} in position {}: incomplete multibyte sequence", b[i], i),
                        ))
                    }
                }
                continue;
            }
            // Issue #13: HZ payload rows are 7-bit (high-bit STRIPPED) gb2312
            // pairs — re-add the 0x80 bit before the table lookup (encode
            // strips it; decode must mirror: 56 50 -> D6 D0 -> 中).
            let key = format!("{:02x}{:02x}", b[i] | 0x80, b[i + 1] | 0x80);
            match t.dec.get(key.as_str()) {
                Some(&cp) => {
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    i += 2;
                }
                None => match handler {
                    DecHandler::Strict => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!(
                                "'hz' codec can't decode byte 0x{:02x} in position {}: illegal multibyte sequence",
                                b[i], i
                            ),
                        ))
                    }
                    DecHandler::Replace => {
                        out.push('\u{fffd}');
                        i += 2;
                    }
                    DecHandler::Ignore => i += 2,
                    DecHandler::Other => {
                        return Err(raise(
                            "UnicodeDecodeError",
                            format!(
                                "'hz' codec can't decode byte 0x{:02x} in position {}: illegal multibyte sequence",
                                b[i], i
                            ),
                        ))
                    }
                },
            }
        } else {
            // ASCII mode
            out.push(b[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

/// HZ encode: ASCII stays; CJK -> ~{ gb2 ~}; literal ~ -> ~~.
fn hz_encode(t: &CodecTables, text: &str, _errors: &str) -> Result<Vec<u8>, crate::Flow> {
    let mut out = Vec::new();
    let mut gb = false;
    for ch in text.chars() {
        if (ch as u32) < 0x80 {
            if gb {
                out.extend_from_slice(b"~}");
                gb = false;
            }
            if ch == '~' {
                out.extend_from_slice(b"~~");
            } else {
                out.push(ch as u8);
            }
        } else if let Some(bs) = t.enc.get(&(ch as u32)) {
            if !gb {
                out.extend_from_slice(b"~{");
                gb = true;
            }
            // Issue #13 (orchestrator note): HZ payload rows are the 8-bit
            // gb2312 pairs with the HIGH BITS STRIPPED (0xA1.. → 0x21..);
            // emitting the raw 8-bit pair makes hz output raw GBK which is
            // wrong (ref: '中'.encode('hz') == 7e7b56507e7d).
            if bs.len() == 2 && bs[0] >= 0xA1 {
                out.push(bs[0] - 0x80);
                out.push(bs[1] - 0x80);
            } else {
                out.extend_from_slice(bs);
            }
        } else {
            // error path same as plain encode (strict message at char pos)
            let pos = text.chars().take_while(|&c| c != ch).count();
            return Err(raise(
                "UnicodeEncodeError",
                format!(
                    "'hz' codec can't encode character '{}' in position {}: illegal multibyte sequence",
                    msg_char_escape(ch), pos
                ),
            ));
        }
    }
    if gb {
        out.extend_from_slice(b"~}");
    }
    Ok(out)
}

/// ISO-2022 decode: ESC designations switch the G0 charset; payload bytes
/// resolve through the ACTIVE charset's table (2 bytes for CJK sets).
fn iso_decode(t: &CodecTables, b: &[u8], errors: &str) -> Result<String, crate::Flow> {
    let spec = t.iso.expect("iso tables");
    let handler = dec_handler_of(errors);
    let mut out = String::new();
    let mut cs: Cs = Cs::Ascii;
    let mut i = 0usize;
    let n = b.len();
    let mut err_at: Option<(usize, usize, String)> = None; // (start, end, kind)
                                                           // True after an ESC$)C-style G1-only designation (0x29 = ')'), which does
                                                           // NOT switch the active payload charset.
    let mut active_is_g0_ksc = false;
    while i < n {
        // escape matching
        if b[i] == 0x1B {
            let mut matched = false;
            'desig: for d in spec.designations {
                let db = d.bytes;
                if i + db.len() <= n && &b[i..i + db.len()] == db {
                    cs = d.cs;
                    // 0x24=('$') 0x29=(')') = the G1-only form ESC$)C
                    active_is_g0_ksc = db.contains(&0x29);
                    i += db.len();
                    matched = true;
                    break 'desig;
                }
            }
            // ASCII-return designations ESC ( B / ESC ( J accepted everywhere
            if !matched && i + 3 <= n && (&b[i..i + 3] == b"\x1b(B" || &b[i..i + 3] == b"\x1b(J") {
                cs = Cs::Ascii;
                i += 3;
                matched = true;
            }
            if !matched {
                // unknown escape: CPython errors on the ESC itself
                match handler {
                    DecHandler::Strict => {
                        err_at = Some((i, i + 1, "illegal".to_string()));
                        break;
                    }
                    DecHandler::Replace => {
                        out.push('\u{fffd}');
                        i += 1;
                    }
                    DecHandler::Ignore => i += 1,
                    DecHandler::Other => {
                        err_at = Some((i, i + 1, "illegal".to_string()));
                        break;
                    }
                }
            }
            continue;
        }
        // SO/SI shifts (iso2022_kr)
        if spec.shifts && (b[i] == 0x0E || b[i] == 0x0F) {
            if b[i] == 0x0E {
                cs = Cs::Ksc5601;
            } else {
                cs = Cs::Ascii;
            }
            i += 1;
            continue;
        }
        match cs {
            Cs::Ascii => {
                out.push(b[i] as char);
                i += 1;
            }
            Cs::JisX0201Kana => {
                // halfwidth kana: single bytes 0x21..0x5F -> U+FF61..
                if (0x21..=0x5F).contains(&b[i]) {
                    out.push(char::from_u32(0xFF61 + (b[i] as u32 - 0x21)).unwrap_or('\u{fffd}'));
                    i += 1;
                } else {
                    match handler {
                        DecHandler::Strict => {
                            err_at = Some((i, i + 1, "illegal".to_string()));
                            break;
                        }
                        DecHandler::Replace => {
                            out.push('\u{fffd}');
                            i += 1;
                        }
                        DecHandler::Ignore => i += 1,
                        DecHandler::Other => {
                            err_at = Some((i, i + 1, "illegal".to_string()));
                            break;
                        }
                    }
                }
            }
            _ => {
                if i + 1 >= n {
                    match handler {
                        DecHandler::Strict => {
                            err_at = Some((i, n, "incomplete".to_string()));
                            break;
                        }
                        DecHandler::Replace => {
                            out.push('\u{fffd}');
                            i += 1;
                        }
                        DecHandler::Ignore => i += 1,
                        DecHandler::Other => {
                            err_at = Some((i, n, "incomplete".to_string()));
                            break;
                        }
                    }
                    continue;
                }
                // Some designations DON'T actually switch the payload charset
                // (probed: iso2022_jp's ESC$)C / iso2022_kr's ESC$)C only
                // designate G1 — payload stays ASCII until SO).
                let payload_cs = match cs {
                    // Only the ESC$)C G1 designation leaves the payload on
                    // ASCII in the jp family (KSC reachable via SO only,
                    // which the jp codecs don't honor). A REAL ESC$(C G0
                    // switch (iso2022_jp_2) DOES carry KSC payload bytes.
                    Cs::Ksc5601 if !spec.shifts && active_is_g0_ksc => Cs::Ascii,
                    other => other,
                };
                if payload_cs == Cs::Ascii {
                    out.push(b[i] as char);
                    out.push(b[i + 1] as char);
                    i += 2;
                    continue;
                }
                // Issue #13 (root fix, decode side): each payload charset
                // resolves through ITS OWN EUC table, never the codec base
                // table. JIS X 0212 lives in euc_jp's 0x8F 3-byte rows (its
                // 2-byte ISO payload pairs rehydrate to 8F-prefixed keys);
                // KSC/GB/X0213 use their own tables' 2-byte rows.
                let cs_dec: &CodecTables = match payload_cs {
                    Cs::JisX0213P1 | Cs::JisX0213P2 => euc_jis_2004(),
                    Cs::Ksc5601 => euc_kr(),
                    Cs::Gb2312 => gb2312(),
                    _ => euc_jp(),
                };
                let hi = b[i] | 0x80;
                let lo = b[i + 1] | 0x80;
                // JIS X 0212 payload pairs sit in the 0x8F-prefixed 3-byte
                // euc_jp rows (0xA1.. lead); 2-byte rows never contain them.
                let keys: [String; 2] = if payload_cs == Cs::JisX0212 {
                    [
                        format!("8f{:02x}{:02x}", hi, lo),
                        format!("{:02x}{:02x}", hi, lo),
                    ]
                } else {
                    [
                        format!("{:02x}{:02x}", hi, lo),
                        format!("8f{:02x}{:02x}", hi, lo),
                    ]
                };
                let hit = keys
                    .iter()
                    .find_map(|k| cs_dec.dec.get(k.as_str()).copied());
                let _ = t;
                match hit {
                    Some(cp) => {
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        i += 2;
                    }
                    None => match handler {
                        DecHandler::Strict => {
                            err_at = Some((i, i + 2, "illegal".to_string()));
                            break;
                        }
                        DecHandler::Replace => {
                            out.push('\u{fffd}');
                            i += 2;
                        }
                        DecHandler::Ignore => i += 2,
                        DecHandler::Other => {
                            err_at = Some((i, i + 2, "illegal".to_string()));
                            break;
                        }
                    },
                }
            }
        }
    }
    if let Some((start, end, kind)) = err_at {
        let msg = if kind == "incomplete" {
            "incomplete multibyte sequence".to_string()
        } else {
            "illegal multibyte sequence".to_string()
        };
        return Err(raise(
            "UnicodeDecodeError",
            format!(
                "'{}' codec can't decode bytes in position {}-{}: {}",
                t.canon,
                start,
                end.saturating_sub(1),
                msg
            ),
        ));
    }
    Ok(out)
}

/// Plain (stateless) decode: longest-match over the byte-sequence table +
/// gb18030 4-byte linear ranges.
fn plain_decode(t: &CodecTables, b: &[u8], errors: &str) -> Result<String, crate::Flow> {
    let handler = dec_handler_of(errors);
    let mut out = String::with_capacity(b.len());
    let mut i = 0usize;
    let n = b.len();
    let mut err: Option<(usize, usize, bool)> = None; // (start, end, incomplete?)
    while i < n {
        let mut matched = false;
        // gb18030 four-byte first (they may collide with 2-byte table rows
        // on the first byte; CPython's decoder commits per lead byte).
        if t.gb4 && n - i >= 4 {
            let b1 = b[i];
            let b2 = b[i + 1];
            let b3 = b[i + 2];
            let b4 = b[i + 3];
            let is4 = (0x81..=0xFE).contains(&b1)
                && (0x30..=0x39).contains(&b2)
                && (0x81..=0xFE).contains(&b3)
                && (0x30..=0x39).contains(&b4);
            if is4 {
                // Odometric linear lookup over the sibling's Gb4Run layout:
                // N(seq) = (b0-0x81)*12600 + (b1-0x30)*1260 + (b2-0x81)*10 +
                // (b3-0x30); within a run cp = cp_start + (N - N(seq_start)).
                let n_of = |s: &[u8; 4]| -> u32 {
                    (s[0].wrapping_sub(0x81) as u32) * 12600
                        + (s[1].wrapping_sub(0x30) as u32) * 1260
                        + (s[2].wrapping_sub(0x81) as u32) * 10
                        + (s[3].wrapping_sub(0x30) as u32)
                };
                let seq = [b1, b2, b3, b4];
                let n_seq = n_of(&seq);
                let mut cp: Option<u32> = None;
                for run in GB18030_4_BMP_RUNS.iter() {
                    let n0 = n_of(&run.seq_start);
                    let d = n_seq.wrapping_sub(n0);
                    if d <= run.cp_end - run.cp_start {
                        cp = Some(run.cp_start + d);
                        break;
                    }
                }
                if cp.is_none() {
                    // astral: single odometric walk anchored at 0x10000
                    let n_anchor = n_of(&ASTRAL_ANCHOR_SEQ);
                    let d = n_seq.wrapping_sub(n_anchor);
                    // bound the walk to the real astral allocation (avoids
                    // absurd codepoints from arbitrary 4-byte shapes)
                    if d <= 0x10_FFFF - ASTRAL_ANCHOR_CP {
                        cp = Some(ASTRAL_ANCHOR_CP + d);
                    }
                }
                match cp {
                    Some(c) => {
                        out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                        i += 4;
                        matched = true;
                    }
                    None => {
                        // 4-byte shape but unmapped: illegal at i
                        match handler {
                            DecHandler::Strict => {
                                err = Some((i, i + 1, false));
                                break;
                            }
                            DecHandler::Replace => {
                                out.push('\u{fffd}');
                                i += 1;
                            }
                            DecHandler::Ignore => i += 1,
                            DecHandler::Other => {
                                err = Some((i, i + 1, false));
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
        }
        if !matched {
            let maxk = t.max_len.min(n - i);
            // Longest-first match.
            for k in (1..=maxk).rev() {
                if k == 1 && b[i] < 0x80 {
                    // single-byte ASCII rows are in the table; fall through
                }
                let mut key = String::with_capacity(k * 2);
                for j in 0..k {
                    key.push_str(&format!("{:02x}", b[i + j]));
                }
                if let Some(&cp) = t.dec.get(key.as_str()) {
                    out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                    i += k;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            // determine incomplete vs illegal: a multi-byte LEAD at the tail
            let incomplete = t.max_len > 1 && i + 1 == n && is_lead_byte(t, b[i]);
            match handler {
                DecHandler::Strict => {
                    err = Some((i, i + 1, incomplete));
                    break;
                }
                DecHandler::Replace => {
                    out.push('\u{fffd}');
                    i += 1;
                }
                DecHandler::Ignore => i += 1,
                DecHandler::Other => {
                    err = Some((i, i + 1, incomplete));
                    break;
                }
            }
        }
    }
    if let Some((start, _end, incomplete)) = err {
        let msg = if incomplete {
            "incomplete multibyte sequence"
        } else {
            "illegal multibyte sequence"
        };
        return Err(raise(
            "UnicodeDecodeError",
            format!(
                "'{}' codec can't decode byte 0x{:02x} in position {}: {}",
                t.canon, b[start], start, msg
            ),
        ));
    }
    Ok(out)
}

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
fn in_digit_span(b: u8, lo: u8, hi: u8) -> bool {
    (lo..=hi).contains(&b)
}

// ===========================================================================
// gb18030 four-byte engine (runs table generated from CPython 3.14)
// ===========================================================================

/// Odometric linear index of a 4-byte gb18030 sequence: digit bytes 0x30-0x39
/// at positions 1/3 (base 10), wide bytes 0x81-0xFE at positions 0/2 (base 126).
fn gb4_to_lin(b: [u8; 4]) -> u32 {
    ((b[0] - 0x81) as u32 * 10 + (b[1] - 0x30) as u32) * 1260
        + (b[2] - 0x81) as u32 * 10
        + (b[3] - 0x30) as u32
}

/// Inverse of `gb4_to_lin`.
fn gb4_from_lin(l: u32) -> [u8; 4] {
    let b4 = 0x30 + (l % 10) as u8;
    let q = l / 10;
    let b3 = 0x81 + (q % 126) as u8;
    let q = q / 126;
    let b2 = 0x30 + (q % 10) as u8;
    let q = q / 10;
    let b1 = 0x81 + q as u8;
    [b1, b2, b3, b4]
}

/// Encode one codepoint to its gb18030 4-byte sequence; None when the cp is
/// not 4-byte encodable (2-byte table coverage is checked by the caller).
pub fn gb4_encode_cp(cp: u32) -> Option<[u8; 4]> {
    if cp >= ASTRAL_ANCHOR_CP {
        return Some(gb4_from_lin(
            gb4_to_lin(ASTRAL_ANCHOR_SEQ) + (cp - ASTRAL_ANCHOR_CP),
        ));
    }
    // binary search BMP runs
    let mut lo = 0usize;
    let mut hi = GB18030_4_BMP_RUNS.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let r: &Gb4Run = &GB18030_4_BMP_RUNS[mid];
        if cp < r.cp_start {
            hi = mid;
        } else if cp > r.cp_end {
            lo = mid + 1;
        } else {
            return Some(gb4_from_lin(gb4_to_lin(r.seq_start) + (cp - r.cp_start)));
        }
    }
    None
}

/// Decode one gb18030 4-byte sequence to its codepoint; None when unmapped.
pub fn gb4_decode_seq(b: [u8; 4]) -> Option<u32> {
    let l = gb4_to_lin(b);
    // astral walk: single contiguous odometric span from the anchor
    let a_lin = gb4_to_lin(ASTRAL_ANCHOR_SEQ);
    if l >= a_lin {
        let cp = ASTRAL_ANCHOR_CP + (l - a_lin);
        if cp <= 0x10FFFF {
            return Some(cp);
        }
        return None;
    }
    // BMP runs: binary search by sequence index. A run spans linear
    // [to_lin(seq_start), to_lin(seq_start) + (cp_end - cp_start)].
    let mut lo = 0usize;
    let mut hi = GB18030_4_BMP_RUNS.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let r: &Gb4Run = &GB18030_4_BMP_RUNS[mid];
        let s = gb4_to_lin(r.seq_start);
        let e = s + (r.cp_end - r.cp_start);
        if l < s {
            hi = mid;
        } else if l > e {
            lo = mid + 1;
        } else {
            return Some(r.cp_start + (l - s));
        }
    }
    None
}

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
fn is_gb4_shape(b: &[u8]) -> bool {
    b.len() == 4
        && (0x81..=0xFE).contains(&b[0])
        && (0x30..=0x39).contains(&b[1])
        && (0x81..=0xFE).contains(&b[2])
        && (0x30..=0x39).contains(&b[3])
}

/// True when `b` could begin a multi-byte sequence under `t` (used to pick
/// the "incomplete" vs "illegal" reason at end-of-input).
fn is_lead_byte(t: &CodecTables, b: u8) -> bool {
    // Any table row longer than 1 byte starting with this byte value.
    let hex = format!("{:02x}", b);
    t.dec.keys().any(|k| k.len() > 2 && k.starts_with(&hex))
}

/// ISO-2022 encode: switch to the target charset, emit 2 payload bytes,
/// return to ASCII. Charsets chosen by the char's availability.
fn iso_encode(t: &CodecTables, text: &str, errors: &str) -> Result<Vec<u8>, crate::Flow> {
    let spec = t.iso.expect("iso tables");
    let handler = enc_handler_of(errors);
    let mut out = Vec::new();
    let mut active: Option<&'static [u8]> = None; // current designation bytes
    let mut active_cs: Cs = Cs::Ascii;
    // Issue #13 (orchestrator note): iso2022_kr emits the designation ONCE
    // before the first multibyte char, then SO(0x0E) before / SI(0x0F) after
    // each multibyte run (ref: 'a한b' -> 611b2429430e47510f62).
    let mut kr_desig_done = false;
    for (ci, ch) in text.char_indices() {
        if (ch as u32) < 0x80 && ch != '\n' && ch != '\r' {
            // iso2022_kr: SI closes the shifted run (never ESC-returns).
            if spec.shifts && active.is_some() {
                out.push(0x0F);
                active = None;
                active_cs = Cs::Ascii;
            }
            // ISO-2022 jp family returns to ASCII around every non-ASCII run
            // (CPython emits ESC(B before ASCII after multibyte).
            if !spec.shifts && active.is_some() {
                out.extend_from_slice(spec.ascii_return);
                active = None;
                active_cs = Cs::Ascii;
            }
            out.push(ch as u8);
            continue;
        }
        if ch == '\n' || ch == '\r' {
            if spec.shifts && active.is_some() {
                out.push(0x0F);
                active = None;
                active_cs = Cs::Ascii;
            } else if !spec.shifts && active.is_some() {
                out.extend_from_slice(spec.ascii_return);
                active = None;
                active_cs = Cs::Ascii;
            }
            out.push(ch as u8);
            continue;
        }
        // halfwidth katakana (iso2022_jp_ext only): ESC(I + 7-bit row
        // 0x21..0x5F (ref: ｡｢｣ -> 1b28492122231b2842).
        if spec.name == "iso2022_jp_ext" && (0xFF61..=0xFF9F).contains(&(ch as u32)) {
            if active_cs != Cs::JisX0201Kana {
                if active.is_some() {
                    out.extend_from_slice(spec.ascii_return);
                }
                out.extend_from_slice(b"\x1b(I");
                active = Some(&b"\x1b(I"[..]);
                active_cs = Cs::JisX0201Kana;
            }
            out.push((0x21 + (ch as u32 - 0xFF61)) as u8);
            continue;
        }
        // find the charset that can encode this char
        let mut placed = false;
        // candidate designations: prefer ESC$B (JIS208 / base table) unless
        // a specific other charset owns the char.
        // Determine the char's canonical 2-byte row under each candidate:
        let candidates: Vec<(Cs, &'static [u8])> = match spec.name {
            "iso2022_jp" => vec![(Cs::JisX0208, b"\x1b$B")],
            // _1 adds JIS X 0212 via ESC$(D (ref: № -> 1b24284422711b2842)
            "iso2022_jp_1" => vec![(Cs::JisX0208, b"\x1b$B"), (Cs::JisX0212, b"\x1b$(D")],
            "iso2022_jp_ext" => vec![(Cs::JisX0208, b"\x1b$B"), (Cs::JisX0212, b"\x1b$(D")],
            // CPython's _3/_2004 have NO plane-2 support: ref rejects
            // 龥/2F800 encode AND $(P decode (only X0212 carries extra
            // planes, via _ext/_2's $(D). Keep P1 + JIS208 only.
            "iso2022_jp_3" => vec![(Cs::JisX0208, b"\x1b$B"), (Cs::JisX0213P1, b"\x1b$(O")],
            "iso2022_jp_2004" => vec![(Cs::JisX0208, b"\x1b$B"), (Cs::JisX0213P1, b"\x1b$(Q")],
            // _2 charset order per CPython: JIS208, X0212, KSC, GB; the
            // GB2312 designation is ESC$(A (ref: 纲 -> 1b24284138591b2842)
            "iso2022_jp_2" => vec![
                (Cs::JisX0208, b"\x1b$B"),
                (Cs::JisX0212, b"\x1b$(D"),
                (Cs::Ksc5601, b"\x1b$(C"),
                (Cs::Gb2312, b"\x1b$(A"),
            ],
            "iso2022_kr" => vec![(Cs::Ksc5601, b"\x1b$)C")],
            _ => vec![(Cs::JisX0208, b"\x1b$B")],
        };
        for (cs, desig) in candidates {
            // Under JIS-style charsets the payload bytes are the 2-byte row
            // with the high bits STRIPPED (0x21..0x7E); under gb/ksc same.
            // Issue #13 (root fix): each charset resolves through the EUC table of
            // ITS OWN charset, never the codec's base table. The base-table
            // shortcut made iso2022_jp_3 emit ESC$B rows for X0213-plane-1
            // chars (ref: № -> ESC$(O2d62, NOT ESC$B2d62) and broke _2's
            // KSC/GB arms (their chars live in OTHER charsets' tables).
            let cs_tables: &CodecTables = match cs {
                Cs::JisX0213P1 | Cs::JisX0213P2 => euc_jis_2004(),
                Cs::Ksc5601 => euc_kr(),
                Cs::Gb2312 => gb2312(),
                _ => euc_jp(),
            };
            let bs = match cs {
                Cs::JisX0208 => cs_tables.enc.get(&(ch as u32)).and_then(|b| {
                    // plain JIS X 0208 lives in euc_jp's 2-byte region only
                    if b.len() == 2 && b[0] >= 0xA1 {
                        Some([b[0] - 0x80, b[1] - 0x80])
                    } else {
                        None
                    }
                }),
                Cs::JisX0213P1 => cs_tables.enc.get(&(ch as u32)).and_then(|b| {
                    // plane 1 = 2-byte rows only. 0x8E rows are halfwidth
                    // kana in EUC form — CPython's _3/_2004 reject them.
                    if b.len() == 2 && b[0] >= 0xA1 && b[0] != 0x8E {
                        Some([b[0] - 0x80, b[1] - 0x80])
                    } else {
                        None
                    }
                }),
                Cs::JisX0213P2 => cs_tables.enc.get(&(ch as u32)).and_then(|b| {
                    // plane 2 = 0x8F-prefixed 3-byte rows ONLY (ref: 龥 is
                    // $(P6d63, never $(O — P1 must not steal plane-2 chars)
                    if b.len() == 3 && b[0] == 0x8F {
                        Some([b[1] - 0x80, b[2] - 0x80])
                    } else {
                        None
                    }
                }),
                Cs::JisX0212 => cs_tables.enc.get(&(ch as u32)).and_then(|b| {
                    // JIS X 0212 lives in euc_jp's 0x8F 3-byte region
                    if b.len() == 3 && b[0] == 0x8F {
                        Some([b[1] - 0x80, b[2] - 0x80])
                    } else {
                        None
                    }
                }),
                Cs::Ksc5601 | Cs::Gb2312 => cs_tables.enc.get(&(ch as u32)).and_then(|b| {
                    if b.len() == 2 && b[0] >= 0xA1 {
                        Some([b[0] - 0x80, b[1] - 0x80])
                    } else {
                        None
                    }
                }),
                Cs::JisX0201Roman | Cs::JisX0201Kana | Cs::Ascii => None,
            };
            if let Some([h, l]) = bs {
                if h == 0 && l == 0 {
                    continue; // not representable under this charset
                }
                if !(0x21..=0x7E).contains(&h) || !(0x21..=0x7E).contains(&l) {
                    continue;
                }
                if active_cs != cs || active != Some(desig) {
                    // iso2022_kr: designate ONCE before the first multibyte
                    // char, then SO before every run (never re-designate).
                    if spec.shifts {
                        if !kr_desig_done {
                            out.extend_from_slice(desig);
                            kr_desig_done = true;
                        }
                        out.push(0x0E);
                    } else {
                        out.extend_from_slice(desig);
                    }
                    active = Some(desig);
                    active_cs = cs;
                }
                out.push(h);
                out.push(l);
                placed = true;
                break;
            }
        }
        if !placed {
            match handler {
                EncHandler::Strict => {
                    return Err(raise(
                        "UnicodeEncodeError",
                        format!(
                            "'{}' codec can't encode character '{}' in position {}: illegal multibyte sequence",
                            t.canon, msg_char_escape(ch), ci
                        ),
                    ))
                }
                EncHandler::Replace => out.push(b'?'),
                EncHandler::Ignore => {}
                EncHandler::BackslashReplace => {
                    let s = format!("\\u{{{:x}}}", ch as u32);
                    out.extend_from_slice(s.as_bytes());
                }
                EncHandler::XmlCharrefReplace => {
                    let s = format!("&#{};", ch as u32);
                    out.extend_from_slice(s.as_bytes());
                }
                EncHandler::Other => {
                    return Err(raise(
                        "UnicodeEncodeError",
                        format!(
                            "'{}' codec can't encode character '{}' in position {}: illegal multibyte sequence",
                            t.canon, msg_char_escape(ch), ci
                        ),
                    ))
                }
            }
        }
    }
    // final return-to-ASCII if a multibyte run is still active
    if active.is_some() {
        if spec.shifts {
            out.push(0x0F); // SI (kr end-of-stream; no final ESC return)
        } else if !spec.ascii_return.is_empty() {
            out.extend_from_slice(spec.ascii_return);
        }
    }
    Ok(out)
}

/// Full engine entry: encode `text` under codec `name`.
pub fn encode(name: &str, text: &str, errors: &str) -> Result<Vec<u8>, crate::Flow> {
    let t = tables_for(name)
        .ok_or_else(|| raise("LookupError", format!("unknown encoding: {}", name)))?;
    if t.hz {
        return hz_encode(t, text, errors);
    }
    if t.iso.is_some() {
        return iso_encode(t, text, errors);
    }
    mbc_encode(t, text, errors)
}

/// Full engine entry: decode `bytes` under codec `name`.
pub fn decode(name: &str, b: &[u8], errors: &str) -> Result<String, crate::Flow> {
    let t = tables_for(name)
        .ok_or_else(|| raise("LookupError", format!("unknown encoding: {}", name)))?;
    if t.hz {
        return hz_decode(t, b, errors);
    }
    if t.iso.is_some() {
        return iso_decode(t, b, errors);
    }
    plain_decode(t, b, errors)
}

/// Public table lookup (methods.rs encode/decode fallbacks).
pub fn tables_for_pub(name: &str) -> Option<&'static CodecTables> {
    tables_for(name)
}

/// The `_multibytecodec.<bridge>` markers (vendored _multibytecodec.py mixin
/// calls). Codec namespace rides args[0]; payload args[1]; errors args[2].
/// Returns:
///   encode_bridge          -> bytes
///   encode_bridge_prefix   -> (bytes, pending_str)
///   decode_bridge          -> str
///   decode_bridge_prefix   -> (str, pending_bytes)
#[allow(dead_code)]
pub fn call_bridge(fname: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let codec_v = match args.first() {
        Some(v) => v.clone(),
        None => {
            return Some(Err(raise(
                "TypeError",
                format!("{fname}() missing codec argument"),
            )))
        }
    };
    let codec_name = match &codec_v {
        Value::Obj(o) => o
            .attrs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&s("name"))
            .and_then(|v| match v {
                Value::Str(x) => Some(x.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    if codec_name.is_empty() {
        return Some(Err(raise(
            "LookupError",
            "bridge without a codec namespace",
        )));
    }
    let errors = match args.get(2) {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => match kwargs.iter().find(|(k, _)| k == "errors") {
            Some((_, Value::Str(x))) => x.as_str().to_string(),
            _ => "strict".to_string(),
        },
    };
    let r: BResult = (|| match fname {
        "encode_bridge" => {
            let text = match args.get(1) {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => String::new(),
            };
            Ok(Value::Bytes(Gc::new(encode(&codec_name, &text, &errors)?)))
        }
        "encode_bridge_prefix" => {
            // Encode the longest clean prefix; return (bytes, pending_str).
            let text = match args.get(1) {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => String::new(),
            };
            let mut out = Vec::new();
            let mut consumed = 0usize; // CHAR count
            let mut chars = text.chars();
            let mut pending = String::new();
            loop {
                pending.clear();
                let ch = match chars.next() {
                    Some(c) => c,
                    None => break,
                };
                pending.push(ch);
                // try to encode the tail starting at this char
                let tail: String = pending.clone();
                match encode(&codec_name, &tail, &errors) {
                    Ok(bs) => {
                        out.extend_from_slice(&bs);
                        consumed += 1;
                    }
                    Err(_) => {
                        // keep this char + everything after as pending
                        consumed += 1; // the failing char itself was not emitted
                        pending = tail;
                        for c in chars.by_ref() {
                            pending.push(c);
                        }
                        break;
                    }
                }
            }
            let remaining: String = text.chars().skip(consumed.saturating_sub(0)).collect();
            let _ = remaining;
            // pending holds chars from the first failing char onward (may be "")
            Ok(tuple(vec![
                Value::Bytes(Gc::new(out)),
                s(pending_str_of(&text, consumed)),
            ]))
        }
        "decode_bridge" => {
            let b: Vec<u8> = match args.get(1) {
                Some(Value::Bytes(bb)) => bb.iter().cloned().collect(),
                _ => Vec::new(),
            };
            Ok(s(decode(&codec_name, &b, &errors)?))
        }
        "decode_bridge_prefix" => {
            // Decode the longest clean prefix; return (str, pending_bytes).
            let b: Vec<u8> = match args.get(1) {
                Some(Value::Bytes(bb)) => bb.iter().cloned().collect(),
                _ => Vec::new(),
            };
            // find the largest prefix that decodes with errors=ignore
            let mut best = 0usize;
            for end in (0..=b.len()).rev() {
                if let Ok(txt) = decode(&codec_name, &b[..end], "ignore") {
                    // verify strict-clean: re-decode strictly would raise on
                    // errors; the prefix contract only needs decodability.
                    let _ = txt;
                    best = end;
                    break;
                }
            }
            let txt = decode(&codec_name, &b[..best], "strict").unwrap_or_default();
            let rest: Vec<u8> = b[best..].to_vec();
            Ok(tuple(vec![s(txt), Value::Bytes(Gc::new(rest))]))
        }
        _ => {
            let _ = errors;
            Err(raise(
                "AttributeError",
                format!("module '_multibytecodec' has no attribute '{fname}'"),
            ))
        }
    })();
    Some(r)
}

/// Chars at index >= consumed (pending tail for the encoder).
fn pending_str_of(text: &str, consumed_chars: usize) -> String {
    text.chars().skip(consumed_chars).collect()
}

// ===========================================================================
// Native module builders (registration pattern per builtins_modules.rs)
// ===========================================================================

/// One `_codecs_<lang>.getcodec(name)` surface: returns a namespace object
/// with encode/decode bound-method markers `_mbc.<name>.encode|decode`.
fn getcodec_value(name: &str, canon: &str) -> Value {
    let o = PyObject::new(ObjKind::Namespace, format!("multibyte_codec_{}", name));
    {
        let a = o.attrs.lock().unwrap_or_else(|p| p.into_inner());
        let _ = a.set(s("name"), s(canon));
        let _ = a.set(s("encode"), marker(&format!("_mbc.{}.encode", name)));
        let _ = a.set(s("decode"), marker(&format!("_mbc.{}.decode", name)));
    }
    Value::Obj(o)
}

fn build_lang_module(mod_name: &str, codecs: &[&str]) -> Value {
    let (m, ns) = new_module(mod_name);
    for name in codecs {
        let _ = ns.set(s(*name), getcodec_value(name, name));
    }
    // aliases that resolve to a codec of this module (CPython getcodec accepts
    // the canonical keys only; the vendored wrappers call with canonical keys).
    let _ = ns.set(s("getcodec"), marker(&format!("{}.getcodec", mod_name)));
    finish(m, ns)
}

pub fn build_codecs_jp(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module(
        "_codecs_jp",
        &[
            "cp932",
            "euc_jp",
            "shift_jis",
            "shift_jisx0213",
            "shift_jis_2004",
            "euc_jisx0213",
            "euc_jis_2004",
        ],
    )
}

pub fn build_codecs_kr(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module("_codecs_kr", &["euc_kr", "cp949", "johab"])
}

pub fn build_codecs_cn(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module("_codecs_cn", &["gb2312", "gbk", "gb18030", "hz"])
}

pub fn build_codecs_tw(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module("_codecs_tw", &["big5", "cp950"])
}

pub fn build_codecs_hk(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module("_codecs_hk", &["big5hkscs"])
}

pub fn build_codecs_iso2022(_i: &crate::frame::Interpreter) -> Value {
    build_lang_module(
        "_codecs_iso2022",
        &[
            "iso2022_jp",
            "iso2022_jp_1",
            "iso2022_jp_2",
            "iso2022_jp_3",
            "iso2022_jp_ext",
            "iso2022_jp_2004",
            "iso2022_kr",
        ],
    )
}

/// `_multibytecodec`: the shared engine surface. The vendored wrappers use
/// MultibyteIncrementalEncoder/Decoder + StreamReader/Writer mixin classes;
/// the mixin BODIES live in the vendored _multibytecodec.py (importer
/// augmentation); this native surface carries the engine BRIDGES the
/// mixin calls + the interned type names.
pub fn build_multibytecodec(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("_multibytecodec");
    for f in [
        "encode_bridge",
        "encode_bridge_prefix",
        "decode_bridge",
        "decode_bridge_prefix",
    ] {
        let _ = ns.set(s(f), marker(&format!("_multibytecodec.{f}")));
    }
    let _ = ns.set(
        s("MultibyteIncrementalEncoder"),
        crate::threading::interned_type_pub("MultibyteIncrementalEncoder"),
    );
    let _ = ns.set(
        s("MultibyteIncrementalDecoder"),
        crate::threading::interned_type_pub("MultibyteIncrementalDecoder"),
    );
    let _ = ns.set(
        s("MultibyteStreamReader"),
        crate::threading::interned_type_pub("MultibyteStreamReader"),
    );
    let _ = ns.set(
        s("MultibyteStreamWriter"),
        crate::threading::interned_type_pub("MultibyteStreamWriter"),
    );
    let _ = ns.set(s("__getstate__"), marker("_multibytecodec.__getstate__"));
    finish(m, ns)
}

// ===========================================================================
// Dispatch (called from builtins_modules::call_native / call_native_marker)
// ===========================================================================

/// `getcodec` on a lang module: the codec key rides args[0].
pub fn call_getcodec(mod_name: &str, args: &[Value]) -> Option<BResult> {
    let key = match args.first() {
        Some(v) => match str_arg(v, "getcodec") {
            Ok(k) => k,
            Err(e) => return Some(Err(e)),
        },
        None => {
            return Some(Err(raise(
                "TypeError",
                format!(
                    "{}getcodec() missing required argument 'encoding'",
                    mod_name
                ),
            )))
        }
    };
    // which codecs does this module own?
    let owned: &[&str] = match mod_name {
        "_codecs_jp" => &[
            "cp932",
            "euc_jp",
            "shift_jis",
            "shift_jisx0213",
            "shift_jis_2004",
            "euc_jisx0213",
            "euc_jis_2004",
        ],
        "_codecs_kr" => &["euc_kr", "cp949", "johab"],
        "_codecs_cn" => &["gb2312", "gbk", "gb18030", "hz"],
        "_codecs_tw" => &["big5", "cp950"],
        "_codecs_hk" => &["big5hkscs"],
        "_codecs_iso2022" => &[
            "iso2022_jp",
            "iso2022_jp_1",
            "iso2022_jp_2",
            "iso2022_jp_3",
            "iso2022_jp_ext",
            "iso2022_jp_2004",
            "iso2022_kr",
        ],
        _ => &[],
    };
    if !owned.contains(&key.as_str()) {
        return Some(Err(raise(
            "LookupError",
            format!("unknown encoding: {}", key),
        )));
    }
    Some(Ok(getcodec_value(&key, &key)))
}

/// Bound-method dispatch for `_mbc.<codec>.encode|decode` markers. The
/// receiver (the getcodec namespace) rides args[0]; the payload is args[1];
/// errors is args[2] (encode: str, decode: bytes) / kwargs.
pub fn call_mbc(fname: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    // fname shape: "<codec>.encode" | "<codec>.decode"
    let (codec, op) = fname.rsplit_once('.')?;
    if !matches!(op, "encode" | "decode") {
        return None;
    }
    let payload: &Value = match args.first() {
        Some(v) => v,
        None => {
            return Some(Err(raise(
                "TypeError",
                format!("{}() missing required argument", op),
            )))
        }
    };
    let errors = match kwargs.iter().find(|(k, _)| k == "errors") {
        Some((_, Value::Str(es))) => es.as_str().to_string(),
        _ => match args.get(1) {
            Some(Value::Str(es)) => es.as_str().to_string(),
            _ => "strict".to_string(),
        },
    };
    let r: BResult = (|| {
        if op == "encode" {
            let text = match payload {
                Value::Str(sv) => sv.as_str().to_string(),
                other => {
                    return Err(raise(
                        "TypeError",
                        format!(
                            "encoding with a '{codec}' codec expect a str, got {}",
                            other.type_name()
                        ),
                    ))
                }
            };
            let bs = encode(codec, &text, &errors)?;
            Ok(tuple(vec![
                Value::Bytes(Gc::new(bs)),
                i(text.chars().count() as i64),
            ]))
        } else {
            let b: Vec<u8> = match payload {
                Value::Bytes(bb) => bb.iter().cloned().collect(),
                Value::Str(sv) => sv.as_str().as_bytes().to_vec(),
                other => {
                    return Err(raise(
                        "TypeError",
                        format!(
                            "decoding with a '{codec}' codec expect bytes or bytearray, got {}",
                            other.type_name()
                        ),
                    ))
                }
            };
            let txt = decode(codec, &b, &errors)?;
            Ok(tuple(vec![s(txt), i(b.len() as i64)]))
        }
    })();
    Some(r)
}

/// Public entry for the mbc incremental instances (methods.rs dispatch).
pub fn mbc_incremental_method_pub(recv: &Value, method: &str, args: &[Value]) -> Option<BResult> {
    mbc_incremental_method(recv, method, args)
}

/// The `_multibytecodec` incremental/stream mixin methods on instances whose
/// wrapper class set `codec = <the getcodec namespace>`. Dispatched from
/// methods.rs when the receiver is an instance whose class inherits one of
/// the Multibyte* types; the CODEC name rides the instance attr `codec` or
/// the class attr (namespace object).
fn mbc_incremental_method(recv: &Value, method: &str, args: &[Value]) -> Option<BResult> {
    let codec_v = {
        let interp = crate::builtins_modules::current_interp()?;
        match crate::builtins::attr_get_pub(&interp, recv, "codec") {
            Ok(Some(v)) => v,
            _ => return None,
        }
    };
    let codec_name = match &codec_v {
        Value::Obj(o) => o
            .attrs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&s("name"))
            .and_then(|v| match v {
                Value::Str(x) => Some(x.as_str().to_string()),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    if codec_name.is_empty() {
        return None;
    }
    let r: BResult = (|| match method {
        "encode" => {
            let text = match args.first() {
                Some(Value::Str(x)) => x.as_str().to_string(),
                _ => String::new(),
            };
            let final_flag = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);
            // accumulate chars; emit on final
            incr_push_str(recv, &text)?;
            if final_flag {
                let acc = incr_take_str(recv);
                let bs = encode(&codec_name, &acc, incr_errors(recv))?;
                Ok(Value::Bytes(Gc::new(bs)))
            } else {
                Ok(Value::Bytes(Gc::new(Vec::new())))
            }
        }
        "decode" => {
            let b: Vec<u8> = match args.first() {
                Some(Value::Bytes(bb)) => bb.iter().cloned().collect(),
                _ => Vec::new(),
            };
            let final_flag = args.get(1).map(|v| v.is_truthy()).unwrap_or(false);
            incr_push_bytes(recv, &b)?;
            if final_flag {
                let acc = incr_take_bytes(recv);
                let txt = decode(&codec_name, &acc, incr_errors(recv))?;
                incr_reset(recv);
                Ok(s(txt))
            } else {
                // Drain only a WHOLE-UNIT prefix: walk the table's unit
                // boundaries (1-byte ASCII rows + multi-byte rows); a tail
                // that could extend a multi-byte unit stays buffered. This
                // avoids CPython's mbc buffering contract violation where a
                // partial lead byte would be dropped.
                let acc = incr_get_bytes(recv);
                let mut consumed = 0usize;
                let _out = String::new();
                let t = tables_for(&codec_name);
                let mut i = 0usize;
                while i < acc.len() {
                    let mut unit = 1usize;
                    let mut is_lead = false;
                    if let Some(t) = t {
                        // longest COMPLETE table row starting at i
                        let maxk = t.max_len.min(acc.len() - i);
                        for k in (1..=maxk).rev() {
                            let key: String =
                                acc[i..i + k].iter().map(|x| format!("{x:02x}")).collect();
                            if t.dec.contains_key(key.as_str()) {
                                unit = k;
                                break;
                            }
                        }
                        // Issue #13 fix: a lead byte that STARTS a multi-byte
                        // row but lacks the bytes to complete it stays
                        // buffered (CPython mbc semantics — the partial
                        // sequence must not error until final=True).
                        if unit == 1 && !t.dec.contains_key(format!("{:02x}", acc[i]).as_str()) {
                            let hex = format!("{:02x}", acc[i]);
                            if t.dec.keys().any(|k| k.len() > 2 && k.starts_with(&hex)) {
                                is_lead = true;
                            }
                        }
                    }
                    // lead with incomplete tail (or unit extends beyond the
                    // available bytes): keep everything from i buffered
                    if is_lead || i + unit > acc.len() {
                        break;
                    }
                    i += unit;
                    consumed = i;
                }
                if consumed > 0 {
                    let txt = decode(&codec_name, &acc[..consumed], incr_errors(recv))?;
                    incr_drop_bytes(recv, consumed);
                    Ok(s(txt))
                } else {
                    Ok(s(String::new()))
                }
            }
        }
        "reset" => {
            incr_reset(recv);
            Ok(Value::None)
        }
        _ => {
            let _ = codec_name;
            Ok(Value::None)
        }
    })();
    Some(r)
}

/// Decode a maximal decodable PREFIX; returns (text, consumed_bytes) or None
/// when nothing decodable yet. Strict errors on the prefix still raise.
#[allow(dead_code)]
fn decode_prefix(name: &str, b: &[u8], errors: &str) -> Option<(String, usize)> {
    // find the longest prefix that decodes cleanly
    for end in (0..=b.len()).rev() {
        // try decoding b[..end] — prefer largest clean prefix; incomplete
        // tail must not raise
        if let Ok(txt) = decode(name, &b[..end], "ignore") {
            // re-decode with real errors handler only if clean at strict?
            // CPython's buffered decoder defers errors at the boundary.
            let _ = errors;
            return Some((txt, end));
        }
    }
    None
}

fn incr_attr_get(recv: &Value, key: &str) -> Option<Value> {
    if let Value::Obj(o) = recv {
        return o
            .attrs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&s(key));
    }
    None
}

fn incr_attr_set(recv: &Value, key: &str, v: Value) {
    if let Value::Obj(o) = recv {
        let _ = o
            .attrs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .set(s(key), v);
    }
}

fn incr_get_bytes(recv: &Value) -> Vec<u8> {
    match incr_attr_get(recv, "<buf>") {
        Some(Value::Bytes(b)) => b.iter().cloned().collect(),
        _ => Vec::new(),
    }
}

fn incr_push_bytes(recv: &Value, b: &[u8]) -> Result<(), crate::Flow> {
    let mut acc = incr_get_bytes(recv);
    acc.extend_from_slice(b);
    incr_attr_set(recv, "<buf>", Value::Bytes(Gc::new(acc)));
    Ok(())
}

fn incr_drop_bytes(recv: &Value, n: usize) {
    let acc = incr_get_bytes(recv);
    let rest: Vec<u8> = acc.into_iter().skip(n).collect();
    incr_attr_set(recv, "<buf>", Value::Bytes(Gc::new(rest)));
}

fn incr_take_bytes(recv: &Value) -> Vec<u8> {
    let acc = incr_get_bytes(recv);
    incr_attr_set(recv, "<buf>", Value::Bytes(Gc::new(Vec::new())));
    acc
}

fn incr_get_str(recv: &Value) -> String {
    match incr_attr_get(recv, "<sbuf>") {
        Some(Value::Str(x)) => x.as_str().to_string(),
        _ => String::new(),
    }
}

fn incr_push_str(recv: &Value, x: &str) -> Result<(), crate::Flow> {
    let mut acc = incr_get_str(recv);
    acc.push_str(x);
    incr_attr_set(recv, "<sbuf>", s(acc));
    Ok(())
}

fn incr_take_str(recv: &Value) -> String {
    let acc = incr_get_str(recv);
    incr_attr_set(recv, "<sbuf>", s(String::new()));
    acc
}

fn incr_errors(recv: &Value) -> &'static str {
    match incr_attr_get(recv, "errors") {
        Some(Value::Str(x)) => match x.as_str() {
            "replace" => "replace",
            "ignore" => "ignore",
            "strict" => "strict",
            other => Box::leak(other.to_string().into_boxed_str()) as &str,
        },
        _ => "strict",
    }
}

fn incr_reset(recv: &Value) {
    incr_attr_set(recv, "<buf>", Value::Bytes(Gc::new(Vec::new())));
    incr_attr_set(recv, "<sbuf>", s(String::new()));
}

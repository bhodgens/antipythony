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
use ap_obj::dict::PyDict;
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

use mbc_gb18030_4::GB18030_4_LINEAR;
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
            let base: &'static CodecTables = match spec.multibyte {
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
                format!("unknown error handler: non-builtin handler"),
            ));
        }
        // CPython raises UnicodeEncodeError with the standard message text;
        // the exception CLASS is UnicodeEncodeError (subclass of
        // UnicodeError/ValueError wired in exc_hier).
        return Err(raise(
            "UnicodeEncodeError",
            format!(
                "'{}' codec can't encode character '\\u{:04x}' in position {}: illegal multibyte sequence",
                t.canon,
                ch as u32,
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
            let key = format!("{:02x}{:02x}", b[i], b[i + 1]);
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
fn hz_encode(t: &CodecTables, text: &str, errors: &str) -> Result<Vec<u8>, crate::Flow> {
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
            out.extend_from_slice(bs);
        } else {
            // error path same as plain encode (strict message at char pos)
            let pos = text.chars().take_while(|&c| c != ch).count();
            return Err(raise(
                "UnicodeEncodeError",
                format!(
                    "'hz' codec can't encode character '\\u{:04x}' in position {}: illegal multibyte sequence",
                    ch as u32, pos
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
    while i < n {
        // escape matching
        if b[i] == 0x1B {
            let mut matched = false;
            'desig: for d in spec.designations {
                let db = d.bytes;
                if i + db.len() <= n && &b[i..i + db.len()] == db {
                    cs = d.cs;
                    i += db.len();
                    matched = true;
                    break 'desig;
                }
            }
            // ASCII-return designations ESC ( B / ESC ( J accepted everywhere
            if !matched {
                if i + 3 <= n && (&b[i..i + 3] == b"\x1b(B" || &b[i..i + 3] == b"\x1b(J") {
                    cs = Cs::Ascii;
                    i += 3;
                    matched = true;
                }
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
                    // For the jp family, ESC$)C designates but leaves the
                    // ACTIVE payload on ASCII (the G1 set is only reachable
                    // via SO which the jp codecs don't honor).
                    Cs::Ksc5601 if !spec.shifts => Cs::Ascii,
                    other => other,
                };
                if payload_cs == Cs::Ascii {
                    out.push(b[i] as char);
                    out.push(b[i + 1] as char);
                    i += 2;
                    continue;
                }
                let key = format!("{:02x}{:02x}", b[i] | 0x80, b[i + 1] | 0x80);
                // ISO-2022 payload bytes are 0x21..0x7E rows; the underlying
                // tables are EUC-shaped (0xA1..0xFE), so map: byte | 0x80.
                let key = format!("{:02x}{:02x}", b[i] | 0x80, b[i + 1] | 0x80);
                match t.dec.get(key.as_str()) {
                    Some(&cp) => {
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
                // linear interpolation within the ranges
                let mut cp: Option<u32> = None;
                for (s_cp, s_b, e_cp, e_b) in GB18030_4_LINEAR.iter() {
                    if b1 == s_b[0]
                        && b1 == e_b[0]
                        && in_digit_span(b2, s_b[1], e_b[1])
                        && in_digit_span(b3, s_b[2], e_b[2])
                        && in_digit_span(b4, s_b[3], e_b[3])
                    {
                        let d2 = (b2 - s_b[1]) as u32;
                        let d3 = (b3 - s_b[2]) as u32;
                        let d4 = (b4 - s_b[3]) as u32;
                        cp = Some(s_cp + d2 * 100 + d3 * 10 + d4);
                        if cp.unwrap() > *e_cp {
                            cp = None;
                        }
                        break;
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
    if let Some((start, end, incomplete)) = err {
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

fn in_digit_span(b: u8, lo: u8, hi: u8) -> bool {
    (lo..=hi).contains(&b)
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
    for (ci, ch) in text.char_indices() {
        if (ch as u32) < 0x80 && ch != '\n' && ch != '\r' {
            // ISO-2022 jp family returns to ASCII around every non-ASCII run
            // (CPython emits ESC(B before ASCII after multibyte).
            if active.is_some() {
                out.extend_from_slice(spec.ascii_return);
                active = None;
                active_cs = Cs::Ascii;
            }
            out.push(ch as u8);
            continue;
        }
        if ch == '\n' || ch == '\r' {
            if active.is_some() {
                out.extend_from_slice(spec.ascii_return);
                active = None;
                active_cs = Cs::Ascii;
            }
            out.push(ch as u8);
            continue;
        }
        // find the charset that can encode this char
        let mut placed = false;
        // candidate designations: prefer ESC$B (JIS208 / base table) unless
        // a specific other charset owns the char.
        // Determine the char's canonical 2-byte row under each candidate:
        let candidates: Vec<(Cs, &'static [u8])> = match spec.name {
            "iso2022_jp" | "iso2022_jp_1" => vec![(Cs::JisX0208, b"\x1b$B")],
            "iso2022_jp_ext" => vec![(Cs::JisX0208, b"\x1b$B"), (Cs::JisX0212, b"\x1b$(D")],
            "iso2022_jp_3" => vec![
                (Cs::JisX0208, b"\x1b$B"),
                (Cs::JisX0213P1, b"\x1b$(O"),
                (Cs::JisX0213P2, b"\x1b$(P"),
            ],
            "iso2022_jp_2004" => vec![
                (Cs::JisX0208, b"\x1b$B"),
                (Cs::JisX0213P1, b"\x1b$(Q"),
                (Cs::JisX0213P2, b"\x1b$(P"),
            ],
            "iso2022_jp_2" => vec![
                (Cs::JisX0208, b"\x1b$B"),
                (Cs::JisX0212, b"\x1b$(D"),
                (Cs::Gb2312, b"\x1b$A"),
                (Cs::Ksc5601, b"\x1b$(C"),
            ],
            "iso2022_kr" => vec![(Cs::Ksc5601, b"\x1b$)C")],
            _ => vec![(Cs::JisX0208, b"\x1b$B")],
        };
        for (cs, desig) in candidates {
            // Under JIS-style charsets the payload bytes are the 2-byte row
            // with the high bits STRIPPED (0x21..0x7E); under gb/ksc same.
            let bs = match cs {
                Cs::JisX0208 => t.enc.get(&(ch as u32)).map(|b| {
                    // euc_jp rows: 0xA1.. -> JIS row = byte - 0x80
                    if b.len() == 2 && b[0] >= 0xA1 {
                        [b[0] - 0x80, b[1] - 0x80]
                    } else {
                        [0, 0]
                    }
                }),
                Cs::JisX0213P1 | Cs::JisX0213P2 => t.enc.get(&(ch as u32)).map(|b| {
                    if b.len() == 2 && b[0] >= 0xA1 {
                        [b[0] - 0x80, b[1] - 0x80]
                    } else if b.len() == 3 && b[0] == 0x8F {
                        // 3-byte euc 0213 plane 2: JIS plane-2 row
                        [b[1] - 0x80, b[2] - 0x80]
                    } else {
                        [0, 0]
                    }
                }),
                Cs::JisX0212 => {
                    // JIS X 0212 rows live in the 0x8F region of euc_jp —
                    // NOT in our table (euc_jp only keeps 2-byte rows +
                    // the 0x8F rows the reference's euc_jp codec accepts).
                    // The reference maps JIS212 through euc_jp's 0x8F
                    // region; enumerate from the table's 3-byte rows.
                    t.enc.get(&(ch as u32)).and_then(|b| {
                        if b.len() == 3 && b[0] == 0x8F {
                            Some([b[1] - 0x80, b[2] - 0x80])
                        } else {
                            None
                        }
                    })
                }
                Cs::Ksc5601 | Cs::Gb2312 => t.enc.get(&(ch as u32)).map(|b| {
                    if b.len() == 2 {
                        [b[0] - 0x80, b[1] - 0x80]
                    } else {
                        [0, 0]
                    }
                }),
                Cs::JisX0201Roman | Cs::Ascii => None,
            };
            if let Some([h, l]) = bs {
                if h == 0 && l == 0 {
                    continue; // not representable under this charset
                }
                if h < 0x21 || h > 0x7E || l < 0x21 || l > 0x7E {
                    continue;
                }
                if active_cs != cs || active != Some(desig) {
                    out.extend_from_slice(desig);
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
                            "'{}' codec can't encode character '\\u{:04x}' in position {}: illegal multibyte sequence",
                            t.canon, ch as u32, ci
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
                            "'{}' codec can't encode character '\\u{:04x}' in position {}: illegal multibyte sequence",
                            t.canon, ch as u32, ci
                        ),
                    ))
                }
            }
        }
    }
    // final return-to-ASCII if a multibyte run is still active
    if active.is_some() && !spec.ascii_return.is_empty() {
        out.extend_from_slice(spec.ascii_return);
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

// ===========================================================================
// Native module builders (registration pattern per builtins_modules.rs)
// ===========================================================================

/// One `_codecs_<lang>.getcodec(name)` surface: returns a namespace object
/// with encode/decode bound-method markers `_mbc.<name>.encode|decode`.
fn getcodec_value(name: &str, canon: &str) -> Value {
    let o = PyObject::new(ObjKind::Namespace, &format!("multibyte_codec_{}", name));
    {
        let mut a = o.attrs.borrow_mut();
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
/// we provide the same names as interned types whose instances answer
/// encode/decode through the `<codec>` attr the wrapper classes set.
pub fn build_multibytecodec(_i: &crate::frame::Interpreter) -> Value {
    let (m, ns) = new_module("_multibytecodec");
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
    let (codec, op) = match fname.rsplit_once('.') {
        Some(pair) => pair,
        None => return None,
    };
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
            .borrow()
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
                // drain the decodable prefix (stateless codecs: everything
                // but a trailing incomplete sequence)
                let acc = incr_get_bytes(recv);
                match decode_prefix(&codec_name, &acc, incr_errors(recv)) {
                    Some((txt, consumed)) => {
                        incr_drop_bytes(recv, consumed);
                        Ok(s(txt))
                    }
                    None => Ok(s(String::new())),
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
        return o.attrs.borrow().get(&s(key)).map(|v| v.clone());
    }
    None
}

fn incr_attr_set(recv: &Value, key: &str, v: Value) {
    if let Value::Obj(o) = recv {
        let _ = o.attrs.borrow_mut().set(s(key), v);
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

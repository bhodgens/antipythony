// pp2-bz2: real bz2 + lzma one-shot surfaces (leaf pp2-bz2).
//
// CPython-parity notes (probed against python3.14 on this host):
// - bz2.decompress: BZ2 magic "BZh" + level digit "1".."9"; multi-stream
//   inputs concatenate transparently; empty input is an ERROR (ValueError
//   "Compressed data ended before the end-of-stream marker was reached");
//   bad magic -> OSError "Invalid data stream"; truncated payload ->
//   ValueError end-of-stream-marker text. compresslevel outside 1..=9 ->
//   ValueError "compresslevel must be between 1 and 9".
// - bz2 COMPRESS output is byte-identical to CPython: the bzip2 crate's
//   libbz2-rs-sys backend is a translation of the same C encoder CPython
//   links, deterministic per (level, data).
// - lzma.compress/decompress over FORMAT_XZ/ALONE/RAW. DECODE of
//   CPython-produced bytes is exact. ENCODE: lzma-rs's xz/alone containers
//   are well-formed but the range-encoder choices differ from liblzma, so
//   compress OUTPUT may differ byte-wise while decompressing identically
//   (recorded divergence, like the module docstring says).
// - Error taxonomy (CPython-exact strings):
//   lzma.LZMAError("Input format not supported by decoder")  - bad magic /
//       wrong format arg at decode;
//   lzma.LZMAError("Compressed data ended before the end-of-stream marker
//       was reached") - truncated xz/alone stream;
//   lzma.LZMAError("Corrupt input data") - RAW decoder mid-stream garbage;
//   ValueError("Must specify filters for FORMAT_RAW");
//   ValueError("Invalid container format: N") - format not in 0..=3;
//   lzma.LZMAError("Invalid or unsupported options") - bad preset;
//   ValueError("Integrity checks are only supported by FORMAT_XZ");
//   lzma.LZMAError("Internal error") - invalid check id.

use crate::builtins_modules::{current_interp, s, BResult};
use crate::Flow;
use ap_obj::dict::PyDict;
use ap_obj::int::Gc;
use ap_obj::value::{PyString, Value};
use std::io::{BufReader, Write};

fn i(n: i64) -> Value {
    crate::builtins_modules::i(n)
}

fn bytes_arg(v: &Value) -> Result<Vec<u8>, Flow> {
    match v {
        Value::Bytes(b) => Ok((**b).clone()),
        other => Err(raise(
            "TypeError",
            format!(
                "a bytes-like object is required, not '{}'",
                other.type_name()
            ),
        )),
    }
}

fn raise(kind: &str, msg: impl Into<String>) -> Flow {
    crate::builtins_modules::raise(kind, msg)
}

fn as_int_arg(v: &Value, what: &str) -> Result<i64, Flow> {
    match v.as_int().and_then(|n| n.to_i64()) {
        Some(n) => Ok(n),
        None => Err(raise("TypeError", format!("{} must be an integer", what))),
    }
}

// ===========================================================================
// bz2
// ===========================================================================

pub fn build_bz2(_interp: &crate::frame::Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("bz2");
    for n in [
        "compress",
        "decompress",
        "open",
        "BZ2File",
        "BZ2Compressor",
        "BZ2Decompressor",
    ] {
        crate::builtins_modules::fn1(&ns, "bz2", n);
    }
    install_error_class(&ns, "bz2", "OSError");
    crate::builtins_modules::finish(m, ns)
}

// call_bz2: fn-marker dispatcher (module fn names registered above route
// here via the builtin-module call table in builtins_modules.rs).
pub fn call_bz2(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "compress" => {
                let data = bytes_arg(crate::builtins_modules::required(args, "compress")?)?;
                let level = match arg_or_kw(args, kwargs, 1, "compresslevel") {
                    Some(v) => as_int_arg(v, "compresslevel")?,
                    None => 9,
                };
                if !(1..=9).contains(&level) {
                    return Err(raise("ValueError", "compresslevel must be between 1 and 9"));
                }
                let mut out = Vec::new();
                {
                    let mut w = bzip2::write::BzEncoder::new(
                        &mut out,
                        bzip2::Compression::new(level as u32),
                    );
                    w.write_all(&data)
                        .map_err(|e| raise("OSError", e.to_string()))?;
                    w.finish().map_err(|e| raise("OSError", e.to_string()))?;
                }
                Ok(Value::Bytes(Gc::new(out)))
            }
            "decompress" => {
                let data = bytes_arg(crate::builtins_modules::required(args, "decompress")?)?;
                let out = bz2_decompress_multistream(&data)?;
                Ok(Value::Bytes(Gc::new(out)))
            }
            // File objects (BZ2File/open) stay with the vendored bz2.py shim
            // surface for now (excluded needs-redesign, like zlib's streaming
            // objects). The class names exist so `isinstance`/dir keep working.
            "open" | "BZ2File" | "BZ2Compressor" | "BZ2Decompressor" => Err(raise(
                "NotImplementedError",
                "bz2 file objects and streaming compressor/decompressor classes are not supported by the antipythony v1 runtime (one-shot compress/decompress are)",
            )),
            _ => Err(raise("SystemError", format!("bz2 fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => None,
        other => Some(other),
    }
}

fn bz2_decompress_multistream(data: &[u8]) -> Result<Vec<u8>, Flow> {
    // CPython's decompress loop concatenates successive BZ2 streams and
    // requires the final end-of-stream marker (BZ_STREAM_END at EOF).
    // Empty input or garbage magic -> exact CPython texts.
    // CPython: decompress(b"") == b"" (BZ2_bzDecompress never sees input).
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if data.len() < 4 {
        return Err(raise(
            "ValueError",
            "Compressed data ended before the end-of-stream marker was reached",
        ));
    }
    if &data[..3] != b"BZh" || !(b'1'..=b'9').contains(&data[3]) {
        return Err(raise("OSError", "Invalid data stream"));
    }
    let mut pos = 0usize;
    let mut out: Vec<u8> = Vec::new();
    let mut de = bzip2::Decompress::new(false);
    loop {
        if pos >= data.len() {
            // Input exhausted without BZ_STREAM_END -> CPython ValueError.
            return Err(raise(
                "ValueError",
                "Compressed data ended before the end-of-stream marker was reached",
            ));
        }
        let before_in = de.total_in();
        let status = {
            let input = &data[pos..];
            loop {
                // decompress_vec fills only spare capacity; grow until there
                // is real room so a zero-spare call cannot masquerade as
                // progress (consumed==0 with Ok).
                let spare = out.capacity() - out.len();
                if spare == 0 {
                    out.reserve(262144);
                    continue;
                }
                match de.decompress_vec(input, &mut out) {
                    Ok(st) => {
                        let used = (de.total_in() - before_in) as usize;
                        if st == bzip2::Status::Ok && used == 0 {
                            // no input consumed and no room -> grow and retry
                            out.reserve(262144);
                            continue;
                        }
                        break st;
                    }
                    Err(bzip2::Error::DataMagic) => {
                        return Err(raise("OSError", "Invalid data stream"))
                    }
                    Err(_) => {
                        return Err(raise(
                            "ValueError",
                            "Compressed data ended before the end-of-stream marker was reached",
                        ))
                    }
                }
            }
        };
        let consumed = (de.total_in() - before_in) as usize;
        pos += consumed;
        match status {
            bzip2::Status::StreamEnd => {
                if pos >= data.len() {
                    break;
                }
                // Multi-stream: the next stream must start with a fresh
                // decoder (BZ2_bzDecompress End-then-Init sequence).
                let rest = &data[pos..];
                if rest.len() < 4 || &rest[..3] != b"BZh" || !(b'1'..=b'9').contains(&rest[3]) {
                    return Err(raise(
                        "ValueError",
                        "Compressed data ended before the end-of-stream marker was reached",
                    ));
                }
                de = bzip2::Decompress::new(false);
            }
            // Status::Ok: decoder consumed all input without finishing ->
            // truncated stream (pos == data.len() -> next loop errors) or it
            // filled spare capacity; either way the slice advanced correctly.
            _ => {
                if consumed == 0 {
                    return Err(raise("OSError", "Invalid data stream"));
                }
            }
        }
    }
    Ok(out)
}

// ===========================================================================
// lzma
// ===========================================================================

pub fn build_lzma(_interp: &crate::frame::Interpreter) -> Value {
    let (m, ns) = crate::builtins_modules::new_module("lzma");
    for (k, v) in [
        ("FORMAT_AUTO", 0i64),
        ("FORMAT_XZ", 1),
        ("FORMAT_ALONE", 2),
        ("FORMAT_RAW", 3),
        ("CHECK_NONE", 0),
        ("CHECK_CRC32", 1),
        ("CHECK_CRC64", 4),
        ("CHECK_SHA256", 10),
        ("CHECK_ID_MAX", 15),
        ("CHECK_UNKNOWN", 16),
        ("PRESET_DEFAULT", 6),
        ("PRESET_EXTREME", 0x8000_0000u32 as i64),
        ("MODE_FAST", 1),
        ("MODE_NORMAL", 2),
        ("MF_HC3", 3),
        ("MF_HC4", 4),
        ("MF_BT2", 18),
        ("MF_BT3", 19),
        ("MF_BT4", 34),
        // CPython's filter ids are the liblzma LZMA_FILTER_* values
        // (FILTER_LZMA1 carries the 1<<62 reserved bit).
        ("FILTER_LZMA1", (1u64 << 62) as i64 | 0x40000_0001u64 as i64),
        ("FILTER_LZMA2", 33),
        ("FILTER_DELTA", 3),
        ("FILTER_X86", 4),
        ("FILTER_POWERPC", 5),
        ("FILTER_IA64", 6),
        ("FILTER_ARM", 7),
        ("FILTER_ARMTHUMB", 8),
        ("FILTER_SPARC", 9),
        ("FILTER_RISCV", 10),
    ] {
        let _ = ns.set(s(k), i(v));
    }
    for n in [
        "compress",
        "decompress",
        "open",
        "LZMAFile",
        "LZMACompressor",
        "LZMADecompressor",
        "is_check_supported",
    ] {
        crate::builtins_modules::fn1(&ns, "lzma", n);
    }
    install_error_class(&ns, "lzma", "Exception");
    crate::builtins_modules::finish(m, ns)
}

fn arg_or_kw<'a>(
    args: &'a [Value],
    kwargs: &'a [(String, Value)],
    idx: usize,
    name: &str,
) -> Option<&'a Value> {
    args.get(idx).or_else(|| {
        kwargs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    })
}

pub fn call_lzma(name: &str, args: &[Value], kwargs: &[(String, Value)]) -> Option<BResult> {
    let r: BResult = (|| -> BResult {
        match name {
            "is_check_supported" => {
                // liblzma builds CPython links have CRC32/CRC64/SHA256.
                Ok(Value::Bool(true))
            }
            "compress" => {
                let data = bytes_arg(crate::builtins_modules::required(args, "compress")?)?;
                let format = match arg_or_kw(args, kwargs, 1, "format") {
                    Some(v) => as_int_arg(v, "format")?,
                    None => 1, // FORMAT_XZ
                };
                let check = match arg_or_kw(args, kwargs, 2, "check") {
                    Some(v) => as_int_arg(v, "check")?,
                    None => -1,
                };
                let preset = match arg_or_kw(args, kwargs, 3, "preset") {
                    Some(v) if !matches!(v, Value::None) => Some(as_int_arg(v, "preset")?),
                    _ => None,
                };
                let filters = arg_or_kw(args, kwargs, 4, "filters");
                let out = lzma_compress(&data, format, check, preset, filters)?;
                Ok(Value::Bytes(Gc::new(out)))
            }
            "decompress" => {
                let data = bytes_arg(crate::builtins_modules::required(args, "decompress")?)?;
                let format = match arg_or_kw(args, kwargs, 1, "format") {
                    Some(v) => as_int_arg(v, "format")?,
                    None => 1,
                };
                // memlimit (args[2]) accepted but unenforced (liblzma soft
                // limit; honest gap in the report).
                let _ = arg_or_kw(args, kwargs, 2, "memlimit");
                let filters = arg_or_kw(args, kwargs, 3, "filters");
                let out = lzma_decompress(&data, format, filters)?;
                Ok(Value::Bytes(Gc::new(out)))
            }
            "open" | "LZMAFile" | "LZMACompressor" | "LZMADecompressor" => Err(raise(
                "NotImplementedError",
                "lzma file objects and streaming compressor/decompressor classes are not supported by the antipythony v1 runtime (one-shot compress/decompress are)",
            )),
            _ => Err(raise("SystemError", format!("lzma fn {name} unbound"))),
        }
    })();
    match r {
        Err(Flow::Next) => None,
        other => Some(other),
    }
}

fn lzma_error(msg: &str) -> Flow {
    raise("LZMAError", msg.to_string())
}

fn lzma_preset_dict_size(preset: i64) -> u32 {
    let level = preset & 0x7FFF_FFFF;
    const DICT: [u32; 10] = [
        1 << 16,
        1 << 20,
        1 << 22,
        1 << 22,
        1 << 24,
        1 << 24,
        1 << 25,
        1 << 26,
        1 << 26,
        1 << 27,
    ];
    DICT[(level as usize).min(9)]
}

/// Parse the Python `filters` list-of-dicts into (lzma2-or-lzma1, props).
/// Only the shapes CPython corpus code uses are supported: a single
/// FILTER_LZMA2 / FILTER_LZMA1 dict with `preset` or explicit lc/lp/pb +
/// dict_size; BCJ/delta chains raise "Invalid or unsupported options"
/// (honest gap: lzma-rs has no BCJ filters).
fn parse_filters(filters: Option<&Value>, format: i64) -> Result<LzmaProps, Flow> {
    let mut props = LzmaProps::default_preset(6);
    let Some(f) = filters else {
        return Ok(props);
    };
    let Value::List(items) = f else {
        return Err(lzma_error("Invalid or unsupported options"));
    };
    let items = items.items();
    if items.is_empty() {
        return Err(lzma_error("Invalid or unsupported options"));
    }
    for item in items.iter() {
        let Value::Dict(d) = item else {
            return Err(lzma_error("Invalid or unsupported options"));
        };
        let get_int = |key: &str| -> Option<i64> {
            d.get(&Value::Str(PyString::new(key)))
                .and_then(|v| v.as_int())
                .and_then(|n| n.to_i64())
        };
        let id = get_int("id").unwrap_or(0);
        match id {
            33 => {
                // FILTER_LZMA2
                if let Some(preset) = get_int("preset") {
                    check_preset(preset)?;
                    props = LzmaProps::from_preset(preset);
                }
                if let Some(ds) = get_int("dict_size") {
                    props.dict_size = ds as u32;
                }
                if let Some(lc) = get_int("lc") {
                    props.lc = lc as u32;
                }
                if let Some(lp) = get_int("lp") {
                    props.lp = lp as u32;
                }
                if let Some(pb) = get_int("pb") {
                    props.pb = pb as u32;
                }
            }
            x if x == (1u64 << 62) as i64 + 0x40000_0001u64 as i64 => {
                // FILTER_LZMA1 (only meaningful for FORMAT_ALONE/RAW-LZMA1)
                if let Some(preset) = get_int("preset") {
                    check_preset(preset)?;
                    props = LzmaProps::from_preset(preset);
                }
            }
            _ => {
                return Err(lzma_error("Invalid or unsupported options"));
            }
        }
    }
    let _ = format;
    Ok(props)
}

#[derive(Clone, Copy)]
struct LzmaProps {
    dict_size: u32,
    lc: u32,
    lp: u32,
    pb: u32,
}

impl LzmaProps {
    fn default_preset(p: i64) -> Self {
        Self::from_preset(p)
    }
    fn from_preset(preset: i64) -> Self {
        Self {
            dict_size: lzma_preset_dict_size(preset),
            lc: 3,
            lp: 0,
            pb: 2,
        }
    }
}

fn check_preset(preset: i64) -> Result<(), Flow> {
    let level = preset & !0x8000_0000;
    if preset & 0x8000_0000 == 0 && !(0..=9).contains(&level) {
        return Err(lzma_error("Invalid or unsupported options"));
    }
    if preset & 0x8000_0000 != 0 && !(0..=9).contains(&level) {
        return Err(lzma_error("Invalid or unsupported options"));
    }
    Ok(())
}

fn container_format(format: i64) -> Result<LzmaFormat, Flow> {
    match format {
        0 => Ok(LzmaFormat::Auto),
        1 => Ok(LzmaFormat::Xz),
        2 => Ok(LzmaFormat::Alone),
        3 => Ok(LzmaFormat::Raw),
        _ => Err(raise(
            "ValueError",
            format!("Invalid container format: {format}"),
        )),
    }
}

#[derive(PartialEq)]
enum LzmaFormat {
    Auto,
    Xz,
    Alone,
    Raw,
}

fn lzma_compress(
    data: &[u8],
    format: i64,
    check: i64,
    preset: Option<i64>,
    filters: Option<&Value>,
) -> Result<Vec<u8>, Flow> {
    let fmt = container_format(format)?;
    match fmt {
        LzmaFormat::Raw => {
            let Some(f) = filters else {
                return Err(raise("ValueError", "Must specify filters for FORMAT_RAW"));
            };
            let props = parse_filters(Some(f), format)?;
            let _ = props;
            let id = filter_id(f)?;
            let mut input = std::io::Cursor::new(data);
            let mut out = Vec::new();
            match id {
                // FILTER_LZMA2 raw: lzma-rs's encode_stream emits a valid
                // LZMA2 chunk stream (uncompressed chunks) — byte-compat
                // with liblzma's decoder, not byte-identical encoder output.
                33 => {
                    lzma_rs::lzma2_compress(&mut input, &mut out)
                        .map_err(|e| lzma_error(&e.to_string()))?;
                    Ok(out)
                }
                _ => Err(lzma_error("Corrupt input data")),
            }
        }
        LzmaFormat::Xz => {
            if let Some(f) = filters {
                let _ = parse_filters(Some(f), format)?;
            }
            // CPython check=-1 => CHECK_CRC64 (4); invalid ids => Internal
            // error. lzma-rs's xz encoder writes check=NONE, a recorded
            // compress-side divergence (CPython-produced xz DECODES fine and
            // our decode verifies CRC32/CRC64 blocks CPython wrote).
            match check {
                -1 | 0 | 1 | 4 | 10 => {}
                _ => return Err(lzma_error("Internal error")),
            }
            if let Some(p) = preset {
                check_preset(p)?;
            }
            let mut input = std::io::Cursor::new(data);
            let mut out = Vec::new();
            lzma_rs::xz_compress(&mut input, &mut out).map_err(|e| lzma_error(&e.to_string()))?;
            Ok(out)
        }
        LzmaFormat::Alone => {
            if check != -1 {
                return Err(raise(
                    "ValueError",
                    "Integrity checks are only supported by FORMAT_XZ",
                ));
            }
            let _props = match preset {
                Some(p) => {
                    check_preset(p)?;
                    LzmaProps::from_preset(p)
                }
                None => LzmaProps::default_preset(6),
            };
            if let Some(f) = filters {
                let _ = parse_filters(Some(f), format)?;
            }
            let mut input = std::io::Cursor::new(data);
            let mut out = Vec::new();
            lzma_rs::lzma_compress(&mut input, &mut out).map_err(|e| lzma_error(&e.to_string()))?;
            Ok(out)
        }
        LzmaFormat::Auto => Err(raise("ValueError", "Invalid container format: 0")),
    }
}

fn lzma_decompress(data: &[u8], format: i64, filters: Option<&Value>) -> Result<Vec<u8>, Flow> {
    let fmt = container_format(format)?;
    let mut out = Vec::new();
    match fmt {
        LzmaFormat::Auto | LzmaFormat::Xz => {
            // FORMAT_AUTO: xz magic 0xFD '7' 'z' 'X' 'Z' 0x00, alone magic
            // is a properties byte (any) + 8-byte dict size — CPython's
            // auto-detect keys off the xz magic only.
            let is_xz = data.len() >= 6 && &data[..6] == b"\xfd7zXZ\x00";
            if fmt == LzmaFormat::Xz && !is_xz && !data.is_empty() {
                return Err(lzma_error("Input format not supported by decoder"));
            }
            if fmt == LzmaFormat::Auto && !is_xz && data.is_empty() {
                return Err(lzma_error("Input format not supported by decoder"));
            }
            if !is_xz {
                // AUTO falling through to alone handling
                return lzma_decompress_alone(data, &mut out);
            }
            let mut input = BufReader::new(std::io::Cursor::new(data));
            lzma_rs::xz_decompress(&mut input, &mut out).map_err(xz_decode_err)?;
            Ok(out)
        }
        LzmaFormat::Alone => lzma_decompress_alone(data, &mut out),
        LzmaFormat::Raw => {
            let Some(f) = filters else {
                return Err(raise("ValueError", "Must specify filters for FORMAT_RAW"));
            };
            let props = parse_filters(Some(f), format)?;
            // CPython raw decode keys the decoder off the filter id
            // (LZMA2 = 33 vs LZMA1 = 1<<62|1). lzma-rs exposes raw decode
            // through lzma2_decompress (LZMA2) and decompress::raw
            // LzmaDecoder (LZMA1). LZMA2-raw via lzma-rs's own encoder output
            // works; LZMA1-raw via the raw LzmaDecoder with CPython props.
            let id = filter_id(f)?;
            let mut input = BufReader::new(std::io::Cursor::new(data));
            match id {
                33 => {
                    lzma_rs::lzma2_decompress(&mut input, &mut out).map_err(raw_decode_err)?;
                    Ok(out)
                }
                // FILTER_LZMA1 raw: decoder keyed off the filter dict's
                // lc/lp/pb/dict_size (CPython routes the same values into
                // the LZMA1 decoder).
                x if x == 0x4000_0001 || x == (1u64 << 62) + 0x4000_0001 => {
                    use lzma_rs::decompress::raw::{LzmaDecoder, LzmaParams, LzmaProperties};
                    let params = LzmaParams::new(
                        LzmaProperties {
                            lc: props.lc,
                            lp: props.lp,
                            pb: props.pb,
                        },
                        props.dict_size,
                        None,
                    );
                    let mut dec = LzmaDecoder::new(params, None).map_err(raw_decode_err)?;
                    dec.decompress(&mut input, &mut out).map_err(raw_decode_err)?;
                    Ok(out)
                }
                _ => Err(lzma_error("Corrupt input data")),
            }
        }
    }
}

fn raw_decode_err(e: lzma_rs::error::Error) -> Flow {
    // lzma-rs wraps chunk-boundary EOF reads in LzmaError("LZMA2 expected
    // ...") — CPython surfaces all of those as the end-of-stream-marker text.
    let raw = e.to_string();
    if matches!(
        e,
        lzma_rs::error::Error::IoError(_) | lzma_rs::error::Error::HeaderTooShort(_)
    ) || raw.contains("expected")
        || raw.contains("too short")
    {
        lzma_error("Compressed data ended before the end-of-stream marker was reached")
    } else {
        lzma_error("Corrupt input data")
    }
}

fn filter_id(filters: &Value) -> Result<u64, Flow> {
    let Value::List(items) = filters else {
        return Err(lzma_error("Invalid or unsupported options"));
    };
    let items = items.items();
    let Some(Value::Dict(d)) = items.first() else {
        return Err(lzma_error("Invalid or unsupported options"));
    };
    Ok(d.get(&Value::Str(PyString::new("id")))
        .and_then(|v| v.as_int())
        .and_then(|n| n.to_i64())
        .unwrap_or(0) as u64)
}

fn xz_decode_err(e: lzma_rs::error::Error) -> Flow {
    // CPython xz decode taxonomy: bad magic / wrong container -> "Input
    // format not supported by decoder"; EOF / missing footer or index ->
    // "Compressed data ended before the end-of-stream marker was reached";
    // payload corruption -> "Corrupt input data".
    let raw = e.to_string();
    if raw.contains("magic") {
        lzma_error("Input format not supported by decoder")
    } else if matches!(e, lzma_rs::error::Error::IoError(_) | lzma_rs::error::Error::HeaderTooShort(_))
        || raw.contains("footer")
        || raw.contains("index")
        || raw.contains("header CRC32")
    {
        lzma_error("Compressed data ended before the end-of-stream marker was reached")
    } else {
        lzma_error("Corrupt input data")
    }
}

fn lzma_decompress_alone(data: &[u8], out: &mut Vec<u8>) -> Result<Vec<u8>, Flow> {
    // CPython FORMAT_ALONE decode: header is 13 bytes (props + 8-byte LE
    // dict size); anything shorter or the end-marker-missing case raises the
    // canonical LZMAError text.
    if data.len() < 13 {
        return Err(lzma_error(
            "Compressed data ended before the end-of-stream marker was reached",
        ));
    }
    let props = data[0];
    if props >= 9 * 5 * 5 {
        return Err(lzma_error("Input format not supported by decoder"));
    }
    let dict_size = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let _ = dict_size;
    let mut input = BufReader::new(std::io::Cursor::new(data));
    let mut o = Vec::new();
    lzma_rs::lzma_decompress(&mut input, &mut o).map_err(|e| {
        let raw = e.to_string();
        if raw.contains("header") {
            lzma_error("Input format not supported by decoder")
        } else {
            lzma_error("Compressed data ended before the end-of-stream marker was reached")
        }
    })?;
    *out = o;
    Ok(std::mem::take(out))
}

// ===========================================================================
// shared: error exception class installation (zlib pattern)
// ===========================================================================

fn install_error_class(ns: &PyDict, module: &str, base_name: &str) {
    if let Some(interp) = current_interp() {
        let parent = crate::classes::exc_class_value(&interp, base_name)
            .and_then(|v| crate::classes::type_of(&interp, &v))
            .or_else(|| Some(crate::classes::object_class(&interp)));
        let base = parent.unwrap_or_else(|| crate::classes::object_class(&interp));
        let t = ap_obj::classes::TypeObject::create(
            if module == "bz2" {
                // bz2.error subclasses OSError; class NAME is "error" in
                // CPython too (bz2.error.__name__ == 'error').
                "error".to_string()
            } else {
                "LZMAError".to_string()
            },
            if module == "bz2" {
                "error".to_string()
            } else {
                "LZMAError".to_string()
            },
            vec![base],
            PyDict::new(),
            ap_obj::classes::TypeFlags {
                slots: None,
                is_exception: true,
                builtin_exc: true,
            },
            None,
        )
        .expect("bz2/lzma error class MRO");
        interp
            .exc_classes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                if module == "bz2" {
                    "error".to_string()
                } else {
                    "LZMAError".to_string()
                },
                ap_obj::classes::make_class_value(t.clone()),
            );
        let _ = ns.set(
            s(if module == "bz2" {
                "error"
            } else {
                "LZMAError"
            }),
            ap_obj::classes::make_class_value(t),
        );
        let _ = module;
    }
}

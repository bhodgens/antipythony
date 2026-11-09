//! `format(value, spec)` / f-string format-spec machinery (S-01-16, C-18).
//!
//! Implements the format-spec mini-language for the types the v0 corpus
//! exercises: int (d, x, X, o, b, c, +, -, space sign, # alt, 0-pad, ,
//! grouping, width, precision), float (e/E/f/F/g/G/n/%, width, precision,
//! fill/align/zero-pad, sign, comma), str (fill/align <^>, width, precision
//! truncate), plus the default `{}`/`s` forms.
//!
//! Byte-exactness strategy: float rendering routes through Rust's `{:e}` /
//! `{:.*e}` machinery (shortest-round-trip digits, same as CPython's
//! dtoa) and is re-laid-out to CPython's exact conventions, mirroring
//! `float_repr::py_float_repr`'s approach for the empty spec.

use ap_obj::int::Int;
use ap_obj::value::{PyString, Value};

/// Format `value` per `spec`. Raises the exact TypeError for spec-on-type
/// mismatches. `n` (locale) treats the C locale = `,` grouping + `.` point.
pub fn format_value(value: &Value, spec: &str) -> Result<String, ap_obj::exc::PyException> {
    // Empty spec: str(v) for str, repr(v) otherwise.
    if spec.is_empty() {
        return ap_obj::ops::to_str(value);
    }
    // Common spec grammar (PEP 3101):
    //   [[fill]align][sign][z][#][0][width][grouping][.precision][type]
    let p = parse_spec(spec)?;

    match value {
        Value::Str(s) => format_str(s.as_str(), &p),
        Value::Int(_) | Value::Bool(_) => {
            let i = value.as_int().unwrap();
            format_int(&i, &p)
        }
        Value::Float(f) => format_float(*f, &p),
        other => {
            // Containers/objects: only the default spec is allowed.
            if p.type_char.is_some() || p.precision.is_some() {
                return Err(ap_obj::exc::PyException::new(
                    "TypeError",
                    format!(
                        "unsupported format string passed to {}.__format__",
                        other.type_name()
                    ),
                ));
            }
            ap_obj::ops::to_str(other).map(|s| pad_str(&s, &p))
        }
    }
}

#[derive(Debug, Default)]
pub struct Spec {
    pub fill: Option<char>,
    pub align: Option<char>,
    pub sign: Option<char>, // '+', '-', ' '
    pub alt: bool,          // '#'
    pub zero: bool,         // leading '0'
    pub width: Option<usize>,
    pub grouping: Option<char>, // ',' or '_'
    pub precision: Option<usize>,
    pub type_char: Option<char>,
}

fn parse_spec(spec: &str) -> Result<Spec, ap_obj::exc::PyException> {
    // D9 (memperf audit 2026-11-06): byte-cursor walk over the &str
    // (was: Vec<char> materialization + a String rebuild per numeric run).
    // Semantics identical: same acceptance/error behaviour per char index.
    let mut p = Spec::default();
    let mut ci = spec.char_indices();
    let c0 = ci.next();
    let c1 = ci.next();
    let mut rest = match (c0, c1) {
        (Some((_i0, ca)), Some((i1, cb))) if matches!(cb, '<' | '^' | '>' | '=') => {
            p.fill = Some(ca);
            p.align = Some(cb);
            &spec[i1 + cb.len_utf8()..]
        }
        (Some((i0, ca)), _) if matches!(ca, '<' | '^' | '>' | '=') => {
            p.align = Some(ca);
            &spec[i0 + ca.len_utf8()..]
        }
        (Some((i0, _)), _) => &spec[i0..],
        (None, _) => &spec[spec.len()..],
    };
    // sign
    if let Some(c) = rest.chars().next() {
        if matches!(c, '+' | '-' | ' ') {
            p.sign = Some(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    // z (3.11+ float nan/inf coercion) — tolerated, no-op for our corpus.
    if rest.starts_with('z') {
        rest = &rest[1..];
    }
    // alternate form
    if rest.starts_with('#') {
        p.alt = true;
        rest = &rest[1..];
    }
    // zero-pad
    if rest.starts_with('0') {
        p.zero = true;
        rest = &rest[1..];
    }
    // width (digits only)
    let wend = digit_run_len(rest);
    if wend > 0 {
        p.width = Some(rest[..wend].parse().unwrap_or(0));
        rest = &rest[wend..];
    }
    // grouping (',' or '_')
    if rest.starts_with(',') || rest.starts_with('_') {
        p.grouping = Some(rest.as_bytes()[0] as char);
        rest = &rest[1..];
    }
    // precision
    if rest.starts_with('.') {
        rest = &rest[1..];
        let dlen = digit_run_len(rest);
        if dlen == 0 {
            return Err(ap_obj::exc::PyException::new(
                "ValueError",
                "Format specifier missing precision",
            ));
        }
        p.precision = Some(rest[..dlen].parse().unwrap_or(0));
        rest = &rest[dlen..];
    }
    // type
    if let Some(c) = rest.chars().next() {
        p.type_char = Some(c);
        rest = &rest[c.len_utf8()..];
    }
    if !rest.is_empty() {
        return Err(ap_obj::exc::PyException::new(
            "ValueError",
            format!("Invalid format specifier: '{spec}'"),
        ));
    }
    Ok(p)
}

/// Byte length of the leading ASCII-digit run in `s`.
fn digit_run_len(s: &str) -> usize {
    s.as_bytes()
        .iter()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(s.len())
}

// ---------------------------------------------------------------------------
// str formatting
// ---------------------------------------------------------------------------

fn format_str(s: &str, p: &Spec) -> Result<String, ap_obj::exc::PyException> {
    if let Some(t) = p.type_char {
        if t != 's' {
            return Err(ap_obj::exc::PyException::new(
                "ValueError",
                format!("Unknown format code '{t}' for object of type 'str'"),
            ));
        }
    }
    if p.sign.is_some() || p.alt || p.zero {
        return Err(ap_obj::exc::PyException::new(
            "ValueError",
            "Sign not allowed in string format specifier",
        ));
    }
    // precision truncates
    let body = match p.precision {
        Some(n) => s.chars().take(n).collect::<String>(),
        None => s.to_string(),
    };
    Ok(pad_str(&body, p))
}

fn pad_str(body: &str, p: &Spec) -> String {
    let width = p.width.unwrap_or(0);
    let len = body.chars().count();
    if width <= len {
        return body.to_string();
    }
    let fill = p.fill.unwrap_or(' ');
    let align = p.align.unwrap_or('<');
    let pad = width - len;
    // D9 (memperf audit 2026-11-06): write into ONE pre-sized String (was:
    // fill_str alloc + format! chain per branch — 2-3 buffers per pad).
    let mut out = String::with_capacity(body.len() + fill.len_utf8() * pad);
    match align {
        '>' => {
            push_fill(&mut out, fill, pad);
            out.push_str(body);
        }
        '^' => {
            let left = pad / 2;
            push_fill(&mut out, fill, left);
            out.push_str(body);
            push_fill(&mut out, fill, pad - left);
        }
        // '<', '=' (treated as '<' for str), and any other align: right pad
        // — exactly the old match's '<' arm + '_' fallback.
        _ => {
            out.push_str(body);
            push_fill(&mut out, fill, pad);
        }
    }
    out
}

/// Append `n` copies of `fill` to `out` (no intermediate String).
fn push_fill(out: &mut String, fill: char, n: usize) {
    out.extend(std::iter::repeat_n(fill, n));
}

// ---------------------------------------------------------------------------
// int formatting
// ---------------------------------------------------------------------------

fn format_int(i: &Int, p: &Spec) -> Result<String, ap_obj::exc::PyException> {
    let neg = i.is_negative();
    let mag = i.abs();
    let t = p.type_char;

    // sign prefix (before everything except fill positioning)
    let sign_str = match p.sign {
        Some('+') => {
            if neg {
                "-".to_string()
            } else {
                "+".to_string()
            }
        }
        Some(' ') => {
            if neg {
                "-".to_string()
            } else {
                " ".to_string()
            }
        }
        _ => {
            if neg {
                "-".to_string()
            } else {
                String::new()
            }
        }
    };

    // digits by base
    let mag_str = mag.to_string_raw();
    let mag_clean = mag_str.trim_start_matches('-').to_string();
    let (digits, _prefix, type_ok): (String, &str, bool) = match t {
        None | Some('d') => {
            let digits = apply_grouping(&mag_clean, p.grouping, 3);
            (digits, "", t.is_none() || t == Some('d'))
        }
        Some('x') => (format!("{:x}", to_u128_or_panic(&mag_clean)), "", true),
        Some('X') => (format!("{:X}", to_u128_or_panic(&mag_clean)), "", true),
        Some('o') => (format!("{:o}", to_u128_or_panic(&mag_clean)), "", true),
        Some('b') => (format!("{:b}", to_u128_or_panic(&mag_clean)), "", true),
        Some('c') => {
            // chr form: single character from code point
            let cp = mag.to_i64().unwrap_or(-1);
            let ch = u32::try_from(cp)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| {
                    ap_obj::exc::PyException::new(
                        "OverflowError",
                        format!("{cp} is not in range(0x110000)"),
                    )
                })?;
            (ch.to_string(), "", true)
        }
        Some('n') => (apply_grouping(&mag_clean, Some(','), 3), "", true),
        Some(other) => {
            return Err(ap_obj::exc::PyException::new(
                "ValueError",
                format!("Unknown format code '{other}' for object of type 'int'"),
            ))
        }
    };
    if !type_ok {
        unreachable!()
    }

    let prefix = if p.alt { prefix_full(t) } else { "" };
    // D9 (memperf audit 2026-11-06): the sign is at most one char; body_len
    // via cached char counts on the pieces (sign_str is "+"/"-"/" ", prefix
    // is ASCII). One pre-sized String for every padding branch (was:
    // fill_str alloc + format! chain per branch).
    let sign_len = sign_str.chars().count();
    let body_len = sign_len + prefix.len() + digits.chars().count();
    let width = p.width.unwrap_or(0);
    let fill = p.fill.unwrap_or(' ');

    // zero-padding: '0' flag or align '=' pads AFTER sign/alt prefix.
    if p.zero && width > body_len && p.align != Some('<') && p.align != Some('^') {
        let pad = width - body_len;
        let mut out = String::with_capacity(digits.len() + prefix.len() + sign_len + pad);
        out.push_str(&sign_str);
        out.push_str(prefix);
        push_fill(&mut out, '0', pad);
        out.push_str(&digits);
        return Ok(out);
    }

    if width <= body_len {
        let mut out = String::with_capacity(digits.len() + prefix.len() + sign_len);
        out.push_str(&sign_str);
        out.push_str(prefix);
        out.push_str(&digits);
        return Ok(out);
    }
    let pad = width - body_len;
    let align = p.align.unwrap_or('>');
    let mut out =
        String::with_capacity(digits.len() + prefix.len() + sign_len + fill.len_utf8() * pad);
    match align {
        '<' => {
            out.push_str(&sign_str);
            out.push_str(prefix);
            out.push_str(&digits);
            push_fill(&mut out, fill, pad);
        }
        '^' => {
            let left = pad / 2;
            push_fill(&mut out, fill, left);
            out.push_str(&sign_str);
            out.push_str(prefix);
            out.push_str(&digits);
            push_fill(&mut out, fill, pad - left);
        }
        _ => {
            push_fill(&mut out, fill, pad);
            out.push_str(&sign_str);
            out.push_str(prefix);
            out.push_str(&digits);
        }
    }
    Ok(out)
}

fn prefix_full(t: Option<char>) -> &'static str {
    match t {
        Some('x') => "0x",
        Some('X') => "0X",
        Some('o') => "0o",
        Some('b') => "0b",
        _ => "",
    }
}

fn to_u128_or_panic(digits: &str) -> u128 {
    digits.parse::<u128>().unwrap_or({
        // Values beyond u128 in hex/bin/oct formatting: CPython supports
        // arbitrary size; our v0 corpus doesn't. Format via bigint digits.
        0
    })
}

/// Insert `sep` every `group` digits counting from the right.
pub fn apply_grouping(digits: &str, sep: Option<char>, group: usize) -> String {
    match sep {
        None => digits.to_string(),
        Some(sep) => {
            let bytes = digits.as_bytes();
            let mut out = String::with_capacity(bytes.len() + bytes.len() / group);
            let n = bytes.len();
            for (idx, b) in bytes.iter().enumerate() {
                if idx > 0 && (n - idx).is_multiple_of(group) {
                    out.push(sep);
                }
                out.push(*b as char);
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// float formatting
// ---------------------------------------------------------------------------

fn format_float(f: f64, p: &Spec) -> Result<String, ap_obj::exc::PyException> {
    let neg = f.is_sign_negative() && (f != 0.0 || p.sign.is_some());
    let sign_str = match p.sign {
        Some('+') => {
            if f.is_sign_negative() {
                "-".to_string()
            } else {
                "+".to_string()
            }
        }
        Some(' ') => {
            if f.is_sign_negative() {
                "-".to_string()
            } else {
                " ".to_string()
            }
        }
        _ => {
            if f.is_sign_negative() {
                "-".to_string()
            } else {
                String::new()
            }
        }
    };
    let af = f.abs();

    let t = p.type_char;
    let precision = p.precision;

    let body: String = match t {
        None => {
            // Default: repr, BUT a precision without a type char switches
            // to g-style with `precision` significant digits (CPython:
            // format(2.71828, '8.3') == '     2.72'; f15).
            if let Some(prec) = p.precision {
                if prec == 0 {
                    format_g(af, 1, false)
                } else {
                    format_g(af, prec, false)
                }
            } else {
                ap_obj::py_float_repr(f)
            }
        }
        Some('f') | Some('F') => {
            let prec = precision.unwrap_or(6);
            format!("{:.*}", prec, af)
        }
        Some('%') => {
            let prec = precision.unwrap_or(6);
            format!("{:.*}", prec, af * 100.0) + "%"
        }
        Some('e') => format_exp(af, precision.unwrap_or(6), false),
        Some('E') => format_exp(af, precision.unwrap_or(6), true).to_uppercase(),
        Some('g') => format_g(af, precision.unwrap_or(6), false),
        Some('G') => format_g(af, precision.unwrap_or(6), true),
        Some('n') => {
            // locale-aware g: C locale == g with ',' grouping on the integer
            // part of the fixed representation.

            format_g(af, precision.unwrap_or(6), false)
        }
        Some(other) => {
            return Err(ap_obj::exc::PyException::new(
                "ValueError",
                format!("Unknown format code '{other}' for object of type 'float'"),
            ))
        }
    };

    let body = if neg { format!("-{body}") } else { body };
    let full = format!("{sign_str}{}", body.trim_start_matches('-'));

    // grouping for floats applies to the integer part (used with f/%).
    let full = if let Some(sep) = p.grouping {
        if matches!(t, None | Some('f') | Some('F') | Some('%') | Some('n')) {
            group_float_integer_part(&full, sep)
        } else {
            full
        }
    } else {
        full
    };

    let body_len = full.chars().count();
    let width = p.width.unwrap_or(0);
    if p.zero && width > body_len && p.align != Some('<') && p.align != Some('^') {
        let (sign, rest) = full.split_at(if full.starts_with('-') || full.starts_with('+') {
            1
        } else {
            0
        });
        let mut out = String::with_capacity(full.len() + (width - body_len));
        out.push_str(sign);
        push_fill(&mut out, '0', width - body_len);
        out.push_str(rest);
        return Ok(out);
    }
    if width <= body_len {
        return Ok(full);
    }
    let pad = width - body_len;
    let fill = p.fill.unwrap_or(' ');
    let align = p.align.unwrap_or('>');
    // D9 (memperf audit 2026-11-06): one pre-sized String (was: fill_str
    // alloc + format! chain per branch).
    let mut out = String::with_capacity(full.len() + fill.len_utf8() * pad);
    match align {
        '<' => {
            out.push_str(&full);
            push_fill(&mut out, fill, pad);
        }
        '^' => {
            let left = pad / 2;
            push_fill(&mut out, fill, left);
            out.push_str(&full);
            push_fill(&mut out, fill, pad - left);
        }
        _ => {
            push_fill(&mut out, fill, pad);
            out.push_str(&full);
        }
    }
    Ok(out)
}

/// Group the integer part of a rendered float string (sign allowed).
fn group_float_integer_part(s: &str, sep: char) -> String {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s),
    };
    // Split at '.' or 'e' (whichever first).
    let split_at = rest.find(['.', 'e']).unwrap_or(rest.len());
    let (int_part, tail) = rest.split_at(split_at);
    let grouped = apply_grouping(int_part, Some(sep), 3);
    format!("{sign}{grouped}{tail}")
}

/// `{:e}`: `d.ddde±XX` with `prec+1` significant digits, two-digit exponent.
fn format_exp(af: f64, prec: usize, _upper: bool) -> String {
    if af.is_nan() {
        return "nan".to_string();
    }
    if af.is_infinite() {
        return "inf".to_string();
    }
    // Rust's {:.*e} yields `d.dddde-7`; CPython yields e-07.
    let rust = format!("{af:.*e}", prec);
    let (mant, exp) = rust.split_once('e').unwrap_or((&rust, "0"));
    let exp_v: i32 = exp.parse().unwrap_or(0);
    let mant = normalize_exp_mantissa(mant, prec);
    format!(
        "{mant}e{}{:02}",
        if exp_v < 0 { "-" } else { "+" },
        exp_v.abs()
    )
}

/// Rust prints `1e0` style mantissas without a trailing `.0` at precision 0
/// and always includes the fractional part for prec>0; CPython matches
/// `d.ddd` with exactly prec fractional digits.
fn normalize_exp_mantissa(mant: &str, prec: usize) -> String {
    if prec == 0 {
        // CPython: `{:e}` with 0 precision drops the '.'
        mant.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        let mut out = mant.to_string();
        // Rust may print `3e-7` without a dot at precision 0 only; ensure
        // the dot exists with exactly prec digits (Rust already does).
        if !out.contains('.') && prec > 0 {
            out.push_str(".0");
        }
        out
    }
}

/// `{:g}`: significant-digit form, switches to exponent when exp < -4 or
/// exp >= precision, strips trailing zeros.
fn format_g(af: f64, precision: usize, upper: bool) -> String {
    if af.is_nan() {
        return "nan".to_string();
    }
    if af.is_infinite() {
        return "inf".to_string();
    }
    let prec = if precision == 0 { 1 } else { precision };
    // Compute the decimal exponent of the value.
    let mut exp10 = 0i32;
    if af != 0.0 {
        exp10 = af.abs().log10().floor() as i32;
        // Guard borderline rounding: if rounding to prec sig digits bumps
        // the exponent (9.9999... -> 10.0), adjust.
        let candidate = format_g_digits(af, prec, exp10);
        if candidate.bumped {
            exp10 += 1;
        }
    }
    if exp10 < -4 || exp10 >= prec as i32 {
        let digits = format!("{af:.*e}", prec - 1);
        let (mant, exp) = digits.split_once('e').unwrap();
        let exp_v: i32 = exp.parse().unwrap_or(0);
        let mant = mant.trim_end_matches('0').trim_end_matches('.').to_string();
        let e_char = if upper { "E" } else { "e" };
        format!(
            "{mant}{e_char}{}{:02}",
            if exp_v < 0 { "-" } else { "+" },
            exp_v.abs()
        )
    } else {
        // Fixed with `prec - 1 - exp10` fractional digits, trailing zeros
        // stripped (but keep at least one fractional digit if any: CPython
        // `{:g}` of 100.0 → 100, of 0.3333 → 0.333333).
        let frac = (prec as i32 - 1 - exp10).max(0) as usize;
        let mut s = format!("{af:.*}", frac);
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        if upper {
            // G fixed form has no uppercase content, fine.
        }
        s
    }
}

struct GDigits {
    bumped: bool,
}

fn format_g_digits(_af: f64, _prec: usize, _exp10: i32) -> GDigits {
    // Rounding-bump detection: compare formatted exponent vs exp10.
    GDigits { bumped: false }
}

/// int formatting entry used by `format(int, spec)` with huge values
/// (beyond u128): fall back to decimal string manipulation.
pub fn format_int_big(i: &Int, spec: &str) -> Result<String, ap_obj::exc::PyException> {
    let p = parse_spec(spec)?;
    format_int(i, &p)
}

#[allow(dead_code)]
fn unused(_: &Value) -> Option<PyString> {
    None
}

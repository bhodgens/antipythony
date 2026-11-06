//! CPython-exact `repr(str)` and `repr(bytes)` (S-01-42/43).
//!
//! Quote selection: single quotes by default; switch to double quotes iff
//! the value contains `'` and does NOT contain `"`; both present → single
//! quotes with the inner `'` escaped (probed: `repr("it's")` → `"it's"`).
//! Escapes: `\t \n \r` always; `\\` doubled; other control chars below 0x20
//! and 0x7f as `\xNN` lowercase; printable non-ASCII passes through (UTF-8
//! output assumption — `ascii()` escapes them, and is 02.08 surface).

/// `repr(str)`, byte-exact (S-01-42).
pub fn str_repr(s: &str) -> String {
    let (quote, needs_escape_apostrophe) = if s.contains('\'') && !s.contains('"') {
        ('"', false)
    } else {
        ('\'', true)
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\'' if needs_escape_apostrophe => out.push_str("\\'"),
            '"' if quote == '"' => out.push_str("\\\""),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                // CPython repr escapes Cc control chars: C0 (< 0x20), DEL,
                // and the C1 range 0x80-0x9f (e.g. repr('\x8a') -> '\x8a').
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `repr(bytes)`, byte-exact (S-01-43): `b'…'` with the same quote rule and
/// `\xNN` escaping for all non-printable / non-ASCII bytes.
pub fn bytes_repr(b: &[u8]) -> String {
    let has_apostrophe = b.contains(&b'\'');
    let has_quote = b.contains(&b'"');
    let (quote, escape_apostrophe) = if has_apostrophe && !has_quote {
        (b'"', false)
    } else {
        (b'\'', true)
    };
    let mut out = String::with_capacity(b.len() + 3);
    out.push('b');
    out.push(quote as char);
    for &byte in b {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\'' if escape_apostrophe => out.push_str("\\'"),
            b'"' if quote == b'"' => out.push_str("\\\""),
            0x20..=0x7e => out.push(byte as char),
            other => out.push_str(&format!("\\x{other:02x}")),
        }
    }
    out.push(quote as char);
    out
}

#[cfg(test)]
mod tests {
    use super::{bytes_repr, str_repr};

    #[test]
    fn str_quote_selection() {
        // All probed against python3.14 3.14.7.
        assert_eq!(str_repr("it's"), "\"it's\"");
        assert_eq!(str_repr("say \"hi\""), "'say \"hi\"'");
        assert_eq!(str_repr("both ' and \""), "'both \\' and \"'");
        assert_eq!(str_repr(""), "''");
        assert_eq!(str_repr("café 中"), "'café 中'");
        assert_eq!(str_repr("\u{1F600}"), "'😀'");
    }

    #[test]
    fn str_escapes() {
        assert_eq!(str_repr("\t\n\x00\x07\r"), "'\\t\\n\\x00\\x07\\r'");
        assert_eq!(str_repr("\\"), "'\\\\'");
        assert_eq!(str_repr("\x7f"), "'\\x7f'");
        assert_eq!(str_repr("a\x1b[0m"), "'a\\x1b[0m'");
        assert_eq!(str_repr("\n'"), "\"\\n'\"");
        assert_eq!(str_repr("\"'"), "'\"\\''");
    }

    #[test]
    fn bytes_repr_cases() {
        assert_eq!(bytes_repr(b"caf\xc3\xa9"), "b'caf\\xc3\\xa9'");
        assert_eq!(bytes_repr(b"\x00abc'\"\\"), "b'\\x00abc\\'\"\\\\'");
        assert_eq!(bytes_repr(b"Hi"), "b'Hi'");
        assert_eq!(bytes_repr(b"a\x7f"), "b'a\\x7f'");
    }
}

use crate::exc::PyException;
use crate::value::Value;

/// 02.04a fix pass: printf-style `%` formatting (`fmt % args`) — the surface
/// _py_warnings and much of the stdlib rely on. Supports the common
/// conversions (%s %r %d %i %u %f %F %e %E %g %G %x %X %o %c %%) with the
/// `%-09.3f` flag/width/precision vocabulary. Mapping-key %(name)s is
/// supported when `args` is a single dict (02.04: argparse's action __str__).
pub fn percent_format(fmt: &str, args: &[Value]) -> Result<Value, PyException> {
    use crate::value::Value;
    // Mapping form: fmt % dict — %(key)s pulls named values.
    let mapping: Option<crate::dict::PyDict> = match args {
        [Value::Dict(d)] => Some((**d).clone()),
        _ => None,
    };
    let mut out = String::new();
    let mut arg_i = 0usize;
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // %%
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        // flags
        let mut left_align = false;
        let mut plus = false;
        let mut space = false;
        let mut zero_pad = false;
        let mut alt = false;
        // Mapping-key form: CPython's grammar puts `%(name)` FIRST, before
        // flags/width/precision (`%(k)-8.2f` is legal). Parse the key here,
        // then fall through to the shared spec machinery; the keyed value
        // becomes the conversion argument.
        let mut mapping_key: Option<String> = None;
        if chars.peek() == Some(&'(') {
            chars.next();
            let mut key = String::new();
            for ch in chars.by_ref() {
                if ch == ')' {
                    break;
                }
                key.push(ch);
            }
            if mapping.is_none() {
                return Err(PyException::new("TypeError", "format requires a mapping"));
            }
            mapping_key = Some(key);
        }
        loop {
            match chars.peek() {
                Some('-') => {
                    left_align = true;
                    chars.next();
                }
                Some('+') => {
                    plus = true;
                    chars.next();
                }
                Some(' ') => {
                    space = true;
                    chars.next();
                }
                Some('0') => {
                    zero_pad = true;
                    chars.next();
                }
                Some('#') => {
                    alt = true;
                    chars.next();
                }
                _ => break,
            }
        }
        // width — literal digits or `*` (next arg / keyed value; negative
        // flips to left-align per CPython).
        let mut width: Option<usize> = None;
        if chars.peek() == Some(&'*') {
            chars.next();
            let wv = if let Some(key) = &mapping_key {
                let d = mapping.as_ref().expect("mapping checked at key parse");
                match d.get(&Value::Str(crate::PyString::new(key.clone()))) {
                    Some(v) => v,
                    None => return Err(PyException::new("KeyError", key.clone())),
                }
            } else {
                let a = args.get(arg_i).cloned().unwrap_or(Value::None);
                arg_i += 1;
                a
            };
            // CPython checks the * argument's type before the conversion.
            let w = match &wv {
                Value::Bool(b) => i64::from(*b),
                Value::Int(n) => match n.to_i64() {
                    Some(x) => x,
                    None => {
                        return Err(PyException::new("TypeError", "* wants int"));
                    }
                },
                _ => return Err(PyException::new("TypeError", "* wants int")),
            };
            if w < 0 {
                left_align = true;
                width = Some((-w) as usize);
            } else {
                width = Some(w as usize);
            }
        } else {
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                width = Some(width.unwrap_or(0) * 10 + d as usize);
                chars.next();
            }
        }
        // precision
        let mut precision: Option<usize> = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut p = 0usize;
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                p = p * 10 + d as usize;
                chars.next();
            }
            precision = Some(p);
        }
        // length modifiers (h/l/L) — accepted and ignored
        while matches!(chars.peek(), Some('h') | Some('l') | Some('L')) {
            chars.next();
        }
        let conv = chars.next().unwrap_or('%');
        let arg = if let Some(key) = &mapping_key {
            let d = mapping.as_ref().expect("mapping checked at key parse");
            match d.get(&Value::Str(crate::PyString::new(key.clone()))) {
                Some(v) => v,
                None => return Err(PyException::new("KeyError", key.clone())),
            }
        } else {
            let a = args.get(arg_i).cloned().unwrap_or(Value::None);
            arg_i += 1;
            a
        };
        let rendered = match conv {
            's' => {
                let s = match &arg {
                    Value::Str(x) => x.as_str().to_string(),
                    other => crate::ops::to_str(other)?,
                };
                match precision {
                    Some(p) => s.chars().take(p).collect::<String>(),
                    None => s,
                }
            }
            'r' => {
                let s = crate::ops::repr(&arg)?;
                match precision {
                    Some(p) => s.chars().take(p).collect::<String>(),
                    None => s,
                }
            }
            'd' | 'i' | 'u' => {
                let n = int_of(&arg)?;
                let neg = n < 0;
                let digits = n.unsigned_abs().to_string();
                let digits = match precision {
                    Some(p) if digits.len() < p => {
                        format!("{}{}", "0".repeat(p - digits.len()), digits)
                    }
                    _ => digits,
                };
                let sign = if neg {
                    "-"
                } else if plus {
                    "+"
                } else if space {
                    " "
                } else {
                    ""
                };
                format!("{}{}", sign, digits)
            }
            'f' | 'F' => {
                let f = float_of(&arg)?;
                let p = precision.unwrap_or(6);
                let s = format!("{:.*}", p, f);
                let sign = if f < 0.0 || (f == 0.0 && f.is_sign_negative()) {
                    "-"
                } else if plus {
                    "+"
                } else if space {
                    " "
                } else {
                    ""
                };
                let body = s.trim_start_matches('-').to_string();
                let conv_ch = if conv == 'F' { 'F' } else { 'f' };
                format!("{}{}{}", sign, body, /*placeholder*/ "").replace('e', &conv_ch.to_string())
            }
            'e' | 'E' => {
                let f = float_of(&arg)?;
                let p = precision.unwrap_or(6);
                let s = format!("{:.*e}", p, f);
                let s = fix_exp_notation(&s);
                let s = if conv == 'E' { s.to_uppercase() } else { s };
                if f < 0.0 || (f == 0.0 && f.is_sign_negative()) {
                    format!("-{}", s.trim_start_matches('-'))
                } else if plus {
                    format!("+{}", s)
                } else {
                    s
                }
            }
            'g' | 'G' => {
                let f = float_of(&arg)?;
                let p = precision.unwrap_or(6).max(1);
                let s = format!("{:.*e}", p - 1, f);
                let s = fix_exp_notation(&s);
                let s = trim_g_trailing(f, &s, p);
                let s = if conv == 'G' { s.to_uppercase() } else { s };
                if f < 0.0 || (f == 0.0 && f.is_sign_negative()) {
                    format!("-{}", s.trim_start_matches('-'))
                } else if plus {
                    format!("+{}", s)
                } else {
                    s
                }
            }
            'x' | 'X' => {
                let n = int_of(&arg)?;
                let neg = n < 0;
                let u = if neg {
                    (-(n + 1)) as u128 + 1
                } else {
                    n as u128
                };
                let mut s = if conv == 'x' {
                    format!("{:x}", u)
                } else {
                    format!("{:X}", u)
                };
                if let Some(p) = precision {
                    if s.len() < p {
                        s = format!("{}{}", "0".repeat(p - s.len()), s);
                    }
                }
                let sign = if neg {
                    "-"
                } else if plus {
                    "+"
                } else {
                    ""
                };
                let prefix = if alt {
                    if conv == 'x' {
                        "0x"
                    } else {
                        "0X"
                    }
                } else {
                    ""
                };
                format!("{}{}{}", sign, prefix, s)
            }
            'o' => {
                let n = int_of(&arg)?;
                let neg = n < 0;
                let u = if neg {
                    (-(n + 1)) as u128 + 1
                } else {
                    n as u128
                };
                let mut s = format!("{:o}", u);
                if let Some(p) = precision {
                    if s.len() < p {
                        s = format!("{}{}", "0".repeat(p - s.len()), s);
                    }
                }
                let sign = if neg {
                    "-"
                } else if plus {
                    "+"
                } else {
                    ""
                };
                let prefix = if alt { "0o" } else { "" };
                format!("{}{}{}", sign, prefix, s)
            }
            'c' => match &arg {
                Value::Str(x) => {
                    let mut it = x.as_str().chars();
                    match (it.next(), it.next()) {
                        (Some(ch), None) => ch.to_string(),
                        _ => return Err(PyException::new("TypeError", "%c requires int or char")),
                    }
                }
                other => {
                    let n = int_of(other)?;
                    match char::from_u32(clamp_i64_to_u32(n)) {
                        Some(ch) => ch.to_string(),
                        None => {
                            return Err(PyException::new(
                                "OverflowError",
                                "%c arg not in range(0x110000)",
                            ))
                        }
                    }
                }
            },
            '%' => "%".to_string(),
            other => {
                let idx = fmt.chars().count() - 1 - chars.clone().count();
                return Err(PyException::new(
                    "ValueError",
                    format!(
                        "unsupported format character '{}' (0x{:x}) at index {}",
                        other, other as u32, idx
                    ),
                ));
            }
        };
        // width padding
        let padded = match width {
            Some(w) if rendered.chars().count() < w => {
                let pad_n = w - rendered.chars().count();
                if left_align {
                    format!("{}{}", rendered, " ".repeat(pad_n))
                } else if zero_pad
                    && matches!(
                        conv,
                        'd' | 'i' | 'u' | 'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'x' | 'X' | 'o'
                    )
                {
                    // zero-padding goes after the sign/prefix
                    if let Some(rest) = rendered.strip_prefix('-') {
                        format!("-{}{}", "0".repeat(pad_n), rest)
                    } else if let Some(rest) = rendered.strip_prefix('+') {
                        format!("+{}{}", "0".repeat(pad_n), rest)
                    } else {
                        format!("{}{}", "0".repeat(pad_n), rendered)
                    }
                } else {
                    format!("{}{}", " ".repeat(pad_n), rendered)
                }
            }
            _ => rendered,
        };
        out.push_str(&padded);
    }
    // Mapping form consumes the dict as a whole (arg_i stays 0).
    if !mapping.is_some() && arg_i < args.len() {
        return Err(PyException::new(
            "TypeError",
            "not all arguments converted during string formatting",
        ));
    }
    Ok(Value::Str(crate::PyString::new(out)))
}

fn fix_exp_notation(s: &str) -> String {
    // Rust: 1e3 → CPython: 1e+03
    if let Some(pos) = s.find(['e', 'E']) {
        let (mantissa, exp) = s.split_at(pos);
        let exp = &exp[1..];
        let (sign, digits) = match exp.strip_prefix('-') {
            Some(d) => ("-", d),
            None => ("+", exp.strip_prefix('+').unwrap_or(exp)),
        };
        let digits = if digits.len() < 2 {
            format!("0{}", digits)
        } else {
            digits.to_string()
        };
        format!("{}e{}{}", mantissa, sign, digits)
    } else {
        s.to_string()
    }
}

fn trim_g_trailing(f: f64, s: &str, precision: usize) -> String {
    // %g drops trailing zeros and switches to exponent when exp < -4 or >= p
    let keep_exp = {
        let exp = if f == 0.0 {
            0
        } else {
            f.abs().log10().floor() as i32
        };
        exp < -4 || exp >= precision as i32
    };
    if keep_exp {
        // CPython %g in exponent form: strip trailing zeros from the
        // mantissa (keep at least one digit), keep the exponent as-is.
        if let Some((mant, exp)) = s.split_once(['e', 'E']) {
            let t = if mant.contains('.') {
                mant.trim_end_matches('0').trim_end_matches('.')
            } else {
                mant
            };
            let t = if t.is_empty() { "0" } else { t };
            return format!("{}e{}", t, exp);
        }
        return s.to_string();
    }
    if s.contains(['e', 'E']) {
        // convert exponent form back to decimal for small ranges
        let f2: f64 = s.parse().unwrap_or(f);
        return format!("{}", f2);
    }
    if s.contains('.') {
        let t = s.trim_end_matches('0').trim_end_matches('.');
        t.to_string()
    } else {
        s.to_string()
    }
}

fn int_of(v: &Value) -> Result<i64, PyException> {
    match v {
        Value::Int(i) => i
            .to_i64()
            .ok_or_else(|| PyException::new("OverflowError", "int too large to convert to C long")),
        Value::Bool(b) => Ok(if *b { 1 } else { 0 }),
        Value::Float(f) => Ok(*f as i64),
        other => Err(PyException::new(
            "TypeError",
            format!(
                "%d format: a real number is required, not {}",
                other.type_name()
            ),
        )),
    }
}

fn clamp_i64_to_u32(n: i64) -> u32 {
    if n < 0 {
        0
    } else {
        (n as u64).min(0x10FFFF) as u32
    }
}

fn float_of(v: &Value) -> Result<f64, PyException> {
    match v {
        Value::Float(f) => Ok(*f),
        Value::Int(i) => i
            .to_i64()
            .map(|n| n as f64)
            .ok_or_else(|| PyException::new("OverflowError", "int too large to convert to float")),
        Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
        other => Err(PyException::new(
            "TypeError",
            format!("must be real number, not {}", other.type_name()),
        )),
    }
}

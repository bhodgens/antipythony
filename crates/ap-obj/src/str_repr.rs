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
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
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

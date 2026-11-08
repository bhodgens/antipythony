use crate::builtins_modules::raise;
/// UTF-7 (RFC 2152): the pure-Python wrapper path. Direct chars pass through;
/// '+' encodes as '+-'; other non-direct chars encode as a modified-base64
/// UTF-16BE run terminated by '-'. Direct set: alphanumerics plus
/// '(),./:?'. Optional direct (default NOT direct): space, \\t, \\r, \\n
/// (space is emitted literally, matching CPython's default set here).
pub fn utf_7_encode(text: &str, _errors: &str) -> Result<Vec<u8>, crate::Flow> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        let c = ch as u32;
        if c == 0x2B {
            // '+' literal -> '+-'
            out.extend_from_slice(b"+-");
        } else if c < 0x80
            && (ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '(' | ')' | ',' | '.' | '/' | ':' | '?' | ' ' | '\t' | '\r' | '\n' | '\''
                ))
        {
            out.push(c as u8);
        } else {
            // modified-base64 run: accumulate consecutive non-direct chars
            let mut units: Vec<u16> = vec![u16::try_from(c)
                .map_err(|_| raise("UnicodeEncodeError", "surrogates not allowed in utf-7"))?];
            while let Some(&nc) = chars.peek() {
                let nc_u = nc as u32;
                if nc_u == 0x2B
                    || (nc_u < 0x80
                        && (nc.is_ascii_alphanumeric()
                            || matches!(
                                nc,
                                '(' | ')'
                                    | ','
                                    | '.'
                                    | '/'
                                    | ':'
                                    | '?'
                                    | ' '
                                    | '\t'
                                    | '\r'
                                    | '\n'
                                    | '\''
                            )))
                {
                    break;
                }
                chars.next();
                if let Ok(u) = u16::try_from(nc_u) {
                    units.push(u);
                }
            }
            out.push(b'+');
            // UTF-16BE bytes -> base64 WITHOUT padding, '+' instead of '/'.
            // Issue #13 fix: accumulate LOW (acc = acc<<16 | unit), emit from
            // the TOP — the old `bits |= u << (16-nbits)` put the first unit
            // at bit 16+ and then shifted relative to nbits, emitting zeros
            // ('+AAA-' instead of '+Ti0-' for U+4E2D).
            let mut acc: u32 = 0;
            let mut nbits = 0u32;
            const ALPHA: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            for u in &units {
                acc = (acc << 16) | (*u as u32);
                nbits += 16;
                while nbits >= 6 {
                    nbits -= 6;
                    let sextet = (acc >> nbits) & 0x3F;
                    out.push(ALPHA[sextet as usize]);
                }
                acc &= (1 << nbits) - 1;
            }
            if nbits > 0 {
                let sextet = (acc << (6 - nbits)) & 0x3F;
                out.push(ALPHA[sextet as usize]);
            }
            // a '-' terminator is needed when the next char would otherwise
            // continue the base64 run (alphanumeric)
            if let Some(&nc) = chars.peek() {
                if nc.is_ascii_alphanumeric() || nc == '+' || nc == '-' {
                    out.push(b'-');
                }
            }
        }
    }
    Ok(out)
}

/// UTF-7 decode: '+' introduces a modified-base64 run terminated by '-' (or
/// end / non-base64 char); everything else passes through as ASCII.
pub fn utf_7_decode(b: &[u8], _errors: &str) -> Result<String, crate::Flow> {
    let mut out = String::new();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'+' {
            // direct byte (high bytes are an error in strict utf-7, but the
            // vendored codecs pass them through — documented v0 relaxation)
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        if i < b.len() && b[i] == b'-' {
            out.push('+');
            i += 1;
            continue;
        }
        // collect base64 sextets until '-'/non-base64/end
        let mut acc: u32 = 0;
        let mut nbits = 0u32;
        let mut units: Vec<u16> = Vec::new();
        while i < b.len() && b[i] != b'-' {
            let c = b[i] as char;
            let v = match c {
                'A'..='Z' => c as u32 - 'A' as u32,
                'a'..='z' => c as u32 - 'a' as u32 + 26,
                '0'..='9' => c as u32 - '0' as u32 + 52,
                '+' => 62,
                '/' => 63,
                _ => break,
            };
            acc = (acc << 6) | v;
            nbits += 6;
            if nbits >= 16 {
                units.push(((acc >> (nbits - 16)) & 0xFFFF) as u16);
                nbits -= 16;
                acc &= (1 << nbits) - 1;
            }
            i += 1;
        }
        if i < b.len() && b[i] == b'-' {
            i += 1;
        }
        let units16: Vec<u16> = units;
        // UTF-16BE units -> chars (surrogate pairs included)
        let mut k = 0usize;
        let mut buf: Vec<u16> = Vec::new();
        while k < units16.len() {
            let u = units16[k];
            if (0xD800..0xDC00).contains(&u) && k + 1 < units16.len() {
                // high surrogate
                let lo = units16[k + 1];
                if (0xDC00..0xE000).contains(&lo) {
                    let cp = 0x10000 + (((u as u32) - 0xD800) << 10) + (lo as u32 - 0xDC00);
                    buf.clear();
                    if let Some(ch) = char::from_u32(cp) {
                        out.push(ch);
                    }
                    k += 2;
                    continue;
                }
            }
            if let Some(ch) = char::from_u32(u as u32) {
                out.push(ch);
            }
            k += 1;
        }
    }
    Ok(out)
}

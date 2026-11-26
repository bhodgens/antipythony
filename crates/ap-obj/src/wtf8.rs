//! V016: WTF-8 code-point helpers for PyString.
//!
//! A lone surrogate (U+D800..U+DFFF) cannot be a Rust `char`. PyString
//! stores it as WTF-8: the 3-byte UTF-8-style sequence ED A0 80 ..
//! ED BF BF inside the existing Unicode `Box<str>`. Valid UTF-8 never
//! contains those bytes, so every surrogate-free string is untouched
//! and every existing as_str() consumer keeps its behavior.
//!
//! This module is the ONE place that knows the encoding. Consumers that
//! must be code-point-correct (len, ord, chr, repr, encode/decode,
//! ordering) route through here; byte-level ops (find/replace/concat
//! with ASCII patterns) stay on as_str() — WTF-8 is self-synchronizing.
//!
//! The same helpers live in ap-parse::surrogates for the parser shim;
//! ap-obj cannot depend on ap-parse (crate-layering rule: the object
//! model sits below the frontend), so the small codec is duplicated
//! here with a cross-reference. Keep the two in sync.

/// True if `s` contains a WTF-8 lone-surrogate sequence (ED A0..BF ..).
pub fn has_surrogates(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0xED && (0xA0..=0xBF).contains(&b[i + 1]) {
            return true;
        }
        i += 1;
    }
    false
}

/// Encode one code point (possibly a lone surrogate) as WTF-8 bytes.
pub fn push_cp(out: &mut String, cp: u32) {
    let mut buf = [0u8; 4];
    let n = if cp < 0x80 {
        buf[0] = cp as u8;
        1
    } else if cp < 0x800 {
        buf[0] = 0xC0 | (cp >> 6) as u8;
        buf[1] = 0x80 | (cp & 0x3F) as u8;
        2
    } else if cp < 0x10000 {
        buf[0] = 0xE0 | (cp >> 12) as u8;
        buf[1] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        buf[2] = 0x80 | (cp & 0x3F) as u8;
        3
    } else {
        buf[0] = 0xF0 | (cp >> 18) as u8;
        buf[1] = 0x80 | ((cp >> 12) & 0x3F) as u8;
        buf[2] = 0x80 | ((cp >> 6) & 0x3F) as u8;
        buf[3] = 0x80 | (cp & 0x3F) as u8;
        4
    };
    // SAFETY: buf[..n] is byte-well-formed UTF-8 by construction (the
    // WTF-8 surrogate form included); str's invariant is byte-well-
    // formedness, and has_surrogates gates every chars()-based consumer.
    out.push_str(unsafe { std::str::from_utf8_unchecked(&buf[..n]) });
}

/// Build a String from code points (WTF-8; surrogates allowed).
pub fn from_code_points(cps: impl IntoIterator<Item = u32>) -> String {
    let mut out = String::new();
    for cp in cps {
        push_cp(&mut out, cp);
    }
    out
}

/// Iterate the code points of a WTF-8 string. Fast path: valid UTF-8.
pub fn code_points(s: &str) -> Vec<u32> {
    if !has_surrogates(s) {
        return s.chars().map(|c| c as u32).collect();
    }
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let x = b[i];
        if x < 0x80 {
            out.push(x as u32);
            i += 1;
        } else if x < 0xE0 {
            let cp = ((x as u32 & 0x1F) << 6) | (b[i + 1] as u32 & 0x3F);
            out.push(cp);
            i += 2;
        } else if x < 0xF0 {
            let cp = ((x as u32 & 0x0F) << 12)
                | ((b[i + 1] as u32 & 0x3F) << 6)
                | (b[i + 2] as u32 & 0x3F);
            out.push(cp);
            i += 3;
        } else {
            let cp = ((x as u32 & 0x07) << 18)
                | ((b[i + 1] as u32 & 0x3F) << 12)
                | ((b[i + 2] as u32 & 0x3F) << 6)
                | (b[i + 3] as u32 & 0x3F);
            out.push(cp);
            i += 4;
        }
    }
    out
}

/// Code-point count of a WTF-8 string.
pub fn cp_len(s: &str) -> usize {
    if !has_surrogates(s) {
        return s.chars().count();
    }
    code_points(s).len()
}

/// True if cp is a lone surrogate.
pub fn is_surrogate(cp: u32) -> bool {
    (0xD800..=0xDFFF).contains(&cp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let cps = vec![0x41, 0xD800, 0xDC00, 0x1F600, 0xE9];
        let s = from_code_points(cps.iter().copied());
        assert!(has_surrogates(&s));
        assert_eq!(code_points(&s), cps);
        assert_eq!(cp_len(&s), cps.len());
    }

    #[test]
    fn valid_utf8_untouched() {
        let s = "café \u{1F600}";
        assert!(!has_surrogates(s));
        assert_eq!(cp_len(s), s.chars().count());
    }
}

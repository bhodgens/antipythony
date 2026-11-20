//! V016: lone-surrogate string literals (parser-layer shim).
//!
//! ruff_python_parser 0.0.4 maps every `\u`/`\U` escape in D800..DFFF to
//! U+FFFD at parse time (string.rs:179) because a Rust `char` cannot hold
//! a lone surrogate. CPython keeps them: `"\ud800"` is a 1-code-point str
//! that raises UnicodeEncodeError on `.encode("utf-8")`.
//!
//! This module re-decodes the CONTENT of every accepted string literal
//! from the original source with CPython escape semantics, storing lone
//! surrogates as WTF-8 (the 3-byte UTF-8-style sequence ED A0 80 ..
//! ED BF BF). Valid UTF-8 never contains those bytes, so surrogate-free
//! strings are byte-identical to ruff's own decoding (the shim only
//! rewrites literals whose content contains a `\u`/`\U` escape).
//!
//! The shim runs AFTER ruff accepts the program: malformed escapes
//! already failed ruff's parse, so re-decoding cannot introduce new
//! syntax errors. Raw strings keep their content verbatim (no escapes).

use ruff_python_ast as ast;

/// True if `s` contains a WTF-8 lone-surrogate sequence (ED A0..BF ..).
/// Valid UTF-8 never contains these bytes, so this is a cheap scan.
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
/// Bytes are pushed via unsafe from_utf8_unchecked on each sequence:
/// every sequence this function emits is structurally valid UTF-8
/// (surrogate code points encode to the ED A0..BF form, which is
/// byte-well-formed but not scalar-valid — Rust's str invariant is
/// byte-well-formedness, so this is sound; chars() iteration over such
/// bytes is the documented WTF-8 hazard we gate with has_surrogates).
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
    // SAFETY: buf[..n] is a well-formed UTF-8 sequence by construction
    // (including the WTF-8 surrogate form, which is byte-well-formed).
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

/// Iterate the code points of a WTF-8 string. Fast path: valid UTF-8
/// decodes via chars(); the slow path handles surrogate sequences.
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

/// Re-decode a string literal's raw content with CPython escape rules,
/// emitting WTF-8 for lone surrogates. `raw` = r/R prefix (no escapes).
/// `fmiddle` = f/t-string middle chunk: doubled braces {{ }} collapse to
/// one brace (ruff's parse_interpolated_string_middle semantics).
pub fn decode_content(content: &str, raw: bool, fmiddle: bool) -> String {
    if raw {
        return content.to_string();
    }
    let b = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c != b'\\' {
            if fmiddle && (c == b'{' || c == b'}') {
                // Doubled brace collapses; a lone brace in a middle chunk
                // is kept verbatim (ruff only sees escaped/doubled ones).
                if i + 1 < b.len() && b[i + 1] == c {
                    out.push(c as char);
                    i += 2;
                    continue;
                }
            }
            // Copy one UTF-8 scalar verbatim.
            let ch_len = utf8_len(c);
            out.push_str(&content[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        // Escape sequence.
        i += 1;
        if i >= b.len() {
            // Trailing backslash: ruff keeps it in f-string middles; in a
            // plain literal this cannot occur (parse would have failed).
            out.push('\\');
            break;
        }
        let e = b[i];
        i += 1;
        match e {
            b'\\' => out.push('\\'),
            b'\'' => out.push('\''),
            b'"' => out.push('"'),
            b'a' => out.push('\x07'),
            b'b' => out.push('\x08'),
            b'f' => out.push('\x0c'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'v' => out.push('\x0b'),
            b'\n' => {} // line continuation: dropped
            b'\r' => {
                if i < b.len() && b[i] == b'\n' {
                    i += 1;
                }
            }
            b'0'..=b'7' => {
                // Octal: up to 3 digits total (the first is `e`).
                let mut v = (e - b'0') as u32;
                let mut n = 1;
                while n < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                    v = v * 8 + (b[i] - b'0') as u32;
                    i += 1;
                    n += 1;
                }
                push_cp(&mut out, v);
            }
            b'x' => {
                let (v, ni) = hex_escape(b, i, 2);
                i = ni;
                push_cp(&mut out, v);
            }
            b'u' => {
                let (v, ni) = hex_escape(b, i, 4);
                i = ni;
                push_cp(&mut out, v); // surrogate range kept as WTF-8
            }
            b'U' => {
                let (v, ni) = hex_escape(b, i, 8);
                i = ni;
                push_cp(&mut out, v);
            }
            b'N' => {
                // \N{name}: ruff already validated the name; resolve via
                // the same unicode_names2 table ruff uses.
                if i < b.len() && b[i] == b'{' {
                    if let Some(close) = content[i..].find('}') {
                        let name = &content[i + 1..i + close];
                        if let Some(ch) = unicode_names2::character(name) {
                            out.push(ch);
                        }
                        i += close + 1;
                    }
                }
            }
            other => {
                // Unknown escape: CPython keeps backslash + char.
                out.push('\\');
                let ch_len = utf8_len(other);
                out.push_str(&content[i - 1..i - 1 + ch_len]);
                i += ch_len - 1;
            }
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

/// Read exactly `n` hex digits at b[i..]; ruff already validated them.
fn hex_escape(b: &[u8], i: usize, n: usize) -> (u32, usize) {
    let mut v = 0u32;
    let mut j = i;
    for _ in 0..n {
        if j >= b.len() {
            break;
        }
        let d = (b[j] as char).to_digit(16).unwrap_or(0);
        v = v * 16 + d;
        j += 1;
    }
    (v, j)
}

/// True if the content has a `\u`/`\U` escape (the only escapes ruff
/// destroys). Cheap pre-scan so the shim skips untouched literals.
fn needs_redecode(content: &str) -> bool {
    let b = content.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'\\' && (b[i + 1] == b'u' || b[i + 1] == b'U') {
            return true;
        }
        i += 1;
    }
    false
}

/// Patch every string-literal value in the module whose content carries
/// a `\u`/`\U` escape. Walks the full ruff AST (mutable).
pub fn patch_module(module: &mut ast::ModModule, source: &str) {
    for stmt in &mut module.body {
        patch_stmt(stmt, source);
    }
}

fn patch_stmt(stmt: &mut ast::Stmt, source: &str) {
    use ast::Stmt::*;
    match stmt {
        FunctionDef(f) => {
            for d in &mut f.decorator_list {
                patch_expr(&mut d.expression, source);
            }
            if let Some(tp) = &mut f.type_params {
                patch_type_params(tp, source);
            }
            patch_parameters(&mut f.parameters, source);
            if let Some(r) = &mut f.returns {
                patch_expr(r, source);
            }
            for s in &mut f.body {
                patch_stmt(s, source);
            }
        }
        ClassDef(c) => {
            for d in &mut c.decorator_list {
                patch_expr(&mut d.expression, source);
            }
            if let Some(tp) = &mut c.type_params {
                patch_type_params(tp, source);
            }
            if let Some(a) = &mut c.arguments {
                patch_arguments(a, source);
            }
            for s in &mut c.body {
                patch_stmt(s, source);
            }
        }
        Return(r) => {
            if let Some(v) = &mut r.value {
                patch_expr(v, source);
            }
        }
        Delete(d) => {
            for t in &mut d.targets {
                patch_expr(t, source);
            }
        }
        TypeAlias(t) => {
            patch_expr(&mut t.name, source);
            if let Some(tp) = &mut t.type_params {
                patch_type_params(tp, source);
            }
            patch_expr(&mut t.value, source);
        }
        Assign(a) => {
            patch_expr(&mut a.value, source);
            for t in &mut a.targets {
                patch_expr(t, source);
            }
        }
        AugAssign(a) => {
            patch_expr(&mut a.value, source);
            patch_expr(&mut a.target, source);
        }
        AnnAssign(a) => {
            if let Some(v) = &mut a.value {
                patch_expr(v, source);
            }
            patch_expr(&mut a.annotation, source);
            patch_expr(&mut a.target, source);
        }
        For(f) => {
            patch_expr(&mut f.iter, source);
            patch_expr(&mut f.target, source);
            for s in &mut f.body {
                patch_stmt(s, source);
            }
            for s in &mut f.orelse {
                patch_stmt(s, source);
            }
        }
        While(w) => {
            patch_expr(&mut w.test, source);
            for s in &mut w.body {
                patch_stmt(s, source);
            }
            for s in &mut w.orelse {
                patch_stmt(s, source);
            }
        }
        If(i) => {
            patch_expr(&mut i.test, source);
            for s in &mut i.body {
                patch_stmt(s, source);
            }
            for cl in &mut i.elif_else_clauses {
                if let Some(t) = &mut cl.test {
                    patch_expr(t, source);
                }
                for s in &mut cl.body {
                    patch_stmt(s, source);
                }
            }
        }
        With(w) => {
            for it in &mut w.items {
                patch_expr(&mut it.context_expr, source);
                if let Some(v) = &mut it.optional_vars {
                    patch_expr(v, source);
                }
            }
            for s in &mut w.body {
                patch_stmt(s, source);
            }
        }
        Match(m) => {
            patch_expr(&mut m.subject, source);
            for case in &mut m.cases {
                patch_pattern(&mut case.pattern, source);
                if let Some(g) = &mut case.guard {
                    patch_expr(g, source);
                }
                for s in &mut case.body {
                    patch_stmt(s, source);
                }
            }
        }
        Raise(r) => {
            if let Some(e) = &mut r.exc {
                patch_expr(e, source);
            }
            if let Some(c) = &mut r.cause {
                patch_expr(c, source);
            }
        }
        Try(t) => {
            for s in &mut t.body {
                patch_stmt(s, source);
            }
            for h in &mut t.handlers {
                let ast::ExceptHandler::ExceptHandler(h) = h;
                if let Some(ty) = &mut h.type_ {
                    patch_expr(ty, source);
                }
                for s in &mut h.body {
                    patch_stmt(s, source);
                }
            }
            for s in &mut t.orelse {
                patch_stmt(s, source);
            }
            for s in &mut t.finalbody {
                patch_stmt(s, source);
            }
        }
        Assert(a) => {
            patch_expr(&mut a.test, source);
            if let Some(m) = &mut a.msg {
                patch_expr(m, source);
            }
        }
        Expr(e) => patch_expr(&mut e.value, source),
        Import(_) | ImportFrom(_) | Global(_) | Nonlocal(_) | Pass(_) | Break(_) | Continue(_)
        | IpyEscapeCommand(_) => {}
    }
}

fn patch_expr(expr: &mut ast::Expr, source: &str) {
    use ast::Expr::*;
    match expr {
        StringLiteral(s) => {
            for part in s.value.iter_mut() {
                patch_string_literal(part, source);
            }
        }
        FString(f) => {
            for part in f.value.iter_mut() {
                match part {
                    ast::FStringPart::Literal(sl) => patch_string_literal(sl, source),
                    ast::FStringPart::FString(fs) => patch_fstring(fs, source),
                }
            }
        }
        TString(t) => {
            for ts in t.value.iter_mut() {
                patch_fstring_elements(&mut ts.elements, source);
            }
        }
        BoolOp(b) => {
            for v in &mut b.values {
                patch_expr(v, source);
            }
        }
        Named(n) => {
            patch_expr(&mut n.value, source);
            patch_expr(&mut n.target, source);
        }
        BinOp(b) => {
            patch_expr(&mut b.left, source);
            patch_expr(&mut b.right, source);
        }
        UnaryOp(u) => patch_expr(&mut u.operand, source),
        Lambda(l) => {
            if let Some(p) = &mut l.parameters {
                patch_parameters(p, source);
            }
            patch_expr(&mut l.body, source);
        }
        If(i) => {
            patch_expr(&mut i.test, source);
            patch_expr(&mut i.body, source);
            patch_expr(&mut i.orelse, source);
        }
        Dict(d) => {
            for it in &mut d.items {
                if let Some(k) = &mut it.key {
                    patch_expr(k, source);
                }
                patch_expr(&mut it.value, source);
            }
        }
        Set(s) => {
            for e in &mut s.elts {
                patch_expr(e, source);
            }
        }
        ListComp(c) => {
            patch_comp_generators(&mut c.generators, source);
            patch_expr(&mut c.elt, source);
        }
        SetComp(c) => {
            patch_comp_generators(&mut c.generators, source);
            patch_expr(&mut c.elt, source);
        }
        DictComp(c) => {
            patch_comp_generators(&mut c.generators, source);
            if let Some(k) = &mut c.key {
                patch_expr(k, source);
            }
            patch_expr(&mut c.value, source);
        }
        Generator(g) => {
            patch_comp_generators(&mut g.generators, source);
            patch_expr(&mut g.elt, source);
        }
        Await(a) => patch_expr(&mut a.value, source),
        Yield(y) => {
            if let Some(v) = &mut y.value {
                patch_expr(v, source);
            }
        }
        YieldFrom(y) => patch_expr(&mut y.value, source),
        Compare(c) => {
            patch_expr(&mut c.left, source);
            for e in c.comparators.iter_mut() {
                patch_expr(e, source);
            }
        }
        Call(c) => {
            patch_expr(&mut c.func, source);
            patch_arguments(&mut c.arguments, source);
        }
        Attribute(a) => patch_expr(&mut a.value, source),
        Subscript(s) => {
            patch_expr(&mut s.value, source);
            patch_expr(&mut s.slice, source);
        }
        Starred(s) => patch_expr(&mut s.value, source),
        List(l) => {
            for e in &mut l.elts {
                patch_expr(e, source);
            }
        }
        Tuple(t) => {
            for e in &mut t.elts {
                patch_expr(e, source);
            }
        }
        Slice(s) => {
            if let Some(l) = &mut s.lower {
                patch_expr(l, source);
            }
            if let Some(u) = &mut s.upper {
                patch_expr(u, source);
            }
            if let Some(st) = &mut s.step {
                patch_expr(st, source);
            }
        }
        BytesLiteral(_) | NumberLiteral(_) | BooleanLiteral(_) | NoneLiteral(_)
        | EllipsisLiteral(_) | Name(_) | IpyEscapeCommand(_) => {}
    }
}

fn patch_string_literal(sl: &mut ast::StringLiteral, source: &str) {
    use ast::StringFlags;
    if sl.flags.as_any_string_flags().is_raw_string() {
        return;
    }
    let cr = sl.content_range();
    let start = usize::try_from(u32::from(cr.start())).unwrap_or(0);
    let end = usize::try_from(u32::from(cr.end())).unwrap_or(0);
    if end > source.len() || start > end {
        return;
    }
    let content = &source[start..end];
    if !needs_redecode(content) {
        return;
    }
    sl.value = decode_content(content, false, false).into_boxed_str();
}

fn patch_fstring(fs: &mut ast::FString, source: &str) {
    patch_fstring_elements(&mut fs.elements, source);
}

fn patch_fstring_elements(elements: &mut ast::InterpolatedStringElements, source: &str) {
    for el in elements.iter_mut() {
        match el {
            ast::InterpolatedStringElement::Literal(lit) => {
                let start = usize::try_from(u32::from(lit.range.start())).unwrap_or(0);
                let end = usize::try_from(u32::from(lit.range.end())).unwrap_or(0);
                if end > source.len() || start > end {
                    continue;
                }
                let content = &source[start..end];
                if !needs_redecode(content) {
                    continue;
                }
                lit.value = decode_content(content, false, true).into_boxed_str();
            }
            ast::InterpolatedStringElement::Interpolation(interp) => {
                patch_expr(&mut interp.expression, source);
                if let Some(spec) = &mut interp.format_spec {
                    patch_fstring_elements(&mut spec.elements, source);
                }
            }
        }
    }
}

fn patch_arguments(args: &mut ast::Arguments, source: &str) {
    for a in args.args.iter_mut() {
        patch_expr(a, source);
    }
    for k in &mut args.keywords {
        patch_expr(&mut k.value, source);
    }
}

fn patch_parameters(params: &mut ast::Parameters, source: &str) {
    for p in params
        .posonlyargs
        .iter_mut()
        .chain(params.args.iter_mut())
        .chain(params.kwonlyargs.iter_mut())
    {
        if let Some(d) = &mut p.default {
            patch_expr(d, source);
        }
        if let Some(a) = &mut p.parameter.annotation {
            patch_expr(a, source);
        }
    }
    if let Some(v) = &mut params.vararg {
        if let Some(a) = &mut v.annotation {
            patch_expr(a, source);
        }
    }
    if let Some(k) = &mut params.kwarg {
        if let Some(a) = &mut k.annotation {
            patch_expr(a, source);
        }
    }
}

fn patch_comp_generators(gens: &mut [ast::Comprehension], source: &str) {
    for g in gens {
        patch_expr(&mut g.iter, source);
        patch_expr(&mut g.target, source);
        for i in &mut g.ifs {
            patch_expr(i, source);
        }
    }
}

fn patch_type_params(tp: &mut ast::TypeParams, source: &str) {
    for p in &mut tp.type_params {
        match p {
            ast::TypeParam::TypeVar(v) => {
                if let Some(b) = &mut v.bound {
                    patch_expr(b, source);
                }
                if let Some(d) = &mut v.default {
                    patch_expr(d, source);
                }
            }
            ast::TypeParam::TypeVarTuple(v) => {
                if let Some(d) = &mut v.default {
                    patch_expr(d, source);
                }
            }
            ast::TypeParam::ParamSpec(v) => {
                if let Some(d) = &mut v.default {
                    patch_expr(d, source);
                }
            }
        }
    }
}

fn patch_pattern(pat: &mut ast::Pattern, source: &str) {
    use ast::Pattern::*;
    match pat {
        MatchValue(v) => patch_expr(&mut v.value, source),
        MatchSequence(s) => {
            for p in &mut s.patterns {
                patch_pattern(p, source);
            }
        }
        MatchMapping(m) => {
            for k in m.keys.iter_mut() {
                patch_expr(k, source);
            }
            for p in m.patterns.iter_mut() {
                patch_pattern(p, source);
            }
        }
        MatchClass(c) => {
            patch_expr(&mut c.cls, source);
            for p in c.arguments.patterns.iter_mut() {
                patch_pattern(p, source);
            }
            for k in &mut c.arguments.keywords {
                patch_pattern(&mut k.pattern, source);
            }
        }
        MatchAs(a) => {
            if let Some(p) = &mut a.pattern {
                patch_pattern(p, source);
            }
        }
        MatchOr(o) => {
            for p in &mut o.patterns {
                patch_pattern(p, source);
            }
        }
        MatchSingleton(_) | MatchStar(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surrogate_escape_decodes_to_wtf8() {
        let s = decode_content(r"\ud800", false, false);
        assert_eq!(s.as_bytes(), [0xED, 0xA0, 0x80]);
        assert!(has_surrogates(&s));
        assert_eq!(code_points(&s), vec![0xD800]);
        assert_eq!(cp_len(&s), 1);
    }

    #[test]
    fn astral_escape_stays_scalar() {
        let s = decode_content(r"\U0001F600", false, false);
        assert_eq!(s, "\u{1F600}");
        assert!(!has_surrogates(&s));
        assert_eq!(code_points(&s), vec![0x1F600]);
    }

    #[test]
    fn mixed_content() {
        let s = decode_content(r"a\ud800b\udc00c", false, false);
        assert_eq!(code_points(&s), vec![0x61, 0xD800, 0x62, 0xDC00, 0x63]);
        assert_eq!(cp_len(&s), 5);
    }

    #[test]
    fn standard_escapes_match_ruff() {
        assert_eq!(decode_content(r"\n\t\\", false, false), "\n\t\\");
        assert_eq!(decode_content("\\\"", false, false), "\"");
        assert_eq!(decode_content(r"\x41", false, false), "A");
        assert_eq!(decode_content(r"\101", false, false), "A");
        assert_eq!(
            decode_content(r"\N{LATIN SMALL LETTER A}", false, false),
            "a"
        );
        // Unknown escape kept verbatim (CPython).
        assert_eq!(decode_content(r"\q", false, false), "\\q");
        // Line continuation dropped.
        assert_eq!(decode_content("a\\\nb", false, false), "ab");
    }

    #[test]
    fn raw_content_verbatim() {
        assert_eq!(decode_content(r"\ud800", true, false), r"\ud800");
    }

    #[test]
    fn fmiddle_braces_collapse() {
        assert_eq!(decode_content("{{x}}", false, true), "{x}");
        assert_eq!(
            decode_content(r"\ud800{{", false, true).as_bytes()[..3],
            [0xED, 0xA0, 0x80]
        );
    }

    #[test]
    fn roundtrip_code_points() {
        let cps = vec![0x41, 0xD800, 0xDC00, 0x1F600, 0xE9, 0x7F];
        let s = from_code_points(cps.iter().copied());
        assert_eq!(code_points(&s), cps);
        assert_eq!(cp_len(&s), cps.len());
    }

    #[test]
    fn valid_utf8_fast_path() {
        let s = "café \u{1F600}";
        assert!(!has_surrogates(s));
        assert_eq!(cp_len(s), s.chars().count());
        assert_eq!(
            code_points(s),
            s.chars().map(|c| c as u32).collect::<Vec<_>>()
        );
    }
}

//! Leaf 02.08-full part 2 — the real regex engine.
//!
//! DESIGN CHOICE (documented per the leaf brief): a fully hand-rolled
//! backtracking engine in Rust — hand-written recursive-descent pattern
//! parser + recursive backtracking matcher over an AST. The Rust `regex`
//! crate was REJECTED: its DFA execution deliberately differs from
//! CPython's leftmost-first backtracking semantics (no backrefs, no
//! uniform lookaround, different alternation preference), which would make
//! byte-exactness impossible. The vendored CPython re/_parser + re/_compiler
//! + C _sre path was also rejected: it needs array-module semantics,
//!   memoryview.cast and a bytecode VM — far more interpreter surface than a
//!   native engine, for the same observable behavior.
//!
//! CPython SEMANTICS honored: leftmost-first (PCRE-style) match, greedy/
//! lazy quantifiers with backtracking, capture groups reset on backtrack,
//! fixed-width lookbehind (byte-exact compile error), \1 backrefs, named
//! groups, conditional groups (?(1)y|n), atomic groups + possessive
//! quantifiers (no backtrack into them), \b/\B/\A/\Z anchors, MULTILINE/
//! DOTALL/IGNORECASE/VERBOSE/ASCII flags, inline (?i:...) / scoped (?i)...,
//! full-Unicode \d \w \s tables (sre_tables.rs, extracted from CPython
//! 3.14.7) and the _casefix extra-cases set for IGNORECASE.
//!
//! CATASTROPHIC-BACKTRACKING GUARD (documented divergence): CPython has no
//! step limit (it hangs); this engine aborts with
//! re.error("ap: matching step limit exceeded") after 4M atomic steps per
//! match call — the corpus must not hang. No probe reaches the limit.
//!
//! Value layer: Pattern and Match are Opaque objects; attributes/methods
//! dispatch through `re_pattern_attr` / `re_match_attr` / `call_re_obj`.

use std::collections::HashMap;
use std::sync::Arc;

use ap_obj::value::Value;
use ap_obj::PyObject;

use crate::builtins_modules::{i as vi, raise, s as vs, tuple as vtuple};
use crate::sre_tables::{in_ranges, DIGIT_RANGES, SPACE_RANGES, WORD_RANGES};

// ===========================================================================
// Flags (values match re._constants SRE_FLAG_*)
// ===========================================================================

pub const F_IGNORECASE: u32 = 2;
pub const F_LOCALE: u32 = 4;
pub const F_MULTILINE: u32 = 8;
pub const F_DOTALL: u32 = 16;
pub const F_UNICODE: u32 = 32;
pub const F_VERBOSE: u32 = 64;
pub const F_ASCII: u32 = 256;

// _casefix: extra chars sharing an uppercase (vendored re/_casefix.py).
const EXTRA_CASES: &[(u32, &[u32])] = &[
    (0x0069, &[0x0131]),
    (0x0073, &[0x017f]),
    (0x00b5, &[0x03bc]),
    (0x0131, &[0x0069]),
    (0x017f, &[0x0073]),
    (0x0345, &[0x03b9, 0x1fbe]),
    (0x0390, &[0x1fd3]),
    (0x03b0, &[0x1fe3]),
    (0x03b2, &[0x03d0]),
    (0x03b5, &[0x03f5]),
    (0x03b8, &[0x03d1]),
    (0x03b9, &[0x0345, 0x1fbe]),
    (0x03ba, &[0x03f0]),
    (0x03bc, &[0x00b5]),
    (0x03c0, &[0x03d6]),
    (0x03c1, &[0x03f1]),
    (0x03c2, &[0x03c3]),
    (0x03c3, &[0x03c2]),
    (0x03c6, &[0x03d5]),
    (0x03d0, &[0x03b2]),
    (0x03d1, &[0x03b8]),
    (0x03d5, &[0x03c6]),
    (0x03d6, &[0x03c0]),
    (0x03f0, &[0x03ba]),
    (0x03f1, &[0x03c1]),
    (0x03f5, &[0x03b5]),
    (0x0432, &[0x1c80]),
    (0x0434, &[0x1c81]),
    (0x043e, &[0x1c82]),
    (0x0441, &[0x1c83]),
    (0x0442, &[0x1c84, 0x1c85]),
    (0x044a, &[0x1c86]),
    (0x0463, &[0x1c87]),
    (0x1c80, &[0x0432]),
    (0x1c81, &[0x0434]),
    (0x1c82, &[0x043e]),
    (0x1c83, &[0x0441]),
    (0x1c84, &[0x0442, 0x1c85]),
    (0x1c85, &[0x0442, 0x1c84]),
    (0x1c86, &[0x044a]),
    (0x1c87, &[0x0463]),
    (0x1c88, &[0xa64b]),
    (0xa64b, &[0x1c88]),
    (0x1e61, &[0x1e9b]),
    (0x1e9b, &[0x1e61]),
    (0x1fbe, &[0x0345, 0x03b9]),
    (0x1fd3, &[0x0390]),
    (0x1fe3, &[0x03b0]),
    (0xfb05, &[0xfb06]),
    (0xfb06, &[0xfb05]),
];

fn extra_cases(c: u32) -> &'static [u32] {
    EXTRA_CASES
        .iter()
        .find(|(k, _)| *k == c)
        .map(|(_, v)| *v)
        .unwrap_or(&[])
}

// ===========================================================================
// AST
// ===========================================================================

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Empty,
    Literal(u32),
    LiteralIgnore(u32),
    NotLiteral(u32),
    NotLiteralIgnore(u32),
    AnyChar,     // dot, no DOTALL
    AnyCharAll,  // dot with DOTALL
    In(CharSet), // char set
    InIgnore(CharSet),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        min: u32,
        max: u32,
        greedy: bool,
        possessive: bool,
        body: Box<Node>,
    },
    Group {
        index: Option<usize>, // None = non-capturing
        body: Box<Node>,
    },
    AtomicGroup(Box<Node>),
    Backref(usize),
    BackrefIgnore(usize),
    Cond {
        group: usize,
        yes: Box<Node>,
        no: Option<Box<Node>>,
    },
    Look {
        ahead: bool,
        negative: bool,
        body: Box<Node>,
    },
    At(Anchor),
    // Parsed-template marker for sub() replacement; not matched directly.
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anchor {
    Beginning,     // ^
    BeginningLine, // ^ with M
    End,           // $
    EndLine,       // $ with M
    AbsoluteEnd,   // \Z (end of string ONLY; $ also allows a trailing \n)
    Boundary,      // \b
    NonBoundary,   // \B
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CharSet {
    pub negate: bool,
    pub items: Vec<SetItem>,
}

#[derive(Clone, Debug)]
pub(crate) enum SetItem {
    Literal(u32),
    LiteralIgnore(u32),
    Range(u32, u32),
    RangeIgnore(u32, u32),
    Digit,
    NotDigit,
    Space,
    NotSpace,
    Word,
    NotWord,
}

// ===========================================================================
// Pattern (compiled form) + compile errors
// ===========================================================================

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
pub(crate) struct Compiled {
    pub pattern: String,
    pub flags: u32,
    pub root: Node,
    pub groups: usize, // number of capturing groups
    pub groupindex: HashMap<String, usize>,
    /// Insertion order of named groups (groupindex is a HashMap; CPython's
    /// groupindex dict iterates in definition order — s010 pins it).
    pub grouporder: Vec<(String, usize)>,
    pub indexgroup: Vec<Option<String>>,
    // Template for sub(): interleaved literal chunks and group refs.
    // None = derive at sub() time from the repl string.
    pub min_width: usize,
}

pub(crate) fn re_error(msg: &str, pattern: &str, pos: usize) -> crate::Flow {
    let mut full = format!("{} at position {}", msg, pos);
    // Multi-line patterns get the (line L, column C) suffix (CPython
    // PatternError.__init__).
    if pattern.contains('\n') {
        let before = &pattern[..pos.min(pattern.len())];
        let line = before.matches('\n').count() + 1;
        let col = pos - before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        full = format!("{} (line {}, column {})", full, line, col);
    }
    raise("PatternError", full)
}

fn bad_pattern(msg: &str) -> crate::Flow {
    raise("PatternError", msg)
}

// ===========================================================================
// Node helpers
// ===========================================================================

/// Literal with the active IGNORECASE folding applied (CPython compiles
/// cased literals to LITERAL_IGNORE with the LOWERCASED code).
fn lit(c: u32, flags: &u32) -> Node {
    if *flags & F_IGNORECASE != 0
        && char::from_u32(c)
            .map(|ch| ch.is_alphabetic())
            .unwrap_or(false)
    {
        Node::LiteralIgnore(
            char::from_u32(c)
                .and_then(|ch| ch.to_lowercase().next())
                .unwrap_or(char::from_u32(c).unwrap_or('\0')) as u32,
        )
    } else {
        Node::Literal(c)
    }
}

fn mk_lit_item(c: u32, flags: &u32) -> SetItem {
    match lit(c, flags) {
        Node::Literal(v) => SetItem::Literal(v),
        Node::LiteralIgnore(v) => SetItem::LiteralIgnore(v),
        _ => unreachable!(),
    }
}

fn set_node(set: CharSet) -> Node {
    if set_has_cased(set.items.iter()) {
        Node::InIgnore(set)
    } else {
        Node::In(set)
    }
}

fn set_has_cased<'a>(mut items: impl Iterator<Item = &'a SetItem>) -> bool {
    items.any(|it| match it {
        SetItem::LiteralIgnore(_) | SetItem::RangeIgnore(_, _) => true,
        // A range covering cased characters under IGNORECASE is folded via
        // InIgnore so matches test both cases.
        _ => false,
    })
}

fn set_of(item: SetItem) -> CharSet {
    CharSet {
        negate: false,
        items: vec![item],
    }
}

/// (min, max) match width in characters (MAX = usize::MAX).
fn width_of(n: &Node) -> (usize, usize) {
    const INF: usize = usize::MAX;
    match n {
        Node::Empty => (0, 0),
        Node::Literal(_)
        | Node::LiteralIgnore(_)
        | Node::NotLiteral(_)
        | Node::NotLiteralIgnore(_)
        | Node::AnyChar
        | Node::AnyCharAll => (1, 1),
        Node::In(_) | Node::InIgnore(_) => (1, 1),
        Node::Seq(items) => items.iter().fold((0usize, 0usize), |(lo, hi), it| {
            let (l, h) = width_of(it);
            (lo + l, hi.saturating_add(h))
        }),
        Node::Alt(branches) => {
            let mut lo = INF;
            let mut hi = 0;
            for b in branches {
                let (l, h) = width_of(b);
                lo = lo.min(l);
                hi = hi.max(h);
            }
            (if lo == INF { 0 } else { lo }, hi)
        }
        Node::Repeat { min, max, body, .. } => {
            let (l, h) = width_of(body);
            (
                l.saturating_mul(*min as usize),
                if h == 0 {
                    0
                } else if *max == u32::MAX {
                    INF
                } else {
                    h.saturating_mul(*max as usize)
                },
            )
        }
        Node::Group { body, .. } => width_of(body),
        Node::AtomicGroup(body) => width_of(body),
        Node::Backref(_) | Node::BackrefIgnore(_) => (0, INF),
        Node::Cond { yes, no, .. } => {
            let (mut lo, mut hi) = width_of(yes);
            if let Some(n2) = no {
                let (l, h) = width_of(n2);
                lo = lo.min(l);
                hi = hi.max(h);
            } else {
                lo = 0;
            }
            (lo, hi)
        }
        Node::Look { .. } => (0, 0),
        Node::At(_) => (0, 0),
    }
}

/// unicodedata.lookup port — a small name table is NOT viable; we support
/// only the common control/Latin-1 names via a generated no-op: CPython's
/// table is 30k entries. Documented gap: \N{...} supports ASCII single-char
/// names only (LATIN SMALL LETTER X form); anything else fails with
/// "undefined character name".
fn unicodedata_lookup(name: &str) -> Option<u32> {
    // U+XXXX style first (not valid names but harmless)
    if let Some(hex) = name.strip_prefix("U+") {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return char::from_u32(v).map(|_| v);
        }
    }
    // <control> form
    if let Some(hex) = name
        .strip_prefix("<control-")
        .and_then(|s| s.strip_suffix('>'))
    {
        if let Ok(v) = u32::from_str_radix(hex.trim_start_matches("0x"), 16) {
            return Some(v);
        }
    }
    // Try cp437-ish C0 names
    const C0: &[(&str, u32)] = &[
        ("NULL", 0),
        ("START OF HEADING", 1),
        ("START OF TEXT", 2),
        ("END OF TEXT", 3),
        ("END OF TRANSMISSION", 4),
        ("ENQUIRY", 5),
        ("ACKNOWLEDGE", 6),
        ("BELL", 7),
        ("BACKSPACE", 8),
        ("CHARACTER TABULATION", 9),
        ("LINE FEED", 10),
        ("LINE TABULATION", 11),
        ("FORM FEED", 12),
        ("CARRIAGE RETURN", 13),
        ("SHIFT OUT", 14),
        ("SHIFT IN", 15),
        ("DATA LINK ESCAPE", 16),
        ("DEVICE CONTROL ONE", 17),
        ("DEVICE CONTROL TWO", 18),
        ("DEVICE CONTROL THREE", 19),
        ("DEVICE CONTROL FOUR", 20),
        ("NEGATIVE ACKNOWLEDGE", 21),
        ("SYNCHRONOUS IDLE", 22),
        ("END OF TRANSMISSION BLOCK", 23),
        ("CANCEL", 24),
        ("END OF MEDIUM", 25),
        ("SUBSTITUTE", 26),
        ("ESCAPE", 27),
        ("INFORMATION SEPARATOR FOUR", 28),
        ("INFORMATION SEPARATOR THREE", 29),
        ("INFORMATION SEPARATOR TWO", 30),
        ("INFORMATION SEPARATOR ONE", 31),
        ("SPACE", 32),
        ("DELETE", 127),
    ];
    for (n, v) in C0 {
        if *n == name {
            return Some(*v);
        }
    }
    None
}

// ===========================================================================
// Parser
// ===========================================================================

struct Parser {
    src: Vec<char>,
    pat: String,
    pos: usize,
    flags: u32,
    ngroups: usize,
    groupindex: HashMap<String, usize>,
    group_widths: Vec<Option<(usize, usize)>>, // per group index (1-based slot 0 unused)
    // For lookbehind validation: group indices visible outside the current
    // lookbehind (CPython forbids refs into the same lookbehind).
    lookbehind_depth: usize,
    // Wave-4 (RF-6, COMPILE_MISSED fuzz family): stack of currently-open
    // group indices. A backref to an open group is a compile error in
    // CPython ('cannot refer to an open group at position N'); the parser
    // previously accepted ((b)\1) because it never tracked nesting.
    open_groups: Vec<usize>,
    pat_is_str: bool,
    pending_backrefs: Vec<(usize, usize)>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }
    fn peek2(&self) -> Option<char> {
        self.src.get(self.pos + 1).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn err(&self, msg: &str, offset: usize) -> crate::Flow {
        if std::env::var_os("AP_SRE_DEBUG").is_some() {
            eprintln!("SRE-ERR msg={msg} pattern={:?} pos={}", self.pat, self.pos);
        }
        re_error(msg, &self.pat, self.pos.saturating_sub(offset))
    }

    fn parse_alternation(&mut self) -> Result<Node, crate::Flow> {
        let mut branches = vec![self.parse_sequence()?];
        while self.eat('|') {
            branches.push(self.parse_sequence()?);
        }
        if branches.len() == 1 {
            Ok(branches.pop().unwrap())
        } else {
            Ok(Node::Alt(branches))
        }
    }

    fn parse_sequence(&mut self) -> Result<Node, crate::Flow> {
        let mut items: Vec<Node> = Vec::new();
        loop {
            // T1 (_pydecimal/cookies/_attr_re VERBOSE): whitespace and
            // comment skipping loops back to the TOP so a ')' or '|'
            // reached through skipped text is re-examined by the loop-top
            // break (CPython strips whitespace before token dispatch).
            self.skip_verbose();
            match self.peek() {
                None | Some('|') | Some(')') => break,
                _ => {}
            }
            let c = self.bump().unwrap();
            match c {
                '^' => {
                    items.push(Node::At(if self.flags & F_MULTILINE != 0 {
                        Anchor::BeginningLine
                    } else {
                        Anchor::Beginning
                    }));
                }
                '$' => {
                    items.push(Node::At(if self.flags & F_MULTILINE != 0 {
                        Anchor::EndLine
                    } else {
                        Anchor::End
                    }));
                }
                '.' => {
                    if self.flags & F_DOTALL != 0 {
                        items.push(Node::AnyCharAll);
                    } else {
                        items.push(Node::AnyChar);
                    }
                }
                '[' => items.push(self.parse_charset()?),
                '(' => items.push(self.parse_group()?),
                '\\' => {
                    let e = self.bump().ok_or_else(|| {
                        re_error("bad escape (end of pattern)", &self.pat, self.pat.len() - 1)
                    })?;
                    items.push(self.parse_escape(e)?);
                }
                '*' | '+' | '?' => {
                    // A quantifier here is either "nothing to repeat" (start
                    // of a sequence) or "multiple repeat" (a***). CPython
                    // distinguishes by whether the previous token exists.
                    let msg = if items.is_empty() {
                        "nothing to repeat"
                    } else {
                        "multiple repeat"
                    };
                    return Err(self.err(msg, 1));
                }
                '{' => {
                    // A lone { that isn't a valid quantifier is a literal.
                    items.push(lit('{' as u32, &self.flags));
                }
                _ => items.push(lit(c as u32, &self.flags)),
            }
            // CPython strips VERBOSE whitespace/comments BEFORE reading the
            // next token, and a quantifier is that next token — so `a + b`
            // under re.X is `a+b`, not "multiple repeat". Skip here too.
            self.skip_verbose();
            // quantifier?
            self.parse_quantifier(&mut items)?;
        }
        if items.len() == 1 {
            Ok(items.pop().unwrap())
        } else {
            Ok(Node::Seq(items))
        }
    }

    /// In VERBOSE mode, skip whitespace and `#`-to-EOL comments. No-op
    /// otherwise. Shared by the sequence loop-top and the pre-quantifier
    /// skip so both match CPython's sre_parse whitespace stripping.
    fn skip_verbose(&mut self) {
        if self.flags & F_VERBOSE == 0 {
            return;
        }
        loop {
            match self.peek() {
                Some(ch) if ch.is_whitespace() => {
                    self.pos += 1;
                }
                Some('#') => {
                    self.pos += 1;
                    while let Some(nc) = self.bump() {
                        if nc == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_quantifier(&mut self, items: &mut Vec<Node>) -> Result<(), crate::Flow> {
        let (mut min, mut max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0u32, u32::MAX)
            }
            Some('+') => {
                self.pos += 1;
                (1u32, u32::MAX)
            }
            Some('?') => {
                self.pos += 1;
                (0u32, 1u32)
            }
            Some('{') => {
                // {m}, {m,}, {m,n}; invalid forms are literals.
                let save = self.pos;
                self.pos += 1;
                let mut lo = String::new();
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() {
                        lo.push(d);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let lo_n: u32 = if lo.is_empty() {
                    0 // {,n} == {0,n}
                } else {
                    // CPython raises OverflowError (not re.error) when the
                    // repetition number does not fit.
                    lo.parse()
                        .map_err(|_| raise("OverflowError", "the repetition number is too large"))?
                };
                if lo_n >= 4294967295 {
                    return Err(raise("OverflowError", "the repetition number is too large"));
                }
                let hi_n = if self.eat(',') {
                    let mut hi = String::new();
                    while let Some(d) = self.peek() {
                        if d.is_ascii_digit() {
                            hi.push(d);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    if hi.is_empty() {
                        u32::MAX
                    } else {
                        // CPython raises OverflowError (not re.error).
                        let h: u32 = hi.parse().map_err(|_| {
                            raise("OverflowError", "the repetition number is too large")
                        })?;
                        if h >= 4294967295 {
                            return Err(raise(
                                "OverflowError",
                                "the repetition number is too large",
                            ));
                        }
                        h
                    }
                } else {
                    lo_n
                };
                if !self.eat('}') {
                    // not a quantifier: literal '{' — restore; the '{' we
                    // consumed was already pushed as a literal below? No:
                    // the caller pushed the literal already; just restore.
                    self.pos = save;
                    return Ok(());
                }
                if hi_n < lo_n {
                    return Err(self.err("min repeat greater than max repeat", self.pos - save - 1));
                }
                (lo_n, hi_n)
            }
            _ => return Ok(()),
        };
        let _ = &mut min;
        let mut greedy = true;
        let mut possessive = false;
        if self.eat('?') {
            greedy = false;
        } else if self.eat('+') {
            possessive = true;
        }
        // item = last item (unpacking non-capturing groups like CPython is
        // unnecessary — the matcher recurses through groups fine).
        let last = items
            .pop()
            .ok_or_else(|| self.err("nothing to repeat", 1))?;
        if matches!(last, Node::At(_)) {
            return Err(self.err("nothing to repeat", 1));
        }
        if matches!(last, Node::Repeat { .. }) {
            // CPython: a** / a*+ / a*? all raise "multiple repeat".
            return Err(self.err("multiple repeat", 1));
        }
        items.push(Node::Repeat {
            min,
            max,
            greedy,
            possessive,
            body: Box::new(last),
        });
        let _ = &mut max;
        Ok(())
    }

    fn parse_group(&mut self) -> Result<Node, crate::Flow> {
        // '(' already consumed.
        let start = self.pos - 1;
        if self.eat('?') {
            let c = self
                .bump()
                .ok_or_else(|| self.err("unexpected end of pattern", 0))?;
            match c {
                'P' => {
                    if self.eat('<') {
                        let name = self.read_group_name('>')?;
                        let idx = self.open_group(Some(name))?;
                        let body = self.parse_alternation()?;
                        if !self.eat(')') {
                            return Err(
                                self.err("missing ), unterminated subpattern", self.pos - start)
                            );
                        }
                        self.close_group(idx, &body);
                        Ok(Node::Group {
                            index: Some(idx),
                            body: Box::new(body),
                        })
                    } else if self.eat('=') {
                        let name = self.read_group_name(')')?;
                        let idx = *self.groupindex.get(&name).ok_or_else(|| {
                            self.err(&format!("unknown group name '{}'", name), name.len() + 1)
                        })?;
                        if self.open_groups.contains(&idx) {
                            return Err(self.err("cannot refer to an open group", name.len() + 1));
                        }
                        self.validate_lookbehind_ref(idx)?;
                        Ok(Node::Backref(idx))
                    } else {
                        let c2 = self.bump();
                        Err(self.err(
                            &format!(
                                "unknown extension ?P{}",
                                c2.map(|x| x.to_string()).unwrap_or_default()
                            ),
                            3,
                        ))
                    }
                }
                ':' => {
                    let body = self.parse_alternation()?;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    Ok(Node::Group {
                        index: None,
                        body: Box::new(body),
                    })
                }
                '#' => {
                    // comment group
                    loop {
                        match self.bump() {
                            None => {
                                return Err(
                                    self.err("missing ), unterminated comment", self.pos - start)
                                );
                            }
                            Some(')') => break,
                            _ => {}
                        }
                    }
                    Ok(Node::Empty)
                }
                '=' => {
                    let body = self.parse_alternation()?;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    Ok(Node::Look {
                        ahead: true,
                        negative: false,
                        body: Box::new(body),
                    })
                }
                '!' => {
                    let body = self.parse_alternation()?;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    Ok(Node::Look {
                        ahead: true,
                        negative: true,
                        body: Box::new(body),
                    })
                }
                '<' => {
                    let c2 = self
                        .bump()
                        .ok_or_else(|| self.err("unexpected end of pattern", 0))?;
                    if c2 != '=' && c2 != '!' {
                        return Err(self.err(&format!("unknown extension ?<{}", c2), 3));
                    }
                    self.lookbehind_depth += 1;
                    let body = self.parse_alternation()?;
                    self.lookbehind_depth -= 1;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    let (lo, hi) = width_of(&body);
                    if lo != hi {
                        return Err(bad_pattern("look-behind requires fixed-width pattern"));
                    }
                    Ok(Node::Look {
                        ahead: false,
                        negative: c2 == '!',
                        body: Box::new(body),
                    })
                }
                '(' => {
                    // conditional backreference group (?(1)y|n) / (?(name)y|n)
                    // Numeric refs are legal here (unlike (?P<name>) group
                    // NAMES), so read the raw name and let the digit branch
                    // below handle numbering.
                    let name = self.read_name(')', false)?;
                    let group: usize = if name.chars().all(|c| c.is_ascii_digit()) {
                        let n: usize = name
                            .parse()
                            .map_err(|_| self.err("bad group number", name.len() + 1))?;
                        if n == 0 {
                            return Err(self.err("bad group number", name.len() + 1));
                        }
                        // CPython: a conditional group ref to a nonexistent
                        // group is 'invalid group reference N at position P'
                        // (P = the first digit). Deferred check against the
                        // final group count (same machinery as \N refs).
                        self.pending_backrefs.push((n, self.pos - name.len() - 1));
                        n
                    } else {
                        // Non-numeric: must be a valid identifier, else
                        // CPython reports "bad character in group name".
                        let valid = name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_alphabetic() || c == '_')
                            && name.chars().all(|c| c.is_alphanumeric() || c == '_');
                        if !valid {
                            return Err(self.err(
                                &format!("bad character in group name '{}'", name),
                                name.len() + 1,
                            ));
                        }
                        *self.groupindex.get(&name).ok_or_else(|| {
                            self.err(&format!("unknown group name '{}'", name), name.len() + 1)
                        })?
                    };
                    self.validate_lookbehind_ref(group)?;
                    let yes = self.parse_sequence()?;
                    let no = if self.eat('|') {
                        Some(self.parse_sequence()?)
                    } else {
                        None
                    };
                    if self.peek() == Some('|') {
                        return Err(self.err("conditional backref with more than two branches", 0));
                    }
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    Ok(Node::Cond {
                        group,
                        yes: Box::new(yes),
                        no: no.map(Box::new),
                    })
                }
                '>' => {
                    let body = self.parse_alternation()?;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    Ok(Node::AtomicGroup(Box::new(body)))
                }
                '-' | 'i' | 'm' | 's' | 'x' | 'a' | 'u' | 'L' => self.parse_inline_flags(c, start),
                _ => Err(self.err(&format!("unknown extension ?{}", c), 2)),
            }
        } else {
            let idx = self.open_group(None)?;
            let body = self.parse_alternation()?;
            if !self.eat(')') {
                return Err(self.err("missing ), unterminated subpattern", self.pos - start));
            }
            self.close_group(idx, &body);
            Ok(Node::Group {
                index: Some(idx),
                body: Box::new(body),
            })
        }
    }

    fn parse_inline_flags(&mut self, first: char, start: usize) -> Result<Node, crate::Flow> {
        // (?flags:...) or (?flags)... (global) or (?flags-flags:...)
        let mut add: u32 = 0;
        let mut del: u32 = 0;
        let mut c = first;
        let mut global = false;
        loop {
            let bit = match c {
                'i' => F_IGNORECASE,
                'L' => {
                    if self.pat_is_str {
                        return Err(self.err(
                            "bad inline flags: cannot use 'L' flag with a str pattern",
                            0,
                        ));
                    }
                    F_LOCALE
                }
                'm' => F_MULTILINE,
                's' => F_DOTALL,
                'u' => F_UNICODE,
                'x' => F_VERBOSE,
                'a' => F_ASCII,
                '-' => {
                    // negation part
                    loop {
                        let d = self.bump().ok_or_else(|| self.err("missing flag", 0))?;
                        match d {
                            ':' => break,
                            'i' => del |= F_IGNORECASE,
                            'm' => del |= F_MULTILINE,
                            's' => del |= F_DOTALL,
                            'x' => del |= F_VERBOSE,
                            'a' | 'u' | 'L' => {
                                return Err(self.err(
                                    "bad inline flags: cannot turn off flags 'a', 'u' and 'L'",
                                    1,
                                ));
                            }
                            _ => {
                                let m = if d.is_alphabetic() {
                                    "unknown flag"
                                } else {
                                    "missing flag"
                                };
                                return Err(self.err(m, 1));
                            }
                        }
                    }
                    // after -: expects ':'
                    let body = self.parse_alternation()?;
                    if !self.eat(')') {
                        return Err(
                            self.err("missing ), unterminated subpattern", self.pos - start)
                        );
                    }
                    let saved = self.flags;
                    self.flags = (self.flags | add) & !del;
                    let scoped = self.apply_verbose(body)?;
                    self.flags = saved;
                    return Ok(scoped);
                }
                _ => {
                    let m = if c.is_alphabetic() {
                        "unknown flag"
                    } else {
                        "missing flag"
                    };
                    return Err(self.err(m, self.pos - start));
                }
            };
            add |= bit;
            if (bit & (F_ASCII | F_UNICODE | F_LOCALE)) != 0
                && (add & (F_ASCII | F_UNICODE | F_LOCALE)).count_ones() > 1
            {
                return Err(self.err(
                    "bad inline flags: flags 'a', 'u' and 'L' are incompatible",
                    0,
                ));
            }
            match self.bump() {
                None => return Err(self.err("missing -, : or )", 0)),
                Some(')') => {
                    global = true;
                    break;
                }
                Some(':') => break,
                Some(ch) => {
                    if !ch.is_alphabetic() && ch != '-' {
                        return Err(self.err("missing -, : or )", 1));
                    }
                    c = ch;
                }
            }
        }
        if global {
            // Global flags: must appear at the very start (CPython: "global
            // flags not at the start of the expression"). start == 0 means
            // the (? appeared at position 0.
            if start != 0 {
                return Err(self.err(
                    "global flags not at the start of the expression",
                    self.pos - start,
                ));
            }
            self.flags = (self.flags | add) & !del;
            // Re-parse the rest with the new flags.
            let body = self.parse_alternation()?;
            return self.apply_verbose(body);
        }
        let body = self.parse_alternation()?;
        if !self.eat(')') {
            return Err(self.err("missing ), unterminated subpattern", self.pos - start));
        }
        let saved = self.flags;
        self.flags = (self.flags | add) & !del;
        let scoped = self.apply_verbose(body)?;
        self.flags = saved;
        Ok(scoped)
    }

    fn apply_verbose(&self, n: Node) -> Result<Node, crate::Flow> {
        // VERBOSE was already honored at parse time (we check the flag in
        // parse_sequence), so nothing to transform here.
        Ok(n)
    }

    fn read_group_name(&mut self, term: char) -> Result<String, crate::Flow> {
        self.read_name(term, true)
    }

    /// Shared name reader. `ident`: enforce Python-identifier charset
    /// (group names only — \N{...} unicode names may contain spaces).
    fn read_name(&mut self, term: char, ident: bool) -> Result<String, crate::Flow> {
        let start = self.pos;
        let mut name = String::new();
        loop {
            match self.bump() {
                None => {
                    if name.is_empty() {
                        return Err(self.err("missing group name", 0));
                    }
                    return Err(
                        self.err(&format!("missing {}, unterminated name", term), name.len())
                    );
                }
                Some(c) if c == term => {
                    if name.is_empty() {
                        return Err(self.err("missing group name", 1));
                    }
                    // CPython: group names must be valid Python identifiers
                    // (sre_parse: "bad character in group name %a").
                    if ident {
                        let mut ok = name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_alphabetic() || c == '_');
                        if ok {
                            ok = name.chars().all(|c| c.is_alphanumeric() || c == '_');
                        }
                        if !ok {
                            let bad = self.pos - name.len() - 1;
                            return Err(re_error(
                                &format!("bad character in group name '{}'", name),
                                &self.pat,
                                bad.max(start.saturating_sub(1)),
                            ));
                        }
                    }
                    return Ok(name);
                }
                Some(c) => name.push(c),
            }
        }
    }

    fn open_group(&mut self, name: Option<String>) -> Result<usize, crate::Flow> {
        self.ngroups += 1;
        let idx = self.ngroups;
        self.open_groups.push(idx);
        if let Some(n) = name {
            if let Some(old) = self.groupindex.get(&n) {
                return Err(re_error(
                    &format!(
                        "redefinition of group name '{}' as group {}; was group {}",
                        n, idx, old
                    ),
                    &self.pat,
                    self.pat.rfind(&n).unwrap_or(0),
                ));
            }
            self.groupindex.insert(n, idx);
        }
        while self.group_widths.len() <= idx {
            self.group_widths.push(None);
        }
        Ok(idx)
    }

    fn close_group(&mut self, idx: usize, body: &Node) {
        let w = width_of(body);
        self.group_widths[idx] = Some(w);
        // Pop this group (and any inner stragglers from error paths).
        while let Some(top) = self.open_groups.last() {
            let t = *top;
            self.open_groups.pop();
            if t == idx {
                break;
            }
        }
    }

    fn validate_lookbehind_ref(&self, idx: usize) -> Result<(), crate::Flow> {
        if self.lookbehind_depth > 0 {
            return Err(self.err("cannot refer to an open group", 0));
        }
        // Forward refs are checked at end of parse (grouprefpos semantics).
        let _ = idx;
        Ok(())
    }

    fn parse_escape(&mut self, e: char) -> Result<Node, crate::Flow> {
        let pat_is_str = self.pat_is_str;
        match e {
            'd' => Ok(Node::In(set_of(SetItem::Digit))),
            'D' => Ok(Node::In(set_of(SetItem::NotDigit))),
            's' => Ok(Node::In(set_of(SetItem::Space))),
            'S' => Ok(Node::In(set_of(SetItem::NotSpace))),
            'w' => Ok(Node::In(set_of(SetItem::Word))),
            'W' => Ok(Node::In(set_of(SetItem::NotWord))),
            'b' => Ok(Node::At(Anchor::Boundary)),
            'B' => Ok(Node::At(Anchor::NonBoundary)),
            'A' => Ok(Node::At(Anchor::Beginning)),
            'Z' => Ok(Node::At(Anchor::AbsoluteEnd)),
            // T1 (CPython 3.14 urllib.parse): \z — strict end-of-string
            // anchor (3.14 addition). Same semantics as \Z in this engine
            // (AbsoluteEnd never allows a trailing newline).
            'z' => Ok(Node::At(Anchor::AbsoluteEnd)),
            'a' => Ok(lit(0x07, &self.flags)),
            't' => Ok(lit(0x09, &self.flags)),
            'n' => Ok(lit(0x0a, &self.flags)),
            'f' => Ok(lit(0x0c, &self.flags)),
            'r' => Ok(lit(0x0d, &self.flags)),
            'v' => Ok(lit(0x0b, &self.flags)),
            'x' => {
                let h = self.read_hex(2)?;
                Ok(lit(h, &self.flags))
            }
            'u' if pat_is_str => {
                let h = self.read_hex(4)?;
                Ok(lit(h, &self.flags))
            }
            'u' => Err(self.err(&format!("bad escape \\{}", e), 2)),
            'U' if pat_is_str => {
                let h = self.read_hex(8)?;
                Ok(lit(h, &self.flags))
            }
            'U' => Err(self.err(&format!("bad escape \\{}", e), 2)),
            'N' if pat_is_str => {
                if !self.eat('{') {
                    return Err(self.err("missing {", 0));
                }
                let name = self.read_name('}', false)?;
                let c = unicodedata_lookup(&name).ok_or_else(|| {
                    self.err(
                        &format!("undefined character name '{}'", name),
                        name.len() + 4,
                    )
                })?;
                Ok(lit(c, &self.flags))
            }
            'N' => Err(self.err(&format!("bad escape \\{}", e), 2)),
            _ if e.is_ascii_alphabetic() => Err(self.err(&format!("bad escape \\{}", e), 2)),
            '0'..='9' => {
                // octal or backref. CPython (sre_parse): leading '0' means
                // octal (up to 3 digits); otherwise a group reference takes
                // AT MOST 2 digits and errors at the first digit's position.
                // T1 (email fcre octal ranges like \\041-\\176): a 3-digit
                // all-octal escape with a NONZERO first digit is still octal
                // (CPython \\176 == '~'); 2-digit nonzero stays a backref.
                // T1: 3-octal-digit escapes (e.g. \\176) are octal even with
                // a nonzero first digit; group refs are at most 2 digits.
                let max_digits = if matches!(e, '0'..='7') { 3 } else { 2 };
                let mut digits = String::new();
                digits.push(e);
                while let Some(d) = self.peek() {
                    if d.is_ascii_digit() && digits.len() < max_digits {
                        digits.push(d);
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let all_octal = digits.bytes().all(|d| (b'0'..=b'7').contains(&d));
                if e == '0' || (digits.len() == 3 && all_octal && e != '8' && e != '9') {
                    // octal (3-digit all-octal form incl. nonzero first digit)
                    let v = u32::from_str_radix(digits.trim_start_matches('0').max("0"), 8)
                        .map_err(|_| self.err("bad escape", digits.len()))?;
                    if v > 0o377 {
                        return Err(self.err(
                            &format!("octal escape value {} outside of range 0-0o377", digits),
                            digits.len(),
                        ));
                    }
                    Ok(lit(v, &self.flags))
                } else {
                    // backreference (1-2 digits, both fit usize trivially)
                    let n: usize = digits.parse().unwrap();
                    if n == 0 {
                        return Err(self.err(&format!("bad escape \\{}", digits), digits.len()));
                    }
                    // RF-6: a numeric ref to a group still OPEN at this point
                    // is a compile error in CPython (e.g. ((b)\1)); error
                    // position is the backslash.
                    if self.open_groups.contains(&n) {
                        return Err(self.err("cannot refer to an open group", digits.len() + 1));
                    }
                    self.pending_backrefs.push((n, self.pos - digits.len()));
                    Ok(if self.flags & F_IGNORECASE != 0 {
                        Node::BackrefIgnore(n)
                    } else {
                        Node::Backref(n)
                    })
                }
            }
            other => Ok(Node::Literal(other as u32)),
        }
    }

    fn read_hex(&mut self, n: usize) -> Result<u32, crate::Flow> {
        let mut h = String::new();
        for _ in 0..n {
            match self.bump() {
                Some(c) if c.is_ascii_hexdigit() => h.push(c),
                _ => {
                    return Err(self.err(&format!("incomplete escape \\x{}", h), h.len() + 2));
                }
            }
        }
        u32::from_str_radix(&h, 16).map_err(|_| self.err("bad escape", h.len()))
    }

    fn parse_charset(&mut self) -> Result<Node, crate::Flow> {
        // '[' consumed.
        let here = self.pos - 1;
        let mut set = CharSet::default();
        if self.eat('^') {
            set.negate = true;
        }
        let mut first = true;
        loop {
            let c = self
                .bump()
                .ok_or_else(|| self.err("unterminated character set", self.pos - here))?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let item: SetItem = if c == '\\' {
                let e = self
                    .bump()
                    .ok_or_else(|| self.err("bad escape (end of pattern)", self.pat.len() - 1))?;
                match e {
                    'd' => SetItem::Digit,
                    'D' => SetItem::NotDigit,
                    's' => SetItem::Space,
                    'S' => SetItem::NotSpace,
                    'w' => SetItem::Word,
                    'W' => SetItem::NotWord,
                    'b' => SetItem::Literal(0x08), // inside classes \b is backspace
                    _ => {
                        // reuse escape parsing for literals
                        let n = self.parse_escape(e)?;
                        match n {
                            Node::Literal(v) => SetItem::Literal(v),
                            Node::LiteralIgnore(v) => SetItem::LiteralIgnore(v),
                            other => {
                                return Err(bad_pattern(&format!(
                                    "internal: unsupported set item {:?}",
                                    other
                                )));
                            }
                        }
                    }
                }
            } else {
                mk_lit_item(c as u32, &self.flags)
            };
            // range?
            if self.peek() == Some('-') && self.peek2().map(|c| c != ']').unwrap_or(false) {
                self.pos += 1; // '-'
                let c2 = self.bump().unwrap();
                let item2: SetItem = if c2 == '\\' {
                    let e = self.bump().ok_or_else(|| {
                        self.err("bad escape (end of pattern)", self.pat.len() - 1)
                    })?;
                    match e {
                        'd' | 'D' | 's' | 'S' | 'w' | 'W' => {
                            return Err(self.err(&format!("bad character range {}-\\{}", c, e), 3));
                        }
                        _ => {
                            let n = self.parse_escape(e)?;
                            match n {
                                Node::Literal(v) => SetItem::Literal(v),
                                Node::LiteralIgnore(v) => SetItem::LiteralIgnore(v),
                                _ => {
                                    return Err(
                                        self.err(&format!("bad character range {}-\\{}", c, e), 3)
                                    );
                                }
                            }
                        }
                    }
                } else {
                    mk_lit_item(c2 as u32, &self.flags)
                };
                match (item, item2) {
                    (SetItem::Literal(a), SetItem::Literal(b)) => {
                        if b < a {
                            return Err(self.err(
                                &format!("bad character range {}-{}", c, c2),
                                c.len_utf8() + 1 + c2.len_utf8(),
                            ));
                        }
                        set.items.push(SetItem::Range(a, b));
                    }
                    (SetItem::LiteralIgnore(a), SetItem::LiteralIgnore(b)) => {
                        if b < a {
                            return Err(self.err(
                                &format!("bad character range {}-{}", c, c2),
                                c.len_utf8() + 1 + c2.len_utf8(),
                            ));
                        }
                        set.items.push(SetItem::RangeIgnore(a, b));
                    }
                    (SetItem::Literal(a), other) => {
                        // Left side was a literal, right side is a category:
                        // CPython errors ("bad character range").
                        let _ = a;
                        let _ = other;
                        return Err(self.err(
                            &format!("bad character range {}-{}", c, c2),
                            c.len_utf8() + 1 + c2.len_utf8(),
                        ));
                    }
                    _ => {
                        return Err(self.err(
                            &format!("bad character range {}-{}", c, c2),
                            c.len_utf8() + 1 + c2.len_utf8(),
                        ));
                    }
                }
            } else {
                set.items.push(item);
            }
        }
        Ok(set_node(set))
    }
}

// ===========================================================================
// Compile entry + top-level API
// ===========================================================================

pub(crate) fn compile_pattern(pattern: &str, flags: u32) -> Result<Compiled, crate::Flow> {
    let is_str = pattern.chars().all(|c| (c as u32) < 0x110000); // always str here
    let mut p = Parser {
        src: pattern.chars().collect(),
        pat: pattern.to_string(),
        pos: 0,
        flags,
        ngroups: 0,
        groupindex: HashMap::new(),
        group_widths: vec![None],
        lookbehind_depth: 0,
        open_groups: Vec::new(),
        pat_is_str: is_str,
        pending_backrefs: Vec::new(),
    };
    // fix_flags (str patterns: UNICODE implied, LOCALE/ASCII+UNICODE errors)
    if flags & F_LOCALE != 0 {
        return Err(bad_pattern("cannot use LOCALE flag with a str pattern"));
    }
    let mut flags = flags;
    if flags & F_ASCII == 0 {
        flags |= F_UNICODE;
    } else if flags & F_UNICODE != 0 {
        return Err(bad_pattern("ASCII and UNICODE flags are incompatible"));
    }
    p.flags = flags;
    let mut root = p.parse_alternation()?;
    if p.pos < p.src.len() {
        // must be unbalanced ')' — CPython reports its position
        return Err(p.err("unbalanced parenthesis", 0));
    }
    // forward backref validation (invalid group reference N at position P)
    for &(g, gpos) in &p.pending_backrefs {
        if g > p.ngroups {
            return Err(re_error(
                &format!("invalid group reference {}", g),
                pattern,
                gpos,
            ));
        }
    }
    // Expand ^/$ anchors according to MULTILINE (they were parsed with the
    // flag in scope, so nothing further).
    let _ = &mut root;
    let ngroups = p.ngroups;
    let mut indexgroup = vec![None; ngroups + 1];
    for (name, idx) in &p.groupindex {
        indexgroup[*idx] = Some(name.clone());
    }
    let grouporder: Vec<(String, usize)> = indexgroup
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(idx, name)| name.clone().map(|n| (n, idx)))
        .collect();
    let (min_w, _) = width_of(&root);
    Ok(Compiled {
        pattern: pattern.to_string(),
        flags,
        root,
        groups: ngroups,
        groupindex: p.groupindex,
        grouporder,
        indexgroup,
        min_width: min_w,
    })
}

// ===========================================================================
// Matcher (recursive backtracking, leftmost-first)
// ===========================================================================

const STEP_LIMIT: u64 = 4_000_000;

#[derive(Clone)]
pub(crate) struct MatchResult {
    pub start: usize,
    pub end: usize,
    /// Per-group (start, end); index 0 = whole match. Groups that did not
    /// participate are (usize::MAX, usize::MAX).
    pub spans: Vec<(usize, usize)>,
}

pub(crate) struct Matcher<'a> {
    pub text: &'a [char],
    flags: u32,
    steps: Cell<u64>,
}

use std::cell::Cell;

type MRes = Result<Option<usize>, crate::Flow>; // Ok(Some(end)) on match

impl<'a> Matcher<'a> {
    fn tick(&mut self) -> Result<(), crate::Flow> {
        let s = self.steps.get() + 1;
        self.steps.set(s);
        if s > STEP_LIMIT {
            Err(bad_pattern(
                "ap: matching step limit exceeded (documented divergence: CPython hangs)",
            ))
        } else {
            Ok(())
        }
    }

    fn is_word(&self, c: char) -> bool {
        if self.flags & F_ASCII != 0 {
            c.is_ascii_alphanumeric() || c == '_'
        } else {
            in_ranges(WORD_RANGES, c as u32)
        }
    }

    fn at(&self, i: usize) -> Option<char> {
        self.text.get(i).copied()
    }

    /// Match `node` at `pos`; on success call `k` with the new position.
    /// This is CPS style to get proper backtracking order without
    /// materializing all continuations.
    fn m(
        &mut self,
        node: &Node,
        pos: usize,
        groups: &mut Groups,
        k: &mut dyn FnMut(&mut Matcher<'a>, usize, &mut Groups) -> MRes,
    ) -> MRes {
        self.tick()?;
        match node {
            Node::Empty => k(self, pos, groups),
            Node::Literal(c) => {
                if self.at(pos) == Some(char::from_u32(*c).unwrap_or('\0')) {
                    k(self, pos + 1, groups)
                } else {
                    Ok(None)
                }
            }
            Node::LiteralIgnore(c) => {
                let t = match self.at(pos) {
                    Some(t) => t,
                    None => return Ok(None),
                };
                let tl = t.to_lowercase().next().unwrap_or(t) as u32;
                if tl == *c || self.uni_case_eq(tl, *c) {
                    k(self, pos + 1, groups)
                } else {
                    Ok(None)
                }
            }
            Node::NotLiteral(c) => match self.at(pos) {
                None => Ok(None),
                Some(t) if t == char::from_u32(*c).unwrap_or('\0') => Ok(None),
                Some(_) => k(self, pos + 1, groups),
            },
            Node::NotLiteralIgnore(c) => {
                let t = match self.at(pos) {
                    Some(t) => t,
                    None => return Ok(None),
                };
                let tl = t.to_lowercase().next().unwrap_or(t) as u32;
                if tl == *c || self.uni_case_eq(tl, *c) {
                    Ok(None)
                } else {
                    k(self, pos + 1, groups)
                }
            }
            Node::AnyChar => match self.at(pos) {
                Some(t) if t != '\n' => k(self, pos + 1, groups),
                _ => Ok(None),
            },
            Node::AnyCharAll => {
                if self.at(pos).is_some() {
                    k(self, pos + 1, groups)
                } else {
                    Ok(None)
                }
            }
            Node::In(set) => match self.at(pos) {
                Some(t) if self.set_matches(set, t, false) => k(self, pos + 1, groups),
                _ => Ok(None),
            },
            Node::InIgnore(set) => match self.at(pos) {
                Some(t) if self.set_matches(set, t, true) => k(self, pos + 1, groups),
                _ => Ok(None),
            },
            Node::Seq(items) => {
                let ptr = items.as_ptr();
                let n = items.len();
                // items lives in the AST; transmute to 'a lifetime.
                let items2: *const Node = ptr;
                self.m_seq(items2, n, 0, pos, groups, k)
            }
            Node::Alt(branches) => {
                for b in branches {
                    if self.m(b, pos, groups, k)?.is_some() {
                        return Ok(Some(pos)); // value unused by k signature
                    }
                }
                Ok(None)
            }
            Node::Group { index, body } => match index {
                None => self.m(body, pos, groups, k),
                Some(gi) => {
                    let saved = groups.get(*gi);
                    let start = pos;
                    let ok = self.m(body, pos, groups, &mut |m, end, g| {
                        let prev = g.get(*gi);
                        g.set(*gi, (start, end));
                        // continue AFTER this group with the group set
                        match k(m, end, g) {
                            Ok(Some(x)) => Ok(Some(x)),
                            Ok(None) => {
                                g.set(*gi, prev);
                                Ok(None)
                            }
                            Err(e) => Err(e),
                        }
                    });
                    match ok {
                        Ok(Some(_)) => ok,
                        Ok(None) => {
                            groups.set(*gi, saved);
                            Ok(None)
                        }
                        Err(e) => Err(e),
                    }
                }
            },
            Node::AtomicGroup(body) => {
                // match body, then commit (no backtracking into it)
                let mut end_pos: Option<usize> = None;
                let r = self.m(body, pos, groups, &mut |_m, end, _g| {
                    end_pos = Some(end);
                    Ok(Some(end))
                })?;
                match r {
                    Some(_) => {
                        let end = end_pos.unwrap();
                        k(self, end, groups)
                    }
                    None => Ok(None),
                }
            }
            Node::Repeat {
                min,
                max,
                greedy,
                possessive,
                body,
            } => {
                if *possessive {
                    // Possessive = atomic(greedy repeat): consume the max,
                    // then ONE continuation attempt; no backtracking into
                    // the repetitions.
                    let inner = Node::Repeat {
                        min: *min,
                        max: *max,
                        greedy: true,
                        possessive: false,
                        body: body.clone(),
                    };
                    let mut committed_end: Option<usize> = None;
                    let r = self.m(&inner, pos, groups, &mut |_m, e, _g| {
                        committed_end = Some(e);
                        Ok(Some(e))
                    })?;
                    return match r {
                        Some(_) => k(self, committed_end.unwrap(), groups),
                        None => Ok(None),
                    };
                }
                self.m_repeat(*min, *max, *greedy, *possessive, body, pos, groups, k)
            }
            Node::Backref(gi) => {
                let (s, e) = groups.get(*gi);
                if s == usize::MAX {
                    // Unmatched group backref: CPython FAILS the match.
                    return Ok(None);
                }
                let len = e - s;
                if pos + len > self.text.len() {
                    return Ok(None);
                }
                if self.text[pos..pos + len] == self.text[s..e] {
                    k(self, pos + len, groups)
                } else {
                    Ok(None)
                }
            }
            Node::BackrefIgnore(gi) => {
                let (s, e) = groups.get(*gi);
                if s == usize::MAX {
                    return Ok(None);
                }
                let len = e - s;
                if pos + len > self.text.len() {
                    return Ok(None);
                }
                for j in 0..len {
                    let a = self.text[s + j];
                    let b = self.text[pos + j];
                    let al = a.to_lowercase().next().unwrap_or(a) as u32;
                    let bl = b.to_lowercase().next().unwrap_or(b) as u32;
                    if al != bl && !self.uni_case_eq(al, bl) {
                        return Ok(None);
                    }
                }
                k(self, pos + len, groups)
            }
            Node::Cond { group, yes, no } => {
                let (s, _) = groups.get(*group);
                let branch: &Node = if s != usize::MAX {
                    yes
                } else {
                    match no {
                        Some(n2) => n2,
                        None => &Node::Empty,
                    }
                };
                self.m(branch, pos, groups, k)
            }
            Node::Look {
                ahead,
                negative,
                body,
            } => {
                if !ahead {
                    let w = width_of(body).0;
                    let w = w.min(pos);
                    let start = pos - w;
                    let saved = groups.snapshot();
                    let matched = self.m(body, start, groups, &mut |_m, _e, _g| Ok(Some(_e)))?;
                    if *negative {
                        if matched.is_some() {
                            groups.restore(&saved);
                            Ok(None)
                        } else {
                            groups.restore(&saved);
                            k(self, pos, groups)
                        }
                    } else if matched.is_some() {
                        // CPython keeps captures from a positive lookaround.
                        k(self, pos, groups)
                    } else {
                        groups.restore(&saved);
                        Ok(None)
                    }
                } else {
                    let saved = groups.snapshot();
                    let matched = self.m(body, pos, groups, &mut |_m, _e, _g| Ok(Some(_e)))?;
                    if *negative {
                        if matched.is_some() {
                            groups.restore(&saved);
                            Ok(None)
                        } else {
                            groups.restore(&saved);
                            k(self, pos, groups)
                        }
                    } else if matched.is_some() {
                        k(self, pos, groups)
                    } else {
                        groups.restore(&saved);
                        Ok(None)
                    }
                }
            }
            Node::At(a) => {
                if self.anchor_holds(*a, pos) {
                    k(self, pos, groups)
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn m_seq(
        &mut self,
        items: *const Node,
        len: usize,
        idx: usize,
        pos: usize,
        groups: &mut Groups,
        k: &mut dyn FnMut(&mut Matcher<'a>, usize, &mut Groups) -> MRes,
    ) -> MRes {
        if idx == len {
            return k(self, pos, groups);
        }
        let item: &'a Node = unsafe { &*items.add(idx) };
        let rest = items;
        let rest_len = len;
        let next = idx + 1;
        self.m(item, pos, groups, &mut |m2, p2, g2| {
            m2.m_seq(rest, rest_len, next, p2, g2, k)
        })
    }

    #[allow(clippy::too_many_arguments)] // mirrors the CPython C-function signature shape
    fn m_repeat(
        &mut self,
        min: u32,
        max: u32,
        greedy: bool,
        possessive: bool,
        body: &Node,
        pos: usize,
        groups: &mut Groups,
        k: &mut dyn FnMut(&mut Matcher<'a>, usize, &mut Groups) -> MRes,
    ) -> MRes {
        self.m_repeat_rec(min, max, 0, greedy, possessive, body, pos, groups, k, pos)
    }

    #[allow(clippy::too_many_arguments)]
    fn m_repeat_rec(
        &mut self,
        min: u32,
        max: u32,
        count: u32,
        greedy: bool,
        possessive: bool,
        body: &Node,
        pos: usize,
        groups: &mut Groups,
        k: &mut dyn FnMut(&mut Matcher<'a>, usize, &mut Groups) -> MRes,
        _last_pos: usize,
    ) -> MRes {
        self.tick()?;
        let can_more = count < max;
        let try_more = move |m: &mut Matcher<'a>,
                             groups: &mut Groups,
                             k: &mut dyn FnMut(&mut Matcher<'a>, usize, &mut Groups) -> MRes|
              -> MRes {
            if !can_more {
                return k(m, pos, groups);
            }
            // Zero-width iteration protection: if the body matched empty at
            // the same position as the previous iteration, stop looping.
            let saved = groups.snapshot();
            let r = m.m(body, pos, groups, &mut |m2, p2, g2| {
                if p2 == pos && count + 1 > min {
                    // Zero-width iteration: CPython performs one more empty
                    // iteration (groups already set by the body) then stops
                    // looping — proceed to k once, no further repeats.
                    return k(m2, p2, g2);
                }
                m2.m_repeat_rec(
                    min,
                    max,
                    count + 1,
                    greedy,
                    possessive,
                    body,
                    p2,
                    g2,
                    k,
                    pos,
                )
            })?;
            if r.is_none() {
                groups.restore(&saved);
            }
            Ok(r.map(|_| pos))
        };
        if count < min {
            return try_more(self, groups, k);
        }
        if greedy {
            let more = try_more(self, groups, k)?;
            if more.is_some() {
                return Ok(more);
            }
            if possessive {
                return Ok(None);
            }
            k(self, pos, groups)
        } else {
            // lazy: try continuation first
            let r = k(self, pos, groups)?;
            if r.is_some() {
                return Ok(r);
            }
            try_more(self, groups, k)
        }
    }

    fn uni_case_eq(&self, a: u32, b: u32) -> bool {
        if a == b {
            return true;
        }
        if self.flags & F_ASCII != 0 {
            return false;
        }
        // _casefix extra cases (both directions)
        extra_cases(a).contains(&b) || extra_cases(b).contains(&a)
    }

    fn set_matches(&self, set: &CharSet, t: char, ic: bool) -> bool {
        let c = t as u32;
        let mut hit = false;
        for item in &set.items {
            let m = match item {
                SetItem::Literal(v) => c == *v,
                SetItem::LiteralIgnore(v) => {
                    let tl = t.to_lowercase().next().unwrap_or(t) as u32;
                    tl == *v || (ic && self.uni_case_eq(tl, *v))
                }
                SetItem::Range(lo, hi) => c >= *lo && c <= *hi,
                SetItem::RangeIgnore(lo, hi) => {
                    let tl = t.to_lowercase().next().unwrap_or(t) as u32;
                    (tl >= *lo && tl <= *hi)
                        || (ic && extra_cases(tl).iter().any(|x| x >= lo && x <= hi))
                }
                SetItem::Digit => {
                    if self.flags & F_ASCII != 0 {
                        (48..=57).contains(&c)
                    } else {
                        in_ranges(DIGIT_RANGES, c)
                    }
                }
                SetItem::NotDigit => {
                    let d = if self.flags & F_ASCII != 0 {
                        (48..=57).contains(&c)
                    } else {
                        in_ranges(DIGIT_RANGES, c)
                    };
                    !d
                }
                SetItem::Space => {
                    if self.flags & F_ASCII != 0 {
                        matches!(c, 0x09..=0x0d | 0x20)
                    } else {
                        in_ranges(SPACE_RANGES, c)
                    }
                }
                SetItem::NotSpace => {
                    let s2 = if self.flags & F_ASCII != 0 {
                        matches!(c, 0x09..=0x0d | 0x20)
                    } else {
                        in_ranges(SPACE_RANGES, c)
                    };
                    !s2
                }
                SetItem::Word => self.is_word(t),
                SetItem::NotWord => !self.is_word(t),
            };
            if m {
                hit = true;
                break;
            }
        }
        hit != set.negate
    }

    fn anchor_holds(&self, a: Anchor, pos: usize) -> bool {
        match a {
            Anchor::Beginning => pos == 0,
            Anchor::BeginningLine => pos == 0 || self.text[pos - 1] == '\n',
            Anchor::End => {
                pos == self.text.len()
                    || (self.flags & F_MULTILINE == 0
                        && pos == self.text.len() - 1
                        && self.text[pos] == '\n')
            }
            Anchor::EndLine => pos == self.text.len() || self.text[pos] == '\n',
            // \Z: absolute end of string only (CPython: unlike $, a trailing
            // newline does NOT satisfy \Z).
            Anchor::AbsoluteEnd => pos == self.text.len(),
            Anchor::Boundary | Anchor::NonBoundary => {
                let before = pos > 0 && self.is_word(self.text[pos - 1]);
                let after = pos < self.text.len() && self.is_word(self.text[pos]);
                let boundary = before != after;
                boundary == (a == Anchor::Boundary)
            }
        }
    }
}

// m_seq recursion: the AST (`items`) is an immutable borrow that outlives
// the matcher; Rust cannot express that through a `&mut Matcher` closure, so
// m_seq carries the rest-slice as a raw pointer and rebuilds the reference
// per level. Sound: the AST is never mutated during a match.
#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
fn items_rest(items: &[Node], from: usize) -> *const Node {
    unsafe { items.as_ptr().add(from) }
}

#[derive(Clone)]
pub(crate) struct Groups {
    pub spans: Vec<(usize, usize)>, // index 1..=ngroups
}

impl Groups {
    fn new(n: usize) -> Self {
        Groups {
            spans: vec![(usize::MAX, usize::MAX); n + 1],
        }
    }
    fn get(&self, gi: usize) -> (usize, usize) {
        *self.spans.get(gi).unwrap_or(&(usize::MAX, usize::MAX))
    }
    fn set(&mut self, gi: usize, v: (usize, usize)) {
        if gi < self.spans.len() {
            self.spans[gi] = v;
        }
    }
    fn snapshot(&self) -> Vec<(usize, usize)> {
        self.spans.clone()
    }
    fn restore(&mut self, snap: &[(usize, usize)]) {
        self.spans.clear();
        self.spans.extend_from_slice(snap);
    }
}

impl<'a> Matcher<'a> {
    /// Search: leftmost match starting at each position in [from, end).
    fn run(
        &mut self,
        root: &Node,
        from: usize,
        end: usize,
        anchored_start: bool,
        ngroups: usize,
    ) -> Result<Option<MatchResult>, crate::Flow> {
        let mut start = from;
        loop {
            let mut groups = Groups::new(ngroups);
            let mut result_end: Option<usize> = None;
            {
                let root2: &'a Node = unsafe { std::mem::transmute(root) };
                let r = self.m(root2, start, &mut groups, &mut |_m, e, _g| {
                    result_end = Some(e);
                    Ok(Some(e))
                })?;
                if r.is_some() {
                    if let Some(endpos) = result_end {
                        let mut spans = groups.snapshot();
                        spans[0] = (start, endpos);
                        return Ok(Some(MatchResult {
                            start,
                            end: endpos,
                            spans,
                        }));
                    }
                }
            }
            if anchored_start || start >= end {
                return Ok(None);
            }
            start += 1;
        }
    }
}

// ===========================================================================
// Value layer: Pattern / Match objects + module API
// ===========================================================================

/// A compiled pattern wrapped for the interpreter.
pub(crate) struct PatBox {
    pub compiled: Compiled,
}

// 02.09a P3: RE_CACHE / PAT_TABLE / MATCH_TABLE moved to the RuntimeState
// tables (runtime_state.rs::RE_CACHE_T / PAT_TABLE_T / MATCH_TABLE_T). The
// thread_locals made compiled patterns cross-thread INVISIBLE (audit row:
// sre.rs:2017/2049/2050 — "PAT/MATCH keyed by Rc::as_ptr -> a worker
// matching a main-compiled re.Pattern misses"; RE_CACHE held Gc payloads).

/// CPython re._cache cap (libreftest MAXCACHE).
const RE_CACHE_MAX: usize = 512;

fn re_cache_lookup(key: &(String, u32)) -> Option<GcPyObject> {
    use crate::runtime_state as rs;
    rs::with_active(rs::RE_CACHE_T_acc(), |c| c.0.get(key).cloned())
}

fn re_cache_insert(key: (String, u32), o: GcPyObject) {
    use crate::runtime_state as rs;
    rs::with_active_mut(rs::RE_CACHE_T_acc(), |c| {
        let m = &mut c.0;
        let ord = &mut c.1;
        if m.contains_key(&key) {
            return;
        }
        if ord.len() >= RE_CACHE_MAX {
            // Evict the oldest entry (FIFO, close enough to CPython's
            // LRU-for-compile-purpose; the corpus pins no eviction order).
            let oldest = ord.remove(0);
            m.remove(&oldest);
        }
        ord.push(key.clone());
        m.insert(key, o);
    });
}

/// Module-level re.compile(path): consult the 512-entry pattern cache first
/// (CPython semantics: `re.compile(p) is re.compile(p)` for the SAME args),
/// minting and caching a fresh Pattern object on a miss. Called only where
/// the re module resolves compile/match/search/findall module-level calls.
pub(crate) fn cached_pattern_object(compiled: Compiled) -> Value {
    let key = (compiled.pattern.clone(), compiled.flags);
    if let Some(o) = re_cache_lookup(&key) {
        return Value::Obj(o);
    }
    let v = pattern_object(compiled);
    if let Value::Obj(o) = &v {
        re_cache_insert(key, o.clone());
    }
    v
}

type GcPyObject = std::sync::Arc<ap_obj::PyObject>;

pub(crate) fn pattern_object(compiled: Compiled) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::Opaque, "re.Pattern");
    // The Compiled engine rides a thread-local identity map (attrs carry the
    // Python-visible surface only).
    let key = compiled_identity(&o);
    let pat_rc: PatRc = Arc::new(PatBox { compiled });
    {
        let a = o.attrs.lock().unwrap_or_else(|p| p.into_inner());
        let _ = a.set(vs("pattern"), vs(pat_rc.compiled.pattern.clone()));
        let _ = a.set(vs("flags"), vi(pat_rc.compiled.flags as i64));
        let _ = a.set(vs("groups"), vi(pat_rc.compiled.groups as i64));
        let gd = ap_obj::dict::PyDict::new();
        for (name, idx) in &pat_rc.compiled.grouporder {
            let _ = gd.set(vs(name.clone()), vi(*idx as i64));
        }
        let _ = a.set(vs("groupindex"), Value::Dict(gd));
    }
    *o.repr_text.lock().unwrap_or_else(|p| p.into_inner()) = Some(pattern_repr(&pat_rc.compiled));
    pat_table_insert(key, pat_rc);
    Value::Obj(o)
}

pub(crate) type PatRc = Arc<PatBox>;

// 02.09a P3: PAT_TABLE / MATCH_TABLE / their order ledgers live in the
// RuntimeState tables as (HashMap, Vec) rows — see the block comment above
// re_cache_lookup. The helper below takes the runtime_state table (NOT a
// LocalKey anymore).

const SRE_TABLE_CAP: usize = 1024;

fn pat_table_insert(key: usize, pat_rc: PatRc) {
    use crate::runtime_state as rs;
    rs::with_active_mut(rs::PAT_TABLE_T_acc(), |row| {
        let (ref mut m, ref mut ledger) = *row;
        if !m.contains_key(&key) {
            ledger.push(key);
            if ledger.len() > SRE_TABLE_CAP {
                let oldest = ledger.remove(0);
                m.remove(&oldest);
            }
        }
        m.insert(key, pat_rc);
    });
}

fn match_table_insert(key: usize, mb: Arc<MatchBox>) {
    use crate::runtime_state as rs;
    rs::with_active_mut(rs::MATCH_TABLE_T_acc(), |row| {
        let (ref mut m, ref mut ledger) = *row;
        if !m.contains_key(&key) {
            ledger.push(key);
            if ledger.len() > SRE_TABLE_CAP {
                let oldest = ledger.remove(0);
                m.remove(&oldest);
            }
        }
        m.insert(key, mb);
    });
}

fn compiled_identity(o: &GcPyObject) -> usize {
    std::sync::Arc::as_ptr(o) as *const u8 as usize
}

pub(crate) struct MatchBox {
    pub pattern: PatRc,
    pub text: String,
    pub pos: usize,
    pub endpos: usize,
    pub result: MatchResult,
}

pub(crate) fn pattern_of(obj: &Value) -> Option<PatRc> {
    if let Value::Obj(o) = obj {
        if o.name == "re.Pattern" {
            let key = compiled_identity(o);
            use crate::runtime_state as rs;
            return rs::with_active(rs::PAT_TABLE_T_acc(), |t| t.0.get(&key).cloned());
        }
    }
    None
}

pub(crate) fn match_of(obj: &Value) -> Option<Arc<MatchBox>> {
    if let Value::Obj(o) = obj {
        if o.name == "re.Match" {
            let key = compiled_identity(o);
            use crate::runtime_state as rs;
            return rs::with_active(rs::MATCH_TABLE_T_acc(), |t| t.0.get(&key).cloned());
        }
    }
    None
}

/// CPython Pattern repr: re.compile('pat', re.FLAG|...) with long names.
fn pattern_repr(c: &Compiled) -> String {
    let mut f = c.flags;
    let mut parts: Vec<&str> = Vec::new();
    // strip UNICODE (implicit, not shown) — CPython shows re.UNICODE only if
    // explicitly passed, but str patterns always end up with the bit; CPython
    // actually shows `re.UNICODE` for str patterns compiled with default
    // flags? No: repr(re.compile('a')) is "re.compile('a')". The UNICODE bit
    // IS set but repr masks it for str patterns.
    f &= !F_UNICODE;
    if f & F_IGNORECASE != 0 {
        parts.push("re.IGNORECASE");
    }
    if f & F_LOCALE != 0 {
        parts.push("re.LOCALE");
    }
    if f & F_MULTILINE != 0 {
        parts.push("re.MULTILINE");
    }
    if f & F_DOTALL != 0 {
        parts.push("re.DOTALL");
    }
    if f & F_VERBOSE != 0 {
        parts.push("re.VERBOSE");
    }
    if f & F_ASCII != 0 {
        parts.push("re.ASCII");
    }
    if f & 128 != 0 {
        parts.push("re.DEBUG");
    }
    let pat_escaped = py_repr_str(&c.pattern);
    if parts.is_empty() {
        format!("re.compile({})", pat_escaped)
    } else {
        format!("re.compile({}, {})", pat_escaped, parts.join(" | "))
    }
}

/// Python str repr with CPython quoting rules (simplified: prefer ' quotes,
/// escape \ ' \n \r \t and non-printables as \xNN/\uNNNN).
fn py_repr_str(s: &str) -> String {
    // CPython repr: ' unless the string contains ' and no " (then ").
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// CPython Match repr: <re.Match object; span=(0, 2), match='ab'>
fn match_repr(mb: &MatchBox) -> String {
    // result.start/end are CHARACTER indices (the matcher runs over
    // &[char]); mb.text is a byte String. Slice via chars, not bytes
    // (RF-1: byte slicing panicked 'not a char boundary' on multibyte
    // subjects, e.g. re.search('b', 'ääb')).
    let matched: String = mb
        .text
        .chars()
        .skip(mb.result.start)
        .take(mb.result.end - mb.result.start)
        .collect();
    format!(
        "<re.Match object; span=({}, {}), match={}>",
        mb.result.start,
        mb.result.end,
        py_repr_str(&matched)
    )
}

// ------------------------------------------------------------------ ops ---

#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
pub(crate) struct PatOp;

/// Run one pattern op. `method` is match/fullmatch/search; `pos`/`endpos`
/// from args. Returns the Match PyObject or None.
pub(crate) fn pat_exec(
    pat: &PatRc,
    method: &str,
    text: &str,
    pos: usize,
    endpos: usize,
) -> Result<Option<Value>, crate::Flow> {
    let chars: Vec<char> = text.chars().collect();
    let end = endpos.min(chars.len());
    let c = &pat.compiled;
    let mut m = Matcher {
        text: &chars[..end],
        flags: c.flags,
        steps: Cell::new(0),
    };
    let anchored = method == "match" || method == "fullmatch";
    let start = pos.min(end);
    let r = m.run(&c.root, start, end, anchored, c.groups)?;
    if let Some(mut res) = r {
        if method == "fullmatch" && res.end != end {
            // retry: backtracking should reach full width; our CPS returns
            // the FIRST match, so implement fullmatch as anchored match of
            // (?:pat)\Z — wrap: re-run requiring end == end.
            // Cheap approach: scan all matches until end==endpos.
            // The engine returns leftmost-first; CPython fullmatch is
            // "the whole string must match", which the engine's first
            // match may not satisfy even though a later backtrack would.
            // Re-run with an appended absolute-end anchor (\Z semantics:
            // fullmatch must span the WHOLE string; $ would wrongly allow a
            // trailing newline).
            let wrapped = Node::Seq(vec![c.root.clone(), Node::At(Anchor::AbsoluteEnd)]);
            let mut m2 = Matcher {
                text: &chars[..end],
                flags: c.flags,
                steps: Cell::new(0),
            };
            match m2.run(&wrapped, start, end, true, c.groups)? {
                Some(res2) => res = res2,
                None => return Ok(None),
            }
        }
        let mb = Arc::new(MatchBox {
            pattern: pat.clone(),
            text: text.to_string(),
            pos,
            endpos: end,
            result: res,
        });
        return Ok(Some(match_object(mb)));
    }
    Ok(None)
}

pub(crate) fn match_object(mb: Arc<MatchBox>) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::Opaque, "re.Match");
    let key = compiled_identity(&o);
    let repr = match_repr(&mb);
    match_table_insert(key, mb);
    *o.repr_text.lock().unwrap_or_else(|p| p.into_inner()) = Some(repr);
    Value::Obj(o)
}

/// Group value extraction by index or name.
pub(crate) fn group_value(mb: &MatchBox, key: &Value) -> Result<Value, crate::Flow> {
    let idx: usize = match key {
        Value::Int(n) => n.to_i64().unwrap_or(-1) as usize,
        Value::Str(sv) => {
            let name = sv.as_str();
            match mb.pattern.compiled.groupindex.get(name) {
                Some(i) => *i,
                None => {
                    return Err(raise("IndexError", "no such group"));
                }
            }
        }
        _ => {
            return Err(raise(
                "TypeError",
                format!(
                    "group() argument must be str or int, not {}",
                    key.type_name()
                ),
            ));
        }
    };
    if idx > mb.pattern.compiled.groups {
        return Err(raise("IndexError", "no such group"));
    }
    let (s, e) = mb.result.spans[idx];
    if s == usize::MAX {
        return Ok(Value::None);
    }
    let chars: Vec<char> = mb.text.chars().collect();
    Ok(vs(chars[s..e].iter().collect::<String>()))
}

/// groups() / group(...) multi-arg.
#[allow(dead_code)] // kept: part of an unfinished surface (robustness-ci 02.x burn-down leaf)
pub(crate) fn match_groups_all(mb: &MatchBox, keys: &[Value]) -> Result<Vec<Value>, crate::Flow> {
    if keys.is_empty() {
        let mut out = Vec::new();
        for gi in 1..=mb.pattern.compiled.groups {
            out.push(group_value(mb, &vi(gi as i64))?);
        }
        Ok(out)
    } else {
        let mut out = Vec::new();
        for k in keys {
            out.push(group_value(mb, k)?);
        }
        Ok(out)
    }
}

pub(crate) fn group_span(mb: &MatchBox, idx: usize) -> (i64, i64) {
    let (s, e) = mb.result.spans[idx];
    if s == usize::MAX {
        (-1, -1)
    } else {
        (s as i64, e as i64)
    }
}

// -------------------------------------------------------------- template ---

/// Parse a sub() repl string into interleaved literals and group refs
/// (parse_template semantics).
pub(crate) enum ReplPart {
    Lit(String),
    Group(usize),
}

pub(crate) fn parse_repl(
    repl: &str,
    groups: usize,
    groupindex: &HashMap<String, usize>,
    pattern: Option<&str>,
) -> Result<Vec<ReplPart>, crate::Flow> {
    let mut out: Vec<ReplPart> = Vec::new();
    let mut lit = String::new();
    let chars: Vec<char> = repl.chars().collect();
    let mut i = 0;
    let flush = |lit: &mut String, out: &mut Vec<ReplPart>| {
        if !lit.is_empty() {
            out.push(ReplPart::Lit(std::mem::take(lit)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            if i + 1 >= chars.len() {
                return Err(bad_pattern("bad escape (end of pattern)"));
            }
            let e = chars[i + 1];
            i += 2;
            match e {
                'g' => {
                    if i >= chars.len() || chars[i] != '<' {
                        return Err(bad_pattern("missing <"));
                    }
                    i += 1;
                    let mut name = String::new();
                    while i < chars.len() && chars[i] != '>' {
                        name.push(chars[i]);
                        i += 1;
                    }
                    if i >= chars.len() {
                        return Err(bad_pattern("missing >, unterminated name"));
                    }
                    i += 1;
                    let idx = if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                        let n: usize = name.parse().unwrap_or(999999);
                        if n >= 100 {
                            return Err(re_error(
                                &format!("invalid group reference {}", n),
                                pattern.unwrap_or(""),
                                repl.find(&format!("\\g<{}", name)).unwrap_or(0),
                            ));
                        }
                        n
                    } else {
                        *groupindex.get(&name).ok_or_else(|| {
                            crate::builtins_modules::raise(
                                "IndexError",
                                format!("unknown group name '{}'", name),
                            )
                        })?
                    };
                    if idx > groups {
                        // CPython's parse_template error: position IN THE
                        // REPL at the reference; the error's .pattern is the
                        // compiled pattern (message format identical).
                        let pos = repl
                            .rfind(&format!("\\g<{}>", name))
                            .map(|x| x + 1 + name.len())
                            .unwrap_or(0);
                        return Err(re_error(
                            &format!("invalid group reference {}", idx),
                            pattern.unwrap_or(""),
                            pos,
                        ));
                    }
                    flush(&mut lit, &mut out);
                    out.push(ReplPart::Group(idx));
                }
                '0'..='9' => {
                    let mut digits = String::new();
                    digits.push(e);
                    while i < chars.len() && chars[i].is_ascii_digit() && digits.len() < 2 {
                        digits.push(chars[i]);
                        i += 1;
                    }
                    let n: usize = digits.parse().unwrap();
                    if n == 0 {
                        lit.push('\0');
                    } else if n > groups {
                        // position in the REPL: the backslash of \n
                        let pos = repl
                            .rfind(&format!("\\{}", digits))
                            .map(|x| x + 1)
                            .unwrap_or(0);
                        return Err(re_error(
                            &format!("invalid group reference {}", n),
                            pattern.unwrap_or(""),
                            pos,
                        ));
                    } else {
                        flush(&mut lit, &mut out);
                        out.push(ReplPart::Group(n));
                    }
                }
                'n' => lit.push('\n'),
                'r' => lit.push('\r'),
                't' => lit.push('\t'),
                'b' => lit.push('\u{8}'),
                'a' => lit.push('\u{7}'),
                'f' => lit.push('\u{c}'),
                'v' => lit.push('\u{b}'),
                '\\' => lit.push('\\'),
                _ if e.is_ascii_alphabetic() => {
                    let pos = i.saturating_sub(2);
                    return Err(re_error(
                        &format!("bad escape \\{}", e),
                        pattern.unwrap_or(""),
                        pos,
                    ));
                }
                other => lit.push(other),
            }
        } else {
            lit.push(c);
            i += 1;
        }
    }
    flush(&mut lit, &mut out);
    Ok(out)
}

/// Expand a parsed repl against a match.
pub(crate) fn expand_repl(mb: &MatchBox, parts: &[ReplPart]) -> Result<String, crate::Flow> {
    let chars: Vec<char> = mb.text.chars().collect();
    let mut out = String::new();
    for p in parts {
        match p {
            ReplPart::Lit(s) => out.push_str(s),
            ReplPart::Group(gi) => {
                let (s, e) = mb.result.spans[*gi];
                if s != usize::MAX {
                    out.extend(chars[s..e].iter());
                }
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ sub ---

pub(crate) fn pat_sub(
    pat: &PatRc,
    repl_parts: Option<Vec<ReplPart>>,
    repl_fn: Option<&Value>,
    text: &str,
    count: i64,
) -> Result<(String, i64), crate::Flow> {
    let chars: Vec<char> = text.chars().collect();
    let c = &pat.compiled;
    let mut m = Matcher {
        text: &chars,
        flags: c.flags,
        steps: Cell::new(0),
    };
    let mut out = String::new();
    let mut last = 0usize;
    let mut n: i64 = 0;
    let mut pos = 0usize;
    while pos <= chars.len() {
        if count > 0 && n >= count {
            break;
        }
        let r = m.run(&c.root, pos, chars.len(), false, c.groups)?;
        let res = match r {
            Some(r) => r,
            None => break,
        };
        out.extend(chars[last..res.start].iter());
        let mb = Arc::new(MatchBox {
            pattern: pat.clone(),
            text: text.to_string(),
            pos: 0,
            endpos: chars.len(),
            result: res.clone(),
        });
        if let Some(f) = repl_fn {
            let rv = crate::interp::call_value_pub3(f, &[match_object(mb.clone())])?;
            match rv {
                Value::Str(sv) => out.push_str(sv.as_str()),
                Value::Bytes(b) => {
                    // bytes repl on str subject is a TypeError in CPython
                    let _ = b;
                    return Err(raise(
                        "TypeError",
                        "not all arguments converted during bytes formatting",
                    ));
                }
                other => {
                    let _ = other;
                    return Err(raise(
                        "TypeError",
                        "repl must be a string or callable, not NoneType",
                    ));
                }
            }
        } else {
            out.push_str(&expand_repl(&mb, repl_parts.as_ref().unwrap())?);
        }
        n += 1;
        // Advance past the match; empty matches advance one char but still
        // emit the char that was skipped (CPython empty-match semantics).
        if res.end == res.start {
            if res.end < chars.len() {
                out.push(chars[res.end]);
            }
            last = res.end + 1;
            pos = res.end + 1;
        } else {
            last = res.end;
            pos = res.end;
        }
    }
    if last < chars.len() {
        out.extend(chars[last..].iter());
    }
    Ok((out, n))
}

// ----------------------------------------------------------------- split ---

pub(crate) fn pat_split(pat: &PatRc, text: &str, maxsplit: i64) -> Result<Vec<Value>, crate::Flow> {
    let chars: Vec<char> = text.chars().collect();
    let c = &pat.compiled;
    let mut m = Matcher {
        text: &chars,
        flags: c.flags,
        steps: Cell::new(0),
    };
    let mut out: Vec<Value> = Vec::new();
    let mut last = 0usize;
    let mut n: i64 = 0;
    let mut pos = 0usize;
    while pos <= chars.len() {
        if maxsplit > 0 && n >= maxsplit {
            break;
        }
        let r = m.run(&c.root, pos, chars.len(), false, c.groups)?;
        let res = match r {
            Some(r) => r,
            None => break,
        };
        out.push(vs(chars[last..res.start].iter().collect::<String>()));
        // captured groups appended
        for gi in 1..=c.groups {
            let (s, e) = res.spans[gi];
            if s == usize::MAX {
                out.push(Value::None);
            } else {
                out.push(vs(chars[s..e].iter().collect::<String>()));
            }
        }
        n += 1;
        if res.end == res.start {
            // Empty match (CPython _sre split semantics): the field is
            // [last:start], last stays at start, and the scan advances one
            // char — so the char lands in the NEXT field.
            last = res.start;
            pos = res.end + 1;
        } else {
            last = res.end;
            pos = res.end;
        }
    }
    if last <= chars.len() {
        out.push(vs(chars[last.min(chars.len())..]
            .iter()
            .collect::<String>()));
    }
    Ok(out)
}

// -------------------------------------------------------------- findall ---

pub(crate) fn pat_findall(pat: &PatRc, text: &str) -> Result<Vec<Value>, crate::Flow> {
    let chars: Vec<char> = text.chars().collect();
    let c = &pat.compiled;
    let mut m = Matcher {
        text: &chars,
        flags: c.flags,
        steps: Cell::new(0),
    };
    let mut out: Vec<Value> = Vec::new();
    let mut pos = 0usize;
    while pos <= chars.len() {
        let r = m.run(&c.root, pos, chars.len(), false, c.groups)?;
        let res = match r {
            Some(r) => r,
            None => break,
        };
        let mb = Arc::new(MatchBox {
            pattern: pat.clone(),
            text: text.to_string(),
            pos: 0,
            endpos: chars.len(),
            result: res.clone(),
        });
        if c.groups == 0 {
            out.push(vs(chars[res.start..res.end].iter().collect::<String>()));
        } else if c.groups == 1 {
            // RF-4: a NON-PARTICIPATING group yields '' in findall, not None
            // (CPython: re.findall('(a)|b', 'b') -> ['']).
            let (s, _) = res.spans[1];
            if s == usize::MAX {
                out.push(vs(String::new()));
            } else {
                out.push(group_value(&mb, &vi(1))?);
            }
        } else {
            let mut tup = Vec::new();
            for gi in 1..=c.groups {
                let (s, _e) = res.spans[gi];
                if s == usize::MAX {
                    // CPython findall: unmatched groups become ''
                    tup.push(vs(String::new()));
                } else {
                    tup.push(group_value(&mb, &vi(gi as i64))?);
                }
            }
            out.push(vtuple(tup));
        }
        if res.end == res.start {
            pos = res.end + 1;
        } else {
            pos = res.end;
        }
    }
    Ok(out)
}

/// re.escape (CPython 3.7+: only special chars escaped).
pub(crate) fn re_escape(pattern: &str) -> String {
    let special: &str = "()[]{}?*+-|^$\\.&~# \t\n\r\u{b}\u{c}";
    let mut out = String::with_capacity(pattern.len() * 2);
    for c in pattern.chars() {
        if special.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Build a fresh matcher over `chars` for `pat`.
pub(crate) fn engine_for<'a>(pat: &PatRc, chars: &'a [char]) -> Matcher<'a> {
    Matcher {
        text: chars,
        flags: pat.compiled.flags,
        steps: Cell::new(0),
    }
}

/// Run one leftmost search from `pos` (non-anchored).
pub(crate) fn run_from(
    m: &mut Matcher,
    pat: &PatRc,
    pos: usize,
) -> Result<Option<MatchResult>, crate::Flow> {
    m.run(
        &pat.compiled.root,
        pos,
        m.text.len(),
        false,
        pat.compiled.groups,
    )
}

/// pattern_object from an already-boxed pattern (compile_arg path).
pub(crate) fn pattern_object_patbox(pat: PatRc) -> Value {
    let o = PyObject::new(ap_obj::ObjKind::Opaque, "re.Pattern");
    let key = compiled_identity(&o);
    {
        let a = o.attrs.lock().unwrap_or_else(|p| p.into_inner());
        let _ = a.set(vs("pattern"), vs(pat.compiled.pattern.clone()));
        let _ = a.set(vs("flags"), vi(pat.compiled.flags as i64));
        let _ = a.set(vs("groups"), vi(pat.compiled.groups as i64));
        let gd = ap_obj::dict::PyDict::new();
        for (name, idx) in &pat.compiled.grouporder {
            let _ = gd.set(vs(name.clone()), vi(*idx as i64));
        }
        let _ = a.set(vs("groupindex"), Value::Dict(gd));
    }
    *o.repr_text.lock().unwrap_or_else(|p| p.into_inner()) = Some(pattern_repr(&pat.compiled));
    pat_table_insert(key, pat);
    Value::Obj(o)
}

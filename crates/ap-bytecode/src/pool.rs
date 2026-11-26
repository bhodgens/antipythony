//! Deduplicated constant pool.
//!
//! Entries: small ints (i64), big ints (decimal digits, parsed via
//! `ap_obj::Int::parse_base10` at decode/intern time — spec: "big ints via
//! ap-obj::Int::parse at decode time, interned"), floats (bitwise-keyed so
//! `0.0` and `-0.0` stay distinct), strings, bytes.
//!
//! Dedup key: a canonical enum identity per entry. Float dedup keys on the
//! bit pattern (NaN canonicalized to one quiet bit pattern so two NaN
//! literals share a slot — source-level floats are never signaling NaN).

use std::collections::HashMap;

/// One pooled constant.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    /// Small integer (fits i64 at write time).
    Int(i64),
    /// Arbitrary-precision integer, decimal digits as written (may exceed
    /// i128). Interned to `ap_obj::Int` on decode.
    BigInt(String),
    /// IEEE-754 binary64.
    Float(f64),
    /// String literal (already unescaped/joined by the frontend).
    Str(String),
    /// Bytes literal (already unescaped/joined).
    Bytes(Vec<u8>),
}

/// Canonical dedup key (Hash+Eq-safe; floats keyed by bits).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConstKey {
    Int(i64),
    BigInt(String),
    FloatBits(u64),
    Str(String),
    Bytes(Vec<u8>),
}

fn key_of(c: &Const) -> ConstKey {
    match c {
        Const::Int(i) => ConstKey::Int(*i),
        Const::BigInt(s) => ConstKey::BigInt(s.clone()),
        // Canonicalize NaN to a single bit pattern so identical NaN
        // literals dedupe; -0.0 keeps its bit (0.0 != -0.0 by sign).
        Const::Float(f) => {
            let bits = if f.is_nan() {
                0x7ff8_0000_0000_0000
            } else {
                f.to_bits()
            };
            ConstKey::FloatBits(bits)
        }
        Const::Str(s) => ConstKey::Str(s.clone()),
        Const::Bytes(b) => ConstKey::Bytes(b.clone()),
    }
}

/// The pool: insertion-ordered, deduplicated.
#[derive(Debug, Clone, Default)]
pub struct ConstPool {
    entries: Vec<Const>,
    index: HashMap<ConstKey, u32>,
}

impl ConstPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `c`; returns its index. Dedup: an equal constant returns the
    /// existing index.
    pub fn intern(&mut self, c: Const) -> u32 {
        let key = key_of(&c);
        if let Some(&idx) = self.index.get(&key) {
            return idx;
        }
        let idx = self.entries.len() as u32;
        self.entries.push(c);
        self.index.insert(key, idx);
        idx
    }

    /// Number of deduplicated entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, idx: u32) -> &Const {
        &self.entries[idx as usize]
    }

    pub fn iter(&self) -> impl Iterator<Item = &Const> {
        self.entries.iter()
    }

    /// Decode entry `idx` to its textual form (disassembly/tests).
    pub fn render(&self, idx: u32) -> String {
        match self.get(idx) {
            Const::Int(i) => i.to_string(),
            Const::BigInt(s) => s.clone(),
            Const::Float(f) => {
                if f.is_nan() {
                    "nan".to_string()
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        "inf".to_string()
                    } else {
                        "-inf".to_string()
                    }
                } else {
                    format!("{f:?}")
                }
            }
            Const::Str(s) => format!("{s:?}"),
            Const::Bytes(b) => format!("{b:?}"),
        }
    }
}

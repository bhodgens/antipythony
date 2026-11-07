//! Feedback substrate: one frozen-layout cell per feedback slot, adapted
//! from pon (github.com/can1357/pon, pon-runtime/src/feedback.rs).
//! Copyright Can Bölük, MIT/Apache-2.0; adapted for antipythony under the
//! granted dual license with attribution.
//!
//! Layout discipline (pon): one 32-byte-ish `FeedbackCell` per slot with
//! compile-time offset assertions; `packed` doubles as the type-feedback
//! state machine AND a seqlock for torn-read-free publication. Readers:
//! load packed (Acquire) → payload → re-check packed; mismatch ⇒ torn ⇒
//! slow path. Writers: CAS packed → IC_WRITING, store payload,
//! Release-publish. No locks, no allocation on the hot path.
//!
//! Slot assignment happens ONLY at the final lowering stage (clif.rs);
//! nothing in this module ever touches the structured ap-ir tree (leaf
//! Risk-5 mitigation). This module is the runtime substrate the lowered
//! code records into and tier-1 snapshots from.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// Type tags recorded by tier-0 helper calls (pon `TypeTag` analogue).
/// Ordered coarse→fine; tier-1 speculation admits only tags with an
/// unboxed repr (currently IntI64; float planned behind it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeTag {
    /// Unknown / megamorphic — no speculation admitted.
    Other = 0,
    Bool = 1,
    /// Python int that fits i64.
    IntI64 = 2,
    /// Python int that does not fit i64 (bigint).
    IntBig = 3,
    Float = 4,
    Str = 5,
}

impl TypeTag {
    /// Runtime classification of a boxed value (the only place this
    /// module inspects ap-obj values — trait-surface read only).
    pub fn of(v: &ap_obj::Value) -> TypeTag {
        match v {
            ap_obj::Value::None => TypeTag::Other,
            ap_obj::Value::Bool(_) => TypeTag::Bool,
            ap_obj::Value::Int(i) => {
                if i.to_i64().is_some() {
                    TypeTag::IntI64
                } else {
                    TypeTag::IntBig
                }
            }
            ap_obj::Value::Float(_) => TypeTag::Float,
            ap_obj::Value::Str(_) => TypeTag::Str,
            _ => TypeTag::Other,
        }
    }

    /// Whether tier-1 may unbox this tag (IntI64 fast path first; float
    /// planned behind it per the leaf).
    pub fn unboxable(self) -> bool {
        matches!(self, TypeTag::IntI64)
    }

    fn from_u8(v: u8) -> TypeTag {
        match v {
            1 => TypeTag::Bool,
            2 => TypeTag::IntI64,
            3 => TypeTag::IntBig,
            4 => TypeTag::Float,
            5 => TypeTag::Str,
            _ => TypeTag::Other,
        }
    }
}

/// Feedback state machine (pon `next_state` analogue):
/// EMPTY → MONO → POLY → MEGA with saturating counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FeedbackState {
    Empty = 0,
    Mono = 1,
    Poly = 2,
    Mega = 3,
}

/// Sentinel `packed` values (pon constants).
pub const IC_EMPTY: u32 = 0;
/// Writer-claim sentinel: a reader seeing this re-reads (torn window).
pub const IC_WRITING: u32 = u32::MAX;

/// Monomorphic observation limit before promotion to POLY (pon analogue;
/// tuned via env behind a knob, never a hardcoded hot-path constant).
pub fn mono_saturation() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("APX_IC_MONO_SATURATION")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(64)
    })
}

/// One frozen-layout feedback cell (pon `FeedbackCell` analogue).
///
/// `packed` (u32): when not IC_WRITING, encodes state (low 2 bits) and a
/// saturating observation count (next 8 bits); the high bits stay zero so
/// the seqlock recheck compares the whole word. NOT Copy/Clone: atomics.
#[repr(C)]
#[derive(Debug)]
pub struct FeedbackCell {
    /// Guard/state word + seqlock (see module doc).
    pub packed: std::sync::atomic::AtomicU32,
    /// Aux free word (reserved for AttrIC probe-mode bits).
    pub aux: std::sync::atomic::AtomicU32,
    /// Payload: tag pair `(tag_a, tag_b)` packed u64 for binop sites.
    pub payload: std::sync::atomic::AtomicU64,
    /// Identity: e.g. interned type-name hash for AttrIC sites.
    pub identity: std::sync::atomic::AtomicU64,
    /// Extra: version-guard word (type/dict version at cache time).
    pub extra: std::sync::atomic::AtomicU64,
}

const _CELL_SIZE_ASSERT: () = assert!(
    std::mem::size_of::<FeedbackCell>() == 32,
    "FeedbackCell layout frozen at 32 bytes (pon discipline)"
);

impl FeedbackCell {
    pub const fn new() -> Self {
        FeedbackCell {
            packed: std::sync::atomic::AtomicU32::new(IC_EMPTY),
            aux: std::sync::atomic::AtomicU32::new(0),
            payload: std::sync::atomic::AtomicU64::new(0),
            identity: std::sync::atomic::AtomicU64::new(0),
            extra: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Decode the state from a non-writing `packed` word.
    pub fn state_of(packed: u32) -> FeedbackState {
        match packed & 0b11 {
            0 => FeedbackState::Empty,
            1 => FeedbackState::Mono,
            2 => FeedbackState::Poly,
            _ => FeedbackState::Mega,
        }
    }

    fn encode(state: FeedbackState, count: u32) -> u32 {
        (state as u32) | (count.min(0xFF) << 2)
    }

    /// Tier-0 record path (pon `record` analogue): observe an operand tag
    /// pair at this site. Lock-free, allocation-free; a failed claim (a
    /// concurrent writer holds IC_WRITING) drops the observation — benign
    /// racy by design.
    pub fn record(&self, a: TypeTag, b: Option<TypeTag>) {
        use std::sync::atomic::Ordering;
        loop {
            let packed = self.packed.load(Ordering::Acquire);
            if packed == IC_WRITING {
                // Writer in flight: drop the observation (benign-racy).
                return;
            }
            let state = Self::state_of(packed);
            if state == FeedbackState::Mega {
                return; // saturated megamorphic: no further tracking
            }
            let count = packed >> 2;
            let new_packed = match state {
                FeedbackState::Empty => {
                    self.payload.store(pack_tags(a, b), Ordering::Relaxed);
                    Self::encode(FeedbackState::Mono, 1)
                }
                FeedbackState::Mono => {
                    let (had_b, pa, pb) = unpack_tags(self.payload.load(Ordering::Relaxed));
                    let same = had_b == b.is_some()
                        && pa == a
                        && (!had_b || pb == b.unwrap_or(TypeTag::Other));
                    if same {
                        Self::encode(FeedbackState::Mono, count + 1)
                    } else if count >= mono_saturation() {
                        // Saturated mono that saw a divergent tag: POLY.
                        Self::encode(FeedbackState::Poly, 1)
                    } else {
                        // Keep counting; promotion happens after saturation.
                        Self::encode(FeedbackState::Mono, count + 1)
                    }
                }
                FeedbackState::Poly => Self::encode(FeedbackState::Mega, 1),
                FeedbackState::Mega => unreachable!("checked above"),
            };
            match self.packed.compare_exchange(
                packed,
                new_packed,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(_) => continue, // lost race: retry once-around
            }
        }
    }

    /// Torn-read-free snapshot (seqlock protocol): returns `None` when a
    /// writer was in flight across the read (caller falls back to slow
    /// path / re-record).
    pub fn read(&self) -> Option<(FeedbackState, TypeTag, Option<TypeTag>)> {
        use std::sync::atomic::Ordering;
        let p1 = self.packed.load(Ordering::Acquire);
        if p1 == IC_WRITING {
            return None;
        }
        let payload = self.payload.load(Ordering::Acquire);
        let p2 = self.packed.load(Ordering::Acquire);
        if p1 != p2 {
            return None;
        }
        let (has_b, a, b) = unpack_tags(payload);
        Some((Self::state_of(p1), a, if has_b { Some(b) } else { None }))
    }

    /// Full snapshot for the tier-1 compiler (called on the ENQUEUEING
    /// thread only — the compiler never reads live cells concurrently,
    /// pon discipline).
    pub fn snapshot(&self) -> CellSnapshot {
        match self.read() {
            Some((FeedbackState::Mono, a, Some(b))) => CellSnapshot {
                state: FeedbackState::Mono,
                tags: Some((a, b)),
            },
            Some((state, _a, _b)) => CellSnapshot {
                // POLY/MEGA/Empty, or mono without a second tag: no
                // speculation admitted (conservative snapshot).
                state,
                tags: None,
            },
            None => CellSnapshot {
                state: FeedbackState::Empty,
                tags: None,
            },
        }
    }

    /// Debug-build audit cross-check hook (`APX_IC_AUDIT=1`): force the
    /// cell back to EMPTY so the slow path re-records; callers re-verify
    /// the cached result against the slow-path recompute.
    pub fn audit_reset(&self) {
        use std::sync::atomic::Ordering;
        self.packed.store(IC_EMPTY, Ordering::Release);
        self.payload.store(0, Ordering::Relaxed);
    }
}

impl Default for FeedbackCell {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot handed to the tier-1 compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSnapshot {
    pub state: FeedbackState,
    /// `(tag_a, tag_b)` speculation candidates; `None` = no speculation.
    pub tags: Option<(TypeTag, TypeTag)>,
}

fn pack_tags(a: TypeTag, b: Option<TypeTag>) -> u64 {
    let has_b = b.is_some() as u64;
    let bv = (b.unwrap_or(TypeTag::Other) as u32) as u64;
    (has_b << 62) | (bv << 32) | (a as u32 as u64)
}

fn unpack_tags(p: u64) -> (bool, TypeTag, TypeTag) {
    let has_b = (p >> 62) & 1 == 1;
    let b = TypeTag::from_u8(((p >> 32) & 0xFF) as u8);
    let a = TypeTag::from_u8((p & 0xFF) as u8);
    (has_b, a, b)
}

//! Typed inline-cache entries + the per-(thread, code) cache table
//! (leaf fast-path/02-perf/01 — the LoadGlobal/LoadAttr/LoadAttrCall
//! consult/fill bodies live in bytecode_interp.rs; THIS module is the
//! storage + validation layer).
//!
//! NO-GIL SAFETY SHAPE (spec item 4): the tables are PER-THREAD
//! (thread_local) observations — warm hits take NO lock, worker threads
//! never share entries, and there is no cross-thread invalidation problem
//! by construction. Soundness under mutation does NOT ride the storage:
//! every hit re-validates its stamps (02.05b version counters + dict
//! epochs) against the LIVE counters before trusting the payload. A stale
//! entry anywhere = a miss = the exact slow-path probes re-run = correct.
//!
//! CONTENTION PROFILE (documented per the leaf spec): the shape trades
//! the "shared cache with CAS/versioned entries" alternative for zero
//! shared-write traffic — warm reads are pure local loads (2 AtomicU64
//! relaxed loads + compares for globals; 4 for attrs), fills are private.
//! A sharded shared table would amortize memory across threads but adds
//! at least one shared atomic read-modify-write per fill and a shared
//! cacheline bounce per warm hit; at 4 threads the per-thread shape wins
//! on every warm step (see the 4-thread probe in ic_invalidation.rs).

use std::cell::RefCell;
use std::sync::Arc;

use ap_bytecode::caches::{AttrPlan, CallPlan, IC_MAGIC};
use ap_bytecode::sites::SiteKind;
use ap_obj::classes::TypeRef;
use ap_obj::int::Gc;
use ap_obj::value::Value;

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// LoadGlobal site entry (thread-local; one per GlobalLoad site id).
#[derive(Debug, Clone)]
pub struct GlobalIc {
    /// IC_MAGIC = filled.
    pub magic: u64,
    /// The frame's globals dict mutation epoch at fill time.
    pub globals_epoch: u64,
    /// The builtins dict mutation epoch at fill time.
    pub builtins_epoch: u64,
    /// The globals dict identity (Arc as ptr).
    pub globals_ptr: u64,
    /// The builtins dict identity.
    pub builtins_ptr: u64,
    /// ap_bytecode::caches::global_kind::POSITIVE = payload valid. (A
    /// NEGATIVE entry shape was considered and REJECTED: a dict-pair
    /// unmutated stamp does prove absence from both dicts, but the miss
    /// answer may have come from the T1-d module-globals / exc-class
    /// fallbacks which read OTHER surfaces — recording negatives would
    /// need a per-answer provenance walk on every fill. NameErrors stay
    /// fully dynamic; the win on hot loops is the positive reads.)
    pub kind: u8,
    /// The resolved Value. Stored DIRECTLY (not bit-packed): the Arc
    /// handles inside a stored Value keep their referents GC-reachable
    /// (the conservative stack floor pins entries through the
    /// thread-local table's Vec while it lives; a bit-packed copy would
    /// be INVISIBLE to the GC — the first entry of this leaf held bits
    /// and the GC collected a packed referent out from under a warm hit:
    /// use-after-free, seg4 repro. Direct storage is the fix and the
    /// rule: NEVER store Values anywhere as raw bits).
    pub v: Value,
}

impl Default for GlobalIc {
    fn default() -> Self {
        Self {
            magic: 0,
            globals_epoch: 0,
            builtins_epoch: 0,
            globals_ptr: 0,
            builtins_ptr: 0,
            kind: 0,
            v: Value::None,
        }
    }
}

impl GlobalIc {
    #[inline]
    pub fn is_hit(&self, g_epoch: u64, b_epoch: u64, g_ptr: u64, b_ptr: u64) -> Option<Value> {
        if self.magic == IC_MAGIC
            && self.kind == ap_bytecode::caches::global_kind::POSITIVE
            && self.globals_epoch == g_epoch
            && self.builtins_epoch == b_epoch
            && self.globals_ptr == g_ptr
            && self.builtins_ptr == b_ptr
        {
            Some(self.v.clone())
        } else {
            None
        }
    }
}

/// LoadAttr site entry (thread-local; one per AttrLoad site id).
#[derive(Debug, Clone)]
pub struct AttrIc {
    /// IC_MAGIC = filled.
    pub magic: u64,
    /// The dispatch plan (AttrPlan as u8; Slow entries are never stored —
    /// a Slow outcome just leaves the slot empty).
    pub plan: u8,
    /// The receiver type payload identity (Arc<TypeRef> as ptr) — the
    /// primary key: a different class on the same site must miss.
    pub type_ptr: u64,
    /// The type payload's OWN type_version at fill time.
    pub type_version: u64,
    /// chain_surgery_version() over the receiver's MRO at fill time (the
    /// subclass invalidator: an override added on any MRO entry bumps the
    /// chain max).
    pub chain_version: u64,
    /// The type payload's names-dict mutation epoch at fill time (catches
    /// direct names-dict writes that change HOW a name answers without a
    /// bump_class_versions call).
    pub names_epoch: u64,
    /// MRO-entry identity stamp: the Arc ptr of the defining class the
    /// fill resolved through. An override ADDED EARLIER in the MRO
    /// (shadowing) bumps chain_version; a base class PAYLOAD replaced
    /// (impossible today — write-once MRO) would change this ptr.
    pub owner_ptr: u64,
    /// The warm answer payload, stored DIRECTLY (GC-visible Arc handles —
    /// same rule as GlobalIc.v; bit-packing Values hides them from the
    /// collector and use-after-frees on warm hits):
    /// - InstDict: the fill-time instance value (UNUSED on hits — the hit
    ///   re-probes the live attrs dict; kept only so the classification is
    ///   inspectable) — stored as None to avoid pinning a stale value.
    /// - BindFunc: the plain function Value the MRO supplied (minted into
    ///   a fresh BoundMethod per hit).
    /// - ClassValue: the member Value (the warm answer itself).
    pub v: Value,
}

impl Default for AttrIc {
    fn default() -> Self {
        Self {
            magic: 0,
            plan: 0,
            type_ptr: 0,
            type_version: 0,
            chain_version: 0,
            names_epoch: 0,
            owner_ptr: 0,
            v: Value::None,
        }
    }
}

impl AttrIc {
    /// The class-stamp validity check shared by every plan.
    #[inline]
    pub fn stamps_valid(
        &self,
        type_ptr: u64,
        type_version: u64,
        chain_version: u64,
        names_epoch: u64,
    ) -> bool {
        self.magic == IC_MAGIC
            && self.type_ptr == type_ptr
            && self.type_version == type_version
            && self.chain_version == chain_version
            && self.names_epoch == names_epoch
    }
}

/// LoadAttrCall / CallFunction site entry (thread-local).
#[derive(Debug, Clone, Copy)]
pub struct CallIc {
    /// IC_MAGIC = filled.
    pub magic: u64,
    /// The dispatch plan (CallPlan as u8).
    pub plan: u8,
    /// The argc the decision was recorded for (arity changes re-decide).
    pub argc: u16,
}

impl Default for CallIc {
    fn default() -> Self {
        Self {
            magic: 0,
            plan: 0,
            argc: 0,
        }
    }
}

impl CallIc {
    #[inline]
    pub fn is_hit(&self, argc: u16) -> Option<u8> {
        if self.magic == IC_MAGIC && self.argc == argc {
            Some(self.plan)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// The per-(thread, code) table
// ---------------------------------------------------------------------------

/// One code object's cache arrays, indexed by RAW site id (the lowerer
/// allocs all kinds from one counter, so per-kind ids are sparse — arrays
/// are sized to the site-table length; foreign ids stay zero forever).
///
/// Interior mutability: the tables live in a thread-local map keyed by the
/// code ptr, so the interpreter consults them through a shared handle.
/// Reads borrow; fills try_borrow_mut — a fill can only collide when a
/// NESTED frame of the SAME body is mid-dispatch on this thread (recursive
/// calls through call_value re-enter run_frame); a failed fill just skips
/// recording (the site re-fills on a later non-nested miss). No lock ever
/// crosses a thread — thread-local + RefCell, the no-GIL shape.
#[derive(Debug, Default)]
pub struct IcTable {
    globals: RefCell<Vec<GlobalIc>>,
    attrs: RefCell<Vec<AttrIc>>,
    calls: RefCell<Vec<CallIc>>,
}

impl IcTable {
    /// Sized from the code object's site census.
    pub fn sized(site_count: usize) -> Self {
        Self {
            globals: RefCell::new(vec![GlobalIc::default(); site_count]),
            attrs: RefCell::new(vec![AttrIc::default(); site_count]),
            calls: RefCell::new(vec![CallIc::default(); site_count]),
        }
    }

    /// The consult surface: an immutable borrow for the stamp check + plan
    /// dispatch. Panics are impossible: borrows never cross an await/yield
    /// and fills use try_borrow_mut (a conflicting fill returns instead).
    #[inline]
    pub fn with_globals<R>(&self, f: impl FnOnce(&[GlobalIc]) -> R) -> R {
        f(&*self.globals.borrow())
    }
    #[inline]
    pub fn with_attrs<R>(&self, f: impl FnOnce(&[AttrIc]) -> R) -> R {
        f(&*self.attrs.borrow())
    }
    #[inline]
    pub fn with_calls<R>(&self, f: impl FnOnce(&[CallIc]) -> R) -> R {
        f(&*self.calls.borrow())
    }
    /// The fill surface: try_borrow_mut (None = nested-dispatch collision,
    /// skip the fill).
    #[inline]
    pub fn with_globals_mut<R>(&self, f: impl FnOnce(&mut [GlobalIc]) -> R) -> Option<R> {
        let mut g = self.globals.try_borrow_mut().ok()?;
        Some(f(&mut *g))
    }
    #[inline]
    pub fn with_attrs_mut<R>(&self, f: impl FnOnce(&mut [AttrIc]) -> R) -> Option<R> {
        let mut a = self.attrs.try_borrow_mut().ok()?;
        Some(f(&mut *a))
    }
    #[inline]
    pub fn with_calls_mut<R>(&self, f: impl FnOnce(&mut [CallIc]) -> R) -> Option<R> {
        let mut c = self.calls.try_borrow_mut().ok()?;
        Some(f(&mut *c))
    }
}

// ---------------------------------------------------------------------------
// Stamp collectors (the live-counter reads every hit makes)
// ---------------------------------------------------------------------------

/// The receiver's class stamp tuple: (payload ptr, type_version,
/// chain_surgery_version, names dict epoch). `None` = the receiver has no
/// cacheable class payload (primitives, modules) — attr sites on those
/// shapes stay uncached (their answers ride the value-family fast arms).
pub fn class_stamps(t: &Arc<TypeRef>) -> (u64, u64, u64, u64) {
    (
        Arc::as_ptr(t) as usize as u64,
        t.type_version.load(std::sync::atomic::Ordering::Relaxed),
        t.chain_surgery_version(),
        t.dict.mutation_epoch(),
    )
}

/// Site-kind helper for tests/probes (which array a site id belongs to).
pub fn site_kind_of(code: &ap_bytecode::code::CodeObject, site: u32) -> Option<SiteKind> {
    code.sites.get(site).map(|i| i.kind)
}

/// Gc<T> as usize helper (identity stamps).
pub fn gc_ptr<T>(g: &Gc<T>) -> u64 {
    Gc::as_ptr(g) as usize as u64
}

//! Inline-cache plan constants (leaf fast-path/02-perf/01).
//!
//! The site table assigns stable per-site ids ([`SiteTable`]); the bytecode
//! interpreter (ap-run/bytecode_interp.rs + ap-run/ic.rs) keeps a per-(thread,
//! CodeObject) table of TYPED cache entries keyed by those ids and consults/
//! fills them at the LoadGlobal / LoadAttr / LoadAttrCall dispatch arms.
//!
//! WHY THE ENTRIES LIVE IN AP-RUN: a warm hit must be a handful of compares
//! plus a direct load — bit-packing Values/Arc handles into machine words
//! (the shape this crate's CacheSlot reserved) forces a reconstruct step on
//! every hit and a GC-pinning argument on every fill. The interpreter owns
//! the semantics (this crate is a pure translation layer by master.md), so
//! the typed entries live with the arms that answer from them. Only the
//! PLAN constants (the dispatch decision words) are shared here, so the
//! invalidation audit map in one place names every cacheable resolution.
//!
//! STAMP MAP (which counter invalidates which plan — the 02.05b audit rows):
//! - GlobalLoad positives: (globals dict epoch, builtins dict epoch, dict
//!   pair identity). Any write into either dict rebinds/removes the name.
//!   Non-dict fallback answers (module-globals / exc-class) are never
//!   recorded — they stay fully dynamic.
//! - AttrPlan::ClassValue / InstDict / BindFunc: (receiver class identity,
//!   chain_surgery_version over the receiver's MRO, class names-dict epoch).
//!   C.x = / del C.x / method overwrite / subclass override all bump the
//!   pair via TypeObject::bump_class_versions (audit rows 10/11/16); the
//!   names epoch additionally catches direct names-dict writes that change
//!   HOW a name answers (descriptor/__getattr__ installs).
//! - AttrPlan::InstDict additionally re-probes the instance dict per hit
//!   (one lock probe): instance values rebind independently of any class
//!   counter, so the VALUE is never cached for instance reads — only the
//!   class-level resolution ("this name is answered by the instance dict")
//!   is, and even that is stamped against later class-attr additions that
//!   would shadow.
//! - LoadAttrCall / CallFunction (CallPlan): the (receiver family tag,
//!   method name, argc) key is re-derived from the instruction + stack
//!   receiver at dispatch — value-family tags are constants and cannot be
//!   mutated, so no version stamp is required; the entry memoizes only
//!   WHICH dispatcher answers the shape.

/// LoadGlobal entry kinds.
pub mod global_kind {
    /// Empty slot (zero-init).
    pub const EMPTY: u8 = 0;
    /// Positive: resolved from the (globals, builtins) dict pair; the
    /// stamped epochs prove the dicts are unmutated since the fill.
    pub const POSITIVE: u8 = 1;
}

/// LoadAttr arm plans (the `plan` decision word shared with ap-run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrPlan {
    /// Not cacheable (descriptors, dynamic answers, primitive mints) —
    /// always re-walk.
    Slow,
    /// Receiver is a user CLASS; the answer is a plain class-dict member
    /// (stable value under the class stamps). Cached VALUE re-served.
    ClassValue,
    /// Receiver is a user INSTANCE; the class resolution is "instance dict
    /// supplies this name". Re-probe the instance dict per hit (stamped
    /// against class-level shadowing).
    InstDict,
    /// Receiver is a user INSTANCE; the class MRO supplies a plain function
    /// (the owner rides the binding). Hit = fresh BoundMethod mint via
    /// bind_function_owned (CPython identity pin: never cache the mint).
    BindFunc,
}

/// Fused-call site plans (LoadAttrCall) — which dispatcher answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPlan {
    /// Not memoizable — generic route every time.
    Slow,
    /// The call_builtin_values table answers (tag, name) with the receiver
    /// prepended as args[0].
    PrimValues,
    /// methods::call_method answers directly (its own match arms).
    CallMethod,
}

/// Filled-slot magic (any other value = empty; entries zero-init).
pub const IC_MAGIC: u64 = 0x1C4C4F43_49434150; // "PACICOLC1" — arbitrary nonzero

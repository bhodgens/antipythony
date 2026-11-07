//! Process-level debug-flag cache (memperf W1, audit finding A3).
//!
//! The interpreter historically called `std::env::var("APX_*")` directly on
//! hot dispatch paths (~90 sites: every `raise()`, every binop error arm,
//! every attr probe, twice per GC collect). Each `env::var` scans the whole
//! environment and allocates. This module snapshots the debug flags ONCE at
//! the first probe (the interpreter binary sets none of them after startup)
//! into an `AtomicU64` bitset: a probe is a single atomic load + branch.
//!
//! Non-debug configuration variables (AP_MAX_RSS_GB, AP_GEN_STACK_MB,
//! ANTIPTYHONY_*, HOME, COLUMNS, LINES, APX_GC_THRESHOLD) stay on
//! `std::env::var` — they are read a bounded number of times, not per-op.
//!
//! Byte-exactness: the flags are only read for diagnostic side-channels
//! (eprintln traces). A snapshot is behaviorally identical because the
//! interpreter never toggles its own debug flags mid-run through
//! `os.putenv` (os.putenv writes the process env dict + the C environ, but
//! no interpreter decision reads APX_* after entry; os.unsetenv likewise).
//!
//! Adding a new flag: add an `APXFlag` variant + wire it into
//! `ALL_FLAG_BITS` in `snapshot()`.

use std::sync::atomic::{AtomicU64, Ordering};

/// Bit assignments — one per debug flag that is probed per-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApxFlag {
    TbDbg,
    GenDebug,
    AssertDbg,
    ModDbg,
    IdxDebug,
    TDebug,
    SuperDbg,
    ScopeDebug,
    MroDbg,
    IcDbg,
    Debug,
    StmtDbg,
    DbgIss,
    DbgHook,
    ClosureDebug,
    AttrDbg,
    TbDebug,
    SfcDbg,
    IsiDbg,
    DecDbg,
    Calltrace,
    AttrMiss,
    GcDump,
    GcStats,
    SreDebug,
    ImportTrace,
}

impl ApxFlag {
    const fn bit(self) -> u64 {
        1u64 << (self as u64)
    }
}

/// Cached bitset. `snapshot()` fills it exactly once; before that it reads
/// as zero (no flags) which is also the default when unset — safe default.
static FLAGS: AtomicU64 = AtomicU64::new(0);

const FLAG_NAMES: &[(&str, ApxFlag)] = &[
    ("APX_TBDBG", ApxFlag::TbDbg),
    ("APX_GEN_DEBUG", ApxFlag::GenDebug),
    ("APX_ASSERTDBG", ApxFlag::AssertDbg),
    ("APX_MODDBG", ApxFlag::ModDbg),
    ("APX_IDX_DEBUG", ApxFlag::IdxDebug),
    ("APX_TDEBUG", ApxFlag::TDebug),
    ("APX_SUPERDBG", ApxFlag::SuperDbg),
    ("APX_SCOPE_DEBUG", ApxFlag::ScopeDebug),
    ("APX_MRODBG", ApxFlag::MroDbg),
    ("APX_ICDBG", ApxFlag::IcDbg),
    ("APX_DEBUG", ApxFlag::Debug),
    ("APX_STMTDBG", ApxFlag::StmtDbg),
    ("APX_DBG_ISS", ApxFlag::DbgIss),
    ("APX_DBG_HOOK", ApxFlag::DbgHook),
    ("APX_CLOSURE_DEBUG", ApxFlag::ClosureDebug),
    ("APX_ATTRDBG", ApxFlag::AttrDbg),
    ("APX_TB_DEBUG", ApxFlag::TbDebug),
    ("APX_SFCDBG", ApxFlag::SfcDbg),
    ("APX_ISIDBG", ApxFlag::IsiDbg),
    ("APX_DECDBG", ApxFlag::DecDbg),
    ("APX_CALLTRACE", ApxFlag::Calltrace),
    ("APX_ATTRMISS", ApxFlag::AttrMiss),
    ("APX_GC_DUMP", ApxFlag::GcDump),
    ("GCSTATS", ApxFlag::GcStats),
    ("AP_SRE_DEBUG", ApxFlag::SreDebug),
    ("AP_IMPORT_TRACE", ApxFlag::ImportTrace),
];

/// Read every debug flag from the environment once, cache the bitset.
/// Idempotent (first caller wins; later calls are a no-op).
pub fn snapshot() {
    let mut bits = 0u64;
    for (name, flag) in FLAG_NAMES {
        if std::env::var_os(name).is_some() {
            bits |= flag.bit();
        }
    }
    // First writer wins; a racing second snapshot computes the same bits
    // (the environment does not change between the two loads).
    let _ = FLAGS.compare_exchange(0, bits, Ordering::Relaxed, Ordering::Relaxed);
}

/// Per-op probe: `debug_flag(ApxFlag::TbDbg)`. One atomic load.
#[inline(always)]
pub fn debug_flag(flag: ApxFlag) -> bool {
    FLAGS.load(Ordering::Relaxed) & flag.bit() != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_flags_read_false() {
        // Fresh test binary: no APX_* in the environment.
        snapshot();
        assert!(!debug_flag(ApxFlag::TbDbg));
        assert!(!debug_flag(ApxFlag::GenDebug));
    }
}

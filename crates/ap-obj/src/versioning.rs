//! 02.05b Risk 4 — cross-object version domains (docs/ic-version-audit.md).
//!
//! Per-object counters live on their owners (`PyDict::mutation_epoch` — the
//! single dict-version source of truth, audit rule 4; `PyObject::
//! type_version` / `surgery_version` — the type/class-surgery domains).
//! This module holds only what has NO natural per-object owner: the
//! `builtins` shadowing version (audit row 18) — the GlobalIC's second
//! guard, bumped when the `builtins` dict's shadowable surface changes at
//! bootstrap. Rare-path by design: the audit pins it as "rare path; lands
//! with the GlobalIC consumer".
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::sync::atomic::{AtomicU64, Ordering};

/// The builtins-shadowing version (audit row 18; GlobalIC second guard).
/// Seeded 1; bumped by the bootstrap paths that (re)install shadowable
/// names into `builtins` (ap-run/classes.rs install_exception_classes /
/// importer bootstrap). A GlobalIC whose cached resolution went through
/// builtins re-validates against this counter; any bump invalidates.
static BUILTINS_VERSION: AtomicU64 = AtomicU64::new(1);

/// Read the builtins-shadowing version (GlobalIC guard; Relaxed — any
/// change invalidates, per the 02.09 epoch-pattern contract).
pub fn builtins_version() -> u64 {
    BUILTINS_VERSION.load(Ordering::Relaxed)
}

/// Bump the builtins-shadowing version (bootstrap shadow-reinstall sites).
pub fn bump_builtins_version() {
    BUILTINS_VERSION.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_version_seeded_at_one_and_monotonic() {
        let v0 = builtins_version();
        bump_builtins_version();
        let v1 = builtins_version();
        bump_builtins_version();
        let v2 = builtins_version();
        assert!(v1 > v0 && v2 > v1);
    }
}

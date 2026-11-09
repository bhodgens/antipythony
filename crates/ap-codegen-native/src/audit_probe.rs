//! IC audit mode (`APX_IC_AUDIT=1`) — leaf Risk 4 debug cross-check.
//!
//! On every IC hit in audit mode, the cached result is recomputed through
//! the slow path and compared. Mismatch = fail-closed (explicit panic in
//! debug builds, SystemError park in release). This module is the substrate
//! the recording helpers consult; the counters themselves land with the
//! tier-1 typed-codegen wave (see docs/ic-version-audit.md).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// Whether audit mode is enabled (snapshot-cached like the other APX_*
/// debug flags — ap_obj::debug_flags discipline).
pub fn ic_audit_enabled() -> bool {
    std::env::var("APX_IC_AUDIT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Cross-check a cached IC value against a slow-path recompute.
///
/// `site` names the cache site (for the failure message); `cached` is
/// what the IC returned; `recompute` is the slow-path closure. In audit
/// mode a mismatch is a fail-closed panic (debug) — the gate for Risk 4.
/// Outside audit mode this is a no-op (zero hot-path cost).
#[inline]
pub fn audit_ic_hit<T: PartialEq + std::fmt::Debug>(
    site: &str,
    cached: &T,
    recompute: impl FnOnce() -> T,
) {
    if !ic_audit_enabled() {
        return;
    }
    let slow = recompute();
    if cached != &slow {
        panic!(
            "IC AUDIT FAIL [{site}]: cached {cached:?} != slow-path {slow:?} \
             (stale cache — a version bump was missed; see docs/ic-version-audit.md)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The three tests mutate the process-global env var; under parallel test
    // threads a set/remove race flips `audit_disabled_by_default_is_noop`
    // into the panic path (observed 2026-11-08). Serialize env access.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn audit_disabled_by_default_is_noop() {
        // Without APX_IC_AUDIT the hook must not recompute (and must not
        // panic on a mismatch).
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("APX_IC_AUDIT");
        audit_ic_hit("test", &1, || 999);
    }

    #[test]
    #[should_panic(expected = "IC AUDIT FAIL")]
    fn audit_enabled_panics_on_mismatch() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("APX_IC_AUDIT", "1");
        audit_ic_hit("test", &1, || 999);
    }

    #[test]
    fn audit_enabled_passes_on_match() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("APX_IC_AUDIT", "1");
        audit_ic_hit("test", &42, || 42);
        std::env::remove_var("APX_IC_AUDIT");
    }
}

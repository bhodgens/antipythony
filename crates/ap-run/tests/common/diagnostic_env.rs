//! Clean-environment probe spawning + diagnostic-stderr filtering for the
//! differential harnesses (APX_* diagnostic env-leak false-RED fix).
//!
//! Root cause (read-only audit 2026-11): several Rust harnesses built their
//! probe children with a bare `Command::new`, so an exported
//! `APX_TIER=bytecode` + `APX_TIER_STATS=1` in the *invoking shell* leaked
//! into the child. The product's diagnostic emission is correctly opt-in
//! (`jit_hook.rs` gates `[TIER-STATS]` on `APX_TIER_STATS`,
//! `bytecode_interp.rs` gates `[BC-STATS]` on its counter knob,
//! `interp.rs` gates `GCSTATS`), but the leaked stats line became the LAST
//! line of stderr and replaced the exception tail that
//! `import_guard_differential.rs` byte-pins — an 8/9 RED on a healthy tree.
//!
//! Fix discipline (mirrors `crates/ap-conform/src/lib.rs:160-169`): scrub the
//! CHILD environment, never the parent's and never product code. Explicit
//! test env (e.g. tier-mode gates that deliberately opt into stats) is
//! re-applied AFTER the scrub, so a deliberate opt-in still works while an
//! inherited shell value cannot.

/// Diagnostic keys that must never be inherited by a differential probe.
/// Non-diagnostic APX keys (e.g. `APX_IC_AUDIT`, `APX_SYNC_TIERUP`) are not
/// on this list: they are legitimate gate controls, not stats emitters.
pub const DIAGNOSTIC_ENV_KEYS: &[&str] = &[
    "APX_TIER",
    "APX_TIER_STATS",
    "APX_TIER_DEBUG",
    "APX_TIER1_STATS",
    "APX_TIERUP_STATS",
    "APX_OSR_STATS",
    "APX_CALLDBG",
    "APX_CE_TRACE",
    "APX_GC_THRESHOLD",
    "APX_HASHDBG",
    "APX_TDEBUG",
    "APX_SCHED_TRACE",
    "APX_BP_DEBUG",
];

/// Stderr line prefixes emitted by the opt-in diagnostic seams. Filter these
/// out before byte-comparing stderr across probes/schedules.
pub const DIAGNOSTIC_LINE_PREFIXES: &[&str] = &[
    "[TIER-STATS]",
    "[TIERUP-STATS]",
    "[TIER1-STATS]",
    "[BC-STATS]",
    "[OSR-STATS]",
    "[AOT-STATS]",
    "GCSTATS",
];

fn is_diagnostic_key(key: &std::ffi::OsStr) -> bool {
    key.to_str()
        .is_some_and(|k| DIAGNOSTIC_ENV_KEYS.contains(&k))
}

/// Clear the child environment, then restore the parent environment minus
/// the diagnostic denylist. `PATH`, `HOME`, `LANG` and every `CARGO_*`
/// variable are preserved (the ap-conform baseline is env_clear + PATH/HOME/
/// LANG; here we keep the rest of the parent env too so unrelated suites are
/// unaffected, and we guarantee the baseline keys even when unset).
fn reapply_clean_parent_env(cmd: &mut std::process::Command) {
    cmd.env_clear();
    for (key, value) in std::env::vars_os() {
        if is_diagnostic_key(&key) {
            continue;
        }
        cmd.env(key, value);
    }
    for key in ["PATH", "HOME"] {
        if std::env::var_os(key).is_none() {
            cmd.env(key, "");
        }
    }
    if std::env::var_os("LANG").is_none() {
        cmd.env("LANG", "C");
    }
}

/// Build a probe `Command` whose environment has no inherited diagnostic
/// keys. The caller adds args and any explicit test env afterwards.
pub fn clean_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    reapply_clean_parent_env(&mut cmd);
    cmd
}

/// Scrub an already-built `Command` in place. Program, args, cwd and pipes
/// are untouched (env_clear only affects the environment block); every env
/// value the test set EXPLICITLY is re-applied after the scrub, so deliberate
/// opt-ins such as `APX_TIER_STATS=1` in the JIT-exercise gate still reach
/// the child while an inherited shell value does not.
pub fn sanitize(cmd: &mut std::process::Command) {
    use std::ffi::OsString;
    let explicit: Vec<(OsString, Option<OsString>)> = cmd
        .get_envs()
        .map(|(k, v)| (k.to_os_string(), v.map(|s| s.to_os_string())))
        .collect();
    reapply_clean_parent_env(cmd);
    for (key, value) in explicit {
        match value {
            Some(value) => {
                cmd.env(key, value);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
}

/// Drop opt-in diagnostic lines from captured stderr before byte comparison.
/// Preserves line order and the no-trailing-newline shape of the existing
/// ad hoc `.lines().filter(...).join("\n")` filters this replaces.
pub fn strip_diagnostic_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            !DIAGNOSTIC_LINE_PREFIXES
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

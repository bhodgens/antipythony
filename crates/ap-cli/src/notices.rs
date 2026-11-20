//! `--version` / `--notices` (leaf D13, closes 04.03's D1).
//!
//! DECISION (recorded per the leaf dispatch): the notices are EMBEDDED at
//! build time via include_bytes! from the repo-root THIRD_PARTY_NOTICES
//! file. The release archive still ships the same file as a standalone
//! FILE (installed to $PREFIX/lib/antipythony/THIRD_PARTY_NOTICES by
//! scripts/install.sh); the embedded copy and the archive file are
//! regenerated from the same tree at release time, but they CAN drift
//! between releases if the archive is rebuilt without re-embedding (or
//! vice versa) — `xtask notices --check` at the release gate is the drift
//! guard for the file itself; this surface always reports the copy the
//! BINARY was built with, which is the honest provenance answer.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::io::Write;

/// Repo-root notices file at compile time (crates/ap-cli/src -> workspace root).
pub const EMBEDDED_NOTICES: &[u8] = include_bytes!("../../../THIRD_PARTY_NOTICES");

/// `antipythony --version` — name + crate version + short commit-ish when
/// available. The commit is optional: option_env! is None in builds without
/// the AP_COMMIT_SHA env set (release.yml sets it per-build; plain cargo
/// builds print version only — honest, not invented).
pub fn version_line() -> String {
    let mut line = format!("antipythony {}", env!("CARGO_PKG_VERSION"));
    if let Some(sha) = option_env!("AP_COMMIT_SHA") {
        if !sha.is_empty() {
            let short = if sha.len() > 12 { &sha[..12] } else { sha };
            line.push_str(&format!(" (commit {short})"));
        }
    }
    line
}

/// `antipythony --version` verb body: print to stdout, exit 0.
pub fn cmd_version() -> i32 {
    println!("{}", version_line());
    0
}

/// `antipythony --notices` verb body: print the embedded THIRD_PARTY_NOTICES
/// verbatim to stdout, exit 0.
pub fn cmd_notices() -> i32 {
    // stdout, byte-exact: write raw bytes rather than println! (the file
    // already carries its own trailing newline; lossy-String round-trips
    // would mangle any non-UTF8 byte — write the embedded bytes as-is).
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(EMBEDDED_NOTICES);
    let _ = lock.flush();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_carries_name_and_crate_version() {
        let line = version_line();
        assert!(line.starts_with("antipythony "), "got: {line}");
        assert!(line.contains(env!("CARGO_PKG_VERSION")), "got: {line}");
    }

    #[test]
    fn embedded_notices_nonempty_and_headered() {
        assert!(!EMBEDDED_NOTICES.is_empty());
        let head = &EMBEDDED_NOTICES[..EMBEDDED_NOTICES.len().min(64)];
        let head = String::from_utf8_lossy(head);
        assert!(
            head.contains("THIRD_PARTY_NOTICES"),
            "embedded notices missing header: {head}"
        );
    }
}

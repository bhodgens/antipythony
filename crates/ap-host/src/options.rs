//! Capability-scoped run options (leaf 03.05): what a module MAY see.
//!
//! DEFAULT DENY: a module run with default [`RunOptions`] sees NO arguments
//! and NO environment variables — the host env is never inherited wholesale
//! (leaf brief scope 1: "explicit allowlist per Store, never inherit the
//! host env"). Args/env are opt-in through explicit lists, which the CLI
//! surfaces as flags.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// Per-run capability grants. All lists are explicit allowlists.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Bounds for the captured stdout/stderr pipes (bytes).
    pub capture_cap: usize,
    /// argv[1..] visible to the module (argv[0] is always the module name —
    /// WASI args_get convention; empty = NO args granted).
    pub args: Vec<String>,
    /// Explicit key=value environment grant. Empty = NO env granted.
    pub env: Vec<(String, String)>,
    /// argv[0] presented to the module (defaults to the module filename).
    pub argv0: Option<String>,
    /// Record time-to-first-stdout-byte in the RunResult (startup
    /// measurement, leaf 03.05; ~free when off, no extra drain work).
    pub measure_ttfb: bool,
}

impl RunOptions {
    /// The default capture cap (256 KiB — the corpus gates' size).
    pub const DEFAULT_CAP: usize = 256 * 1024;

    /// Defaults with the standard capture cap.
    pub fn new() -> Self {
        Self {
            capture_cap: Self::DEFAULT_CAP,
            args: Vec::new(),
            env: Vec::new(),
            argv0: None,
            measure_ttfb: false,
        }
    }
}

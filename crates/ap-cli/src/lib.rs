#![forbid(unsafe_code)]
// issue #18 wave 3 ratchet: measured zero-unsafe target (scripts/unsafe_report.sh + clippy undocumented_unsafe_blocks = 0)
//! the antipythony binary (run/build/new verbs)

pub mod build;
pub mod hybrid;
pub mod notices;
pub mod wasm;

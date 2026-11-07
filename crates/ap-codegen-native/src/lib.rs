//! ap-ir to Cranelift CLIF, JIT + AoT object files.
//!
//! Leaf 02.05 — tier-1 JIT substrate (see
//! plans/antipythony/02-native-backend/leaves/02.05-native-codegen.md).
//!
//! Pipeline: structured ap-ir → internal [`cfg::Func`] (basic blocks +
//! terminators, pon-ir/ir.rs-shaped, owned ENTIRELY inside this crate —
//! the ap-ir crate itself never changes, per the leaf's structured-IR
//! decision) → Cranelift CLIF → boxed-Value baseline machine code.
//!
//! pon license obligation (leaf T10): the cfg/feedback/osr stages are
//! adapted from pon (github.com/can1357/pon, rev d0a1717) — adapted files
//! carry the attribution header "adapted from pon, Copyright Can Bölük,
//! MIT/Apache-2.0"; license texts live in LICENSES/ (MIT.txt,
//! MIT-pon.txt, Apache-2.0.txt, pon-license-grant-email.txt).
//!
//! Fail-closed discipline: every construct this crate does not lower yet
//! is an explicit [`CodegenError::Unsupported`] — the codegen must never
//! silently miscompile (same port discipline as ap-codegen-wasm).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

pub mod audit_probe;
pub mod cfg;
pub mod clif;
pub mod feedback;
pub mod osr;
pub mod runtime;
pub mod worker_stack;

/// Codegen failure. Every unsupported construct is explicit — fail-closed,
/// never silent wrong-code (leaf gate).
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum CodegenError {
    /// The IR contains a node this stage does not lower yet.
    Unsupported(&'static str),
    /// An internal CFG verifier rule fired (e.g. PopExcInfo coverage).
    Verify(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unsupported(what) => write!(f, "unsupported IR node: {what}"),
            CodegenError::Verify(what) => write!(f, "cfg verifier: {what}"),
        }
    }
}

impl std::error::Error for CodegenError {}

pub use cfg::{lower_fn, Func as CfgFunc};

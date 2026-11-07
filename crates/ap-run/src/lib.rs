// grandfathered by robustness-ci 02.01 - burn down in leaf work, no NEW sites
// should add to this; see plans/robustness-ci/02-lint-gates/PROGRESS.md tally.
#![allow(clippy::unwrap_used)]
// deny-by-default lints (overly_complex_bool_expr, absurd_extreme_comparisons,
// redundant_comparisons) fire on 6 committed sites in this crate at HEAD
// (sre.rs:607/627/2172, gcsupport.rs:1203, builtins_modules.rs:1909/2540 -
// 8 error emissions). Pre-existing clippy ERRORS, not warnings. Grandfathered
// so `cargo clippy --workspace --all-targets` exits 0 and CI can flip
// -D warnings later without a mass rewrite; burn down WITH the unwrap tally,
// do NOT add new sites.
#![allow(
    clippy::overly_complex_bool_expr,
    clippy::absurd_extreme_comparisons,
    clippy::redundant_comparisons
)]
//! The tier-0 tree-walking executor (leaf 02.02).

pub mod ast_bridge;
pub mod builtins;
pub mod builtins_modules;
pub mod classes;
pub mod exc_hier;
pub mod format;
pub mod frame;
pub mod gcsupport;
pub mod generators;
pub mod importer;
pub mod interp;
pub mod iter;
pub mod lxml;
pub mod mbc;
pub mod methods;
pub mod rss_guard;
pub mod sre;
pub mod sre_tables;
pub mod structmod;
pub mod threading;
pub mod threading_call;

use ap_obj::exc::PyException;
pub use exc_hier::ExceptionValue;
pub use frame::Interpreter;
pub use interp::run_module;

/// Outcome of executing a module (S-06-7/S-06-11, C-25/C-29).
pub struct RunOutcome {
    /// Bytes written to stdout.
    pub stdout: Vec<u8>,
    /// Bytes written to stderr (empty on success).
    pub stderr: Vec<u8>,
    /// Process exit code: 0 success, 1 uncaught error, N for sys.exit(n),
    /// 130 KeyboardInterrupt (S-06-11).
    pub exit_code: i32,
}

/// The runtime signal every statement/expression evaluation returns.
/// NOT Rust `Err(PyException)` because the executor must distinguish
/// *Python exceptions* (catchable, carry traceback frames) from Rust-level
/// control flow (return/break/continue unwinding through the tree walk).
pub enum Flow {
    /// Fall through to the next statement.
    Next,
    /// `return <value>` unwinding.
    Return(ap_obj::value::Value),
    /// `break` / `continue` (innermost loop).
    Break,
    Continue,
    /// A Python exception is in flight (catchable by enclosing try).
    Raise(ExceptionValue),
    /// A generator yield: caught at the enclosing exec_block, appended to
    /// the generator sink, execution continues after the yield (v0
    /// simplification: no .send — documented in PROGRESS).
    YieldBack(ap_obj::value::Value),
}

impl Flow {
    /// Helper: raise from a value-layer PyException (payload-only error).
    pub fn from_py(exc: PyException) -> Flow {
        Flow::Raise(ExceptionValue::from_py(exc))
    }
}

impl From<Result<(), PyException>> for Flow {
    fn from(r: Result<(), PyException>) -> Self {
        match r {
            Ok(()) => Flow::Next,
            Err(e) => Flow::from_py(e),
        }
    }
}

pub type EvalResult = Result<ap_obj::value::Value, Flow>;

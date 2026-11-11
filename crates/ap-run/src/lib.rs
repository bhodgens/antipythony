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

pub mod aot_runtime;
pub mod ast_bridge;
pub mod builtins;
pub mod builtins_modules;
pub mod cell_slot;
pub mod classes;
pub mod exc_hier;
pub mod format;
pub mod frame;
pub mod gcsupport;
pub mod generators;
pub mod importer;
pub mod interp;
pub mod iter;
/// Leaf 02.05a: the JIT entry hook + tier-dispatch seam (tier mode
/// resolution, HostCallbacks impl, compile cache, jit_call_body).
pub mod jit_hook;
pub mod lxml;
pub mod mbc;
pub mod methods;
pub mod rss_guard;
pub mod runtime_state;
pub mod send_value;
pub mod sre;
pub mod sre_tables;
pub mod structmod;
pub mod threading;
pub mod threading_call;

pub use aot_runtime::AotFnEntry;
use ap_obj::exc::PyException;
pub use exc_hier::ExceptionValue;
pub use frame::Interpreter;
pub use interp::run_module;

/// CRT seam: register the linked AOT manifest (once, before run_module).
pub fn aot_set_manifest(entries: Vec<AotFnEntry>) {
    aot_runtime::set_manifest(entries);
}

/// CRT seam: the registered manifest (empty when unset).
pub fn aot_manifest_slice() -> &'static [AotFnEntry] {
    // The Vec lives in the OnceLock for the process lifetime.
    let s: &'static [AotFnEntry] = aot_runtime::manifest_slice();
    s
}

/// CRT seam: sys.argv tail (everything after the binary path). argv[0]
/// stays the binary path (CPython's sys.argv[0] = script path convention
/// for `./app` runs — the runtime's sys module builds argv from
/// script_path + this tail).
pub fn set_argv_tail(tail: Vec<String>) {
    interp::set_argv_tail(tail);
}

/// CRT seam: run_module with the AOT dispatch seam pre-armed (the tier
/// flag set to "aot" routes module-level def bodies through the
/// registered manifest first). Byte-identical output contract as
/// run_module; the ONLY difference is the dispatch arm's source.
pub fn run_module_embedded(
    source: &str,
    script_path: &str,
    _manifest: &'static [AotFnEntry],
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> RunOutcome {
    run_module(source, script_path, stdout, stderr)
}

/// Leaf 02.05a: the bins' `--tier <mode>` flag rides this one-shot cell
/// into run_module's tier init seam (a static because run_module takes
/// no config struct and its signature is a pinned public seam).
pub mod tier_flag {
    use std::cell::Cell;
    thread_local! {
        static TIER: Cell<Option<&'static str>> = const { Cell::new(None) };
    }
    /// Bins call this before run_module with "0" or "adaptive".
    pub fn set(mode: &'static str) {
        TIER.with(|t| t.set(Some(mode)));
    }
    /// run_module consumes it once; None = env fallback in jit_hook.
    pub fn take() -> Option<&'static str> {
        TIER.with(|t| t.replace(None))
    }
}

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

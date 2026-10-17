//! The tier-0 tree-walking executor (leaf 02.02).

pub mod builtins;
pub mod exc_hier;
pub mod format;
pub mod frame;
pub mod interp;
pub mod iter;
pub mod methods;

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

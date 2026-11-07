//! Worker-thread interpreter with an explicitly sized stack (leaf 02.05
//! scope item 9 — debug-build stack-overflow fix).
//!
//! Precedent: `AP_GEN_STACK_MB` fiber fix (9ee848c, 8MB reservation after
//! 2MB SIGBUSes in the pluggy debug chain). This module applies the same
//! discipline to the MODULE-EXECUTION frame: `run_on_worker_stack` spawns
//! a dedicated thread sized from `APX_MAIN_STACK_MB` (default 256MB,
//! matching the ap-cli run-verb precedent) and joins it, converting a
//! would-be SIGSEGV (silent, undebuggable) into a normal JoinError/panic.
//!
//! ap-run's `run_module` stays UNCHANGED semantically: the tier-0
//! interpreter is stack-hungry (~50KB/frame per the ap-cli comment), and
//! the recursion_limit=1000 default needs ≳50MB; 256MB gives headroom for
//! the vendored-stdlib debug chains (scrapy/twisted measured 70-120s per
//! import chain in debug — deep frames, not deep stacks, is the fix we
//! must NOT regress).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// Stack size for the interpreter worker thread, bytes.
/// `APX_MAIN_STACK_MB` env override (integer MB); default 256MB.
pub fn worker_stack_size() -> usize {
    let mb = std::env::var("APX_MAIN_STACK_MB")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(256);
    mb.max(16) << 20 // floor at 16MB: below that the tier-0 walker is known-bad
}

/// Run `f` on a dedicated worker thread with [`worker_stack_size`] bytes
/// of stack; join and return its result.
///
/// Errors: thread spawn failure (propagated); panics inside `f` are
/// re-raised on the caller thread by `join` (unwrap_or re-panic), so the
/// failure mode is always an explicit Rust-level error — never a silent
/// wrong answer and never a bare SIGSEGV without a named suspect.
pub fn run_on_worker_stack<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let size = worker_stack_size();
    std::thread::Builder::new()
        .name("ap-interp-main".into())
        .stack_size(size)
        .spawn(f)
        .map_err(|e| format!("ap-codegen-native: spawn interpreter worker: {e}"))?
        .join()
        .map_err(|e| {
            if let Some(s) = e.downcast_ref::<&'static str>() {
                (*s).to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                format!("interpreter worker panicked: {e:?}")
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_stack_is_256mb() {
        std::env::remove_var("APX_MAIN_STACK_MB");
        assert_eq!(worker_stack_size(), 256 << 20);
    }

    #[test]
    fn env_override_respected_with_floor() {
        std::env::set_var("APX_MAIN_STACK_MB", "64");
        assert_eq!(worker_stack_size(), 64 << 20);
        std::env::set_var("APX_MAIN_STACK_MB", "1");
        assert_eq!(worker_stack_size(), 16 << 20, "floor at 16MB");
        std::env::remove_var("APX_MAIN_STACK_MB");
    }

    #[test]
    fn worker_returns_values_and_panics() {
        let v = run_on_worker_stack(|| 40 + 2).expect("spawn ok");
        assert_eq!(v, 42);
        let err = run_on_worker_stack(|| -> i32 { panic!("boom") }).unwrap_err();
        assert!(err.contains("boom"));
    }

    /// THE GATE (leaf scope item 9): deep recursion under the worker
    /// thread completes instead of overflowing. This exercises the same
    /// frame-depth shape as the 500-deep chained-call corpus probe
    /// (t1_016_deep_call_chain_500.py) at the Rust level — 500 nested
    /// calls through a frame-heavy closure chain, on the sized stack.
    #[test]
    fn deep_chained_calls_survive_on_sized_stack() {
        let depth = run_on_worker_stack(|| {
            fn level(n: u32) -> u32 {
                if n == 0 {
                    0
                } else {
                    // Local bulk per frame approximates the tier-0
                    // interpreter's frame cost (~50KB/frame at full tilt;
                    // this is the smoke variant, the corpus probe is the
                    // real gate).
                    let _pad = [0u8; 4096];
                    1 + level(n - 1)
                }
            }
            level(500)
        })
        .expect("500-deep chained calls must not overflow the worker stack");
        assert_eq!(depth, 500);
    }
}

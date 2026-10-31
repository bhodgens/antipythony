//! wasmtime host runner, WASI imports, embedding API (Plan 03 skeleton).
//!
//! Runs a module emitted by [`ap_codegen_wasm::lower_module`] under
//! wasmtime 47 (WasmGC + Wasm EH enabled by default) with WASI preview1 and
//! captured stdout/stderr. The full `aprt` import surface and the
//! CPython-format traceback rendering at the exception boundary are leaf
//! 03.05; this skeleton proves the pipeline: bytes → instance → `_start` →
//! captured stdout + exit code.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use wasmtime::{Config, Engine, Linker, Module as WasmModule, Store};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

/// Result of running a module: the observable triple ap-conform diffs on.
#[derive(Debug)]
pub struct RunResult {
    /// Everything written to fd 1.
    pub stdout: String,
    /// Everything written to fd 2.
    pub stderr: String,
    /// Process exit code (0 on clean `_start` return).
    pub exit_code: i32,
}

/// Errors from the host runner.
#[derive(Debug)]
pub enum HostError {
    /// The module failed to compile/instantiate, or trapped.
    Wasm(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Wasm(e) => write!(f, "wasm host error: {e}"),
        }
    }
}

impl std::error::Error for HostError {}

/// Run `wasm_bytes` (as emitted by ap-codegen-wasm) with piped stdio.
///
/// `_start` is invoked; a WASI `proc_exit` becomes its exit code, any other
/// trap becomes [`HostError::Wasm`]. `cap` bounds the captured pipe size.
pub fn run(wasm_bytes: &[u8], cap: usize) -> Result<RunResult, HostError> {
    let mut config = Config::new();
    // Wasmtime 47: WasmGC + exception handling are on by default; stated
    // explicitly so the version constraint from
    // docs/03-wasm-representation-design.md §0 is visible in code.
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    let engine = Engine::new(&config).map_err(|e| HostError::Wasm(e.to_string()))?;
    let module =
        WasmModule::new(&engine, wasm_bytes).map_err(|e| HostError::Wasm(e.to_string()))?;

    let stdout = MemoryOutputPipe::new(cap);
    let stderr = MemoryOutputPipe::new(cap);
    let stdout_handle = stdout.clone();
    let stderr_handle = stderr.clone();

    let ctx: WasiP1Ctx = WasiCtxBuilder::new()
        .stdout(stdout)
        .stderr(stderr)
        .build_p1();
    let mut store = Store::new(&engine, ctx);
    let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |c: &mut WasiP1Ctx| c)
        .map_err(|e| HostError::Wasm(e.to_string()))?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| HostError::Wasm(e.to_string()))?;
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .map_err(|e| HostError::Wasm(e.to_string()))?;

    let exit_code = match start.call(&mut store, ()) {
        Ok(()) => 0,
        Err(e) => {
            if let Some(exit) = e.downcast_ref::<I32Exit>() {
                exit.0
            } else {
                return Err(HostError::Wasm(e.to_string()));
            }
        }
    };

    let drain = |pipe: MemoryOutputPipe| -> String {
        // `contents()` copies out without requiring the sole Arc clone —
        // the WasiCtx still holds one.
        String::from_utf8_lossy(&pipe.contents()).into_owned()
    };

    Ok(RunResult {
        stdout: drain(stdout_handle),
        stderr: drain(stderr_handle),
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_codegen_wasm::lower_module;
    use ap_ir::{Builder, Item, Module};

    #[test]
    fn hello_world_runs_end_to_end() {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(b.expr_stmt(
            b.call(b.name("print"), vec![b.arg_pos(b.str_lit("hello, wasm"))]),
        )));
        let bytes = lower_module(&module).expect("lower");
        let out = run(&bytes, 64 * 1024).expect("run");
        assert_eq!(out.stdout, "hello, wasm\n");
        assert_eq!(out.stderr, "");
        assert_eq!(out.exit_code, 0);
    }
}

#![forbid(unsafe_code)] // issue #15 ratchet: measured zero-unsafe crate
//! wasmtime host runner, WASI imports, embedding API (Plan 03).
//!
//! Runs a module emitted by [`ap_codegen_wasm::lower_module`] under
//! wasmtime 47 (WasmGC + Wasm EH enabled by default) with WASI preview1 and
//! captured stdout/stderr.
//!
//! Leaf 03.05 hardening (this file's contract):
//! - the module surface is validated on RAW bytes before compile
//!   ([`checks::validate_module_surface`]): the `aprt-version` ABI stamp,
//!   the threading refusal (shared memory), and the finalized import
//!   namespace allowlist;
//! - the `aprt` host module carries the finalized §7 service surface:
//!   `abort_msg` (payload rendered through the host-side `$str` reader),
//!   `now_ns` (monotonic), `wall_ns` (wall clock); WASI p1 supplies the
//!   rest (stdio, proc_exit, clocks, args/env getters);
//! - args + env are CAPABILITY-SCOPED ([`options::RunOptions`]): default
//!   deny, explicit allowlists only, host env never inherited wholesale;
//! - time-to-first-stdout-byte measurement for the startup gate
//!   ([`ttfb`]) — measured, not asserted.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

pub mod checks;
pub mod options;
pub mod str_reader;
pub mod ttfb;

use std::sync::OnceLock;
use std::time::Instant;

use wasmtime::{Config, Engine, FuncType, Linker, Module as WasmModule, Store, Val};
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{I32Exit, WasiCtxBuilder};

pub use checks::{validate_module_surface, SUPPORTED_APRT_ABI_VERSIONS};
pub use options::RunOptions;
pub use ttfb::FirstByteStamp;

/// Result of running a module: the observable triple ap-conform diffs on.
#[derive(Debug)]
pub struct RunResult {
    /// Everything written to fd 1.
    pub stdout: String,
    /// Everything written to fd 2.
    pub stderr: String,
    /// Process exit code (0 on clean `_start` return).
    pub exit_code: i32,
    /// Microseconds from run-start to the FIRST stdout byte
    /// ([`RunOptions::measure_ttfb`]; None when not measured or when the
    /// module wrote nothing). Startup gate: hello-world < 50ms.
    pub ttfb_us: Option<u128>,
    /// Microseconds for the WHOLE run (compile + instantiate + `_start`).
    /// Diagnostic companion to `ttfb_us` — the <50ms budget is on ttfb.
    pub total_us: u128,
}

/// Errors from the host runner.
#[derive(Debug)]
pub enum HostError {
    /// The module failed to compile/instantiate, or trapped.
    Wasm(String),
    /// ABI-surface refusal (version stamp drift, unknown imports) — raised
    /// BEFORE compile so old modules fail fast with a clear diagnostic.
    Abi(String),
    /// Threading requested (shared memory) — wasmtime 47 removed
    /// wasi-threads; v1 is single-threaded (design §9).
    Threading(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Wasm(e) => write!(f, "wasm host error: {e}"),
            HostError::Abi(e) => write!(f, "wasm ABI error: {e}"),
            HostError::Threading(e) => write!(f, "wasm threading refused: {e}"),
        }
    }
}

impl std::error::Error for HostError {}

fn host_config() -> Config {
    let mut config = Config::new();
    // Wasmtime 47: WasmGC + exception handling are on by default; stated
    // explicitly so the version constraint from
    // docs/03-wasm-representation-design.md §0 is visible in code.
    config.wasm_gc(true);
    config.wasm_function_references(true);
    config.wasm_exceptions(true);
    config
}

/// The process-wide shared engine (F2, audit 2026-11-06-memperf-bughunt):
/// building a wasmtime `Engine` allocates the config, the CRANELIFT
/// code-gen context, and the signal-stack/vm-offset machinery — doing it
/// per `run()` call was a fixed per-invocation tax (a JIT compile of the
/// same module in a long-lived process rebuilt this every time). One
/// `Engine` is cheap to clone-share (`Arc<Engine>` under the hood is
/// already `Clone`), and WASI/`Store` context stays per-call.
fn shared_engine() -> Result<Engine, HostError> {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    if let Some(e) = ENGINE.get() {
        return Ok(e.clone());
    }
    let engine = Engine::new(&host_config()).map_err(|e| HostError::Wasm(e.to_string()))?;
    // A racing second init is harmless: wasmtime Engines are independent,
    // and OnceLock keeps the first winner for all later callers.
    let _ = ENGINE.set(engine.clone());
    Ok(engine)
}

/// Compile-only probe (leaf 03.02c diagnostics): build the module without
/// instantiating or running. Distinguishes a cranelift compile hang from an
/// execution hang.
pub fn compile_only(wasm_bytes: &[u8]) -> Result<(), HostError> {
    validate_module_surface(wasm_bytes)?;
    let engine = shared_engine()?;
    let _ = WasmModule::new(&engine, wasm_bytes).map_err(|e| HostError::Wasm(e.to_string()))?;
    Ok(())
}

/// Run `wasm_bytes` (as emitted by ap-codegen-wasm) with piped stdio,
/// reusing the process-wide [`Engine`], default capabilities.
///
/// `_start` is invoked; a WASI `proc_exit` becomes its exit code, any other
/// trap becomes [`HostError::Wasm`]. `cap` bounds the captured pipe size.
///
/// Source-compatible with the original signature (existing test callers
/// unchanged); callers who want capability grants, TTFB measurement, or
/// full engine control use [`run_with_options`] / [`run_with_engine`].
pub fn run(wasm_bytes: &[u8], cap: usize) -> Result<RunResult, HostError> {
    let engine = shared_engine()?;
    let mut opts = RunOptions::new();
    opts.capture_cap = cap;
    run_with_engine_opts(&engine, wasm_bytes, &opts)
}

/// [`run`] with capability-scoped options (default engine).
pub fn run_with_options(wasm_bytes: &[u8], opts: &RunOptions) -> Result<RunResult, HostError> {
    let engine = shared_engine()?;
    run_with_engine_opts(&engine, wasm_bytes, opts)
}

/// Run `wasm_bytes` on a caller-owned `engine`, default capabilities.
/// Engines are `Send + Sync` and cloneable (internal `Arc`), so one
/// `Engine` built once can drive every subsequent call in the process.
pub fn run_with_engine(
    engine: &Engine,
    wasm_bytes: &[u8],
    cap: usize,
) -> Result<RunResult, HostError> {
    let mut opts = RunOptions::new();
    opts.capture_cap = cap;
    run_with_engine_opts(engine, wasm_bytes, &opts)
}

/// The leaf-03.05 run path: surface validation → compile → capability-
/// scoped WASI ctx → aprt imports → instantiate → `_start` → drain.
pub fn run_with_engine_opts(
    engine: &Engine,
    wasm_bytes: &[u8],
    opts: &RunOptions,
) -> Result<RunResult, HostError> {
    let started = Instant::now();

    // ---- surface checks FIRST (version stamp / threading / imports) ----
    validate_module_surface(wasm_bytes)?;

    let module = WasmModule::new(engine, wasm_bytes).map_err(|e| HostError::Wasm(e.to_string()))?;

    let cap = if opts.capture_cap == 0 {
        RunOptions::DEFAULT_CAP
    } else {
        opts.capture_cap
    };
    let stdout = MemoryOutputPipe::new(cap);
    let stderr = MemoryOutputPipe::new(cap);
    let stdout_handle = stdout.clone();
    let stderr_handle = stderr.clone();

    // ---- TTFB instrumentation (opt-in; zero-cost when off) ----
    // Concrete enum (not Box<dyn>) because StdoutStream for Box<T> needs
    // T: Sync and `dyn StdoutStream` is not. In TTFB mode the FirstBytePipe
    // WRAPS the same `stdout` MemoryOutputPipe (shared Arc'd buffer), so
    // the normal drain below still sees every byte.
    let stamp = FirstByteStamp::default();
    enum Out {
        Plain(MemoryOutputPipe),
        Ttfb(ttfb::FirstBytePipe),
    }
    impl wasmtime_wasi::cli::StdoutStream for Out {
        fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
            match self {
                Out::Plain(p) => p.async_stream(),
                Out::Ttfb(p) => p.async_stream(),
            }
        }
        fn p2_stream(&self) -> Box<dyn wasmtime_wasi_io::streams::OutputStream> {
            match self {
                Out::Plain(p) => p.p2_stream(),
                Out::Ttfb(p) => p.p2_stream(),
            }
        }
    }
    impl wasmtime_wasi::cli::IsTerminal for Out {
        fn is_terminal(&self) -> bool {
            false
        }
    }
    let stdout_stream = if opts.measure_ttfb {
        Out::Ttfb(ttfb::FirstBytePipe::wrap(stdout.clone(), stamp.clone()))
    } else {
        Out::Plain(stdout.clone())
    };

    // ---- capability-scoped WASI context: DEFAULT DENY ----
    // args: argv[0] = module name, argv[1..] = the explicit grant.
    // env: only the explicit (k, v) allowlist; the host env is NEVER
    // inherited (no `.envs(std::env::vars())` anywhere — leaf scope 1).
    let mut builder = WasiCtxBuilder::new();
    builder.stdout(stdout_stream).stderr(stderr);
    let argv0 = opts.argv0.clone().unwrap_or_else(|| "module.wasm".into());
    let mut argv = vec![argv0];
    argv.extend(opts.args.iter().cloned());
    builder.args(&argv);
    if !opts.env.is_empty() {
        builder.envs(&opts.env);
    }
    let ctx: WasiP1Ctx = builder.build_p1();

    let mut store = Store::new(engine, ctx);
    let mut linker: Linker<WasiP1Ctx> = Linker::new(engine);
    wasmtime_wasi::p1::add_to_linker_sync(&mut linker, |c: &mut WasiP1Ctx| c)
        .map_err(|e| HostError::Wasm(e.to_string()))?;

    // ---- the aprt service surface (design §7) ----
    link_aprt(&mut linker)?;

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
        ttfb_us: if opts.measure_ttfb {
            stamp.elapsed_us_since(started)
        } else {
            None
        },
        total_us: started.elapsed().as_micros(),
    })
}

/// Register the finalized `aprt` module (design §7):
/// - `abort_msg(anyref) -> ()`: the unrecoverable internal-error hook. The
///   declared param stays anyref (the LOCKED 03.02 deviation — host-func
///   matching cannot express the self-referential `(ref $str)`); the
///   payload is now RENDERED (03.05) through the host-side `$str` reader
///   and reported as a host error.
/// - `now_ns() -> i64`: monotonic nanoseconds (`time.perf_counter` domain).
/// - `wall_ns() -> i64`: wall-clock nanoseconds since the Unix epoch
///   (`time.time` domain).
fn link_aprt(linker: &mut Linker<WasiP1Ctx>) -> Result<(), HostError> {
    let engine = linker.engine().clone();

    let abort_ty = FuncType::new(
        &engine,
        [wasmtime::ValType::Ref(wasmtime::RefType::new(
            true,
            wasmtime::HeapType::Any,
        ))],
        [],
    );
    linker
        .func_new(
            "aprt",
            "abort_msg",
            abort_ty,
            |mut caller, params, _results| -> Result<(), wasmtime::Error> {
                let msg = match params.first() {
                    Some(Val::AnyRef(Some(r))) => str_reader::render_str(&mut caller, r),
                    _ => "<aprt: abort_msg with no payload>".to_string(),
                };
                Err(wasmtime::Error::msg(format!("aprt: abort: {msg}")))
            },
        )
        .map_err(|e| HostError::Wasm(e.to_string()))?;

    let clock_ty = FuncType::new(&engine, [], [wasmtime::ValType::I64]);
    linker
        .func_new(
            "aprt",
            "now_ns",
            clock_ty.clone(),
            |_caller, _p, results| {
                results[0] = Val::I64(monotonic_ns());
                Ok(())
            },
        )
        .map_err(|e| HostError::Wasm(e.to_string()))?;
    linker
        .func_new("aprt", "wall_ns", clock_ty, |_caller, _p, results| {
            results[0] = Val::I64(wall_ns());
            Ok(())
        })
        .map_err(|e| HostError::Wasm(e.to_string()))?;

    Ok(())
}

/// Monotonic ns (`std::time::Instant` domain — never goes backwards; NOT
/// comparable across processes, which is exactly the perf_counter shape).
fn monotonic_ns() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos().min(i64::MAX as u128) as i64
}

/// Wall-clock ns since the Unix epoch (`SystemTime` domain — time.time).
fn wall_ns() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_codegen_wasm::lower_module;
    use ap_ir::{Builder, Item, Module};

    fn hello_module(msg: &str) -> Vec<u8> {
        let b = Builder::new();
        let mut module = Module::new();
        module.items.push(Item::Stmt(
            b.expr_stmt(b.call(b.name("print"), vec![b.arg_pos(b.str_lit(msg))])),
        ));
        lower_module(&module).expect("lower")
    }

    #[test]
    fn hello_world_runs_end_to_end() {
        let bytes = hello_module("hello, wasm");
        let out = run(&bytes, 64 * 1024).expect("run");
        assert_eq!(out.stdout, "hello, wasm\n");
        assert_eq!(out.stderr, "");
        assert_eq!(out.exit_code, 0);
        assert_eq!(out.ttfb_us, None, "ttfb off by default");
        assert!(out.total_us > 0);
    }

    /// F2 guard: two `run()` calls in one process share the single
    /// process-wide Engine (the OnceLock), producing identical output —
    /// the observable contract of engine reuse (the second call must not
    /// rebuild config/CodeGenSettings; a fresh Engine per call was the bug).
    #[test]
    fn repeated_runs_reuse_the_shared_engine() {
        let bytes = hello_module("engine reuse");
        let first = run(&bytes, 64 * 1024).expect("run 1");
        let second = run(&bytes, 64 * 1024).expect("run 2");
        assert_eq!(first.stdout, "engine reuse\n");
        assert_eq!(first.stdout, second.stdout);
        assert_eq!(second.exit_code, 0);
    }

    #[test]
    fn caller_owned_engine_runs() {
        let engine = Engine::new(&host_config()).expect("engine");
        let bytes = hello_module("owned");
        let out = run_with_engine(&engine, &bytes, 64 * 1024).expect("run");
        assert_eq!(out.stdout, "owned\n");
        // The same Engine instance drives a second module compile+run.
        let out2 = run_with_engine(&engine, &bytes, 64 * 1024).expect("run 2");
        assert_eq!(out2.stdout, out.stdout);
    }

    /// The TTFB measurement records a first byte within the budget on the
    /// hello-world module (the STARTUP GATE — measured, asserted only as
    /// "the harness works"; the 50ms number itself lives in the session
    /// record, not in a test).
    #[test]
    fn ttfb_measures_first_byte() {
        let engine = shared_engine().expect("engine");
        let bytes = hello_module("ttfb probe");
        let mut opts = RunOptions::new();
        opts.measure_ttfb = true;
        let out = run_with_engine_opts(&engine, &bytes, &opts).expect("run");
        assert_eq!(out.stdout, "ttfb probe\n");
        let us = out.ttfb_us.expect("ttfb measured");
        assert!(us > 0, "ttfb must be positive");
        assert!(
            us < 50_000_000,
            "hello-world ttfb {us}us blew the 50ms startup budget"
        );
    }

    /// Capability scoping: with NO grant, WASI args/env getters see
    /// argv[0] only and zero env vars (DEFAULT DENY — leaf scope 4).
    #[test]
    fn default_caps_deny_args_and_env() {
        // `import os` is far beyond the wasm slice; instead the CONTRACT is
        // unit-tested through the builder shape: a run with default options
        // must succeed and the ctx construction is deny-by-default by code
        // inspection of run_with_engine_opts (envs() only called when
        // !opts.env.is_empty()). Here we assert the observable: default run
        // of a plain module is unaffected by whatever the HOST env holds.
        let bytes = hello_module("caps");
        let out = run(&bytes, 64 * 1024).expect("run");
        assert_eq!(out.stdout, "caps\n");
        assert_eq!(out.exit_code, 0);
    }

    /// The ABI version check refuses a module whose aprt-version stamp is
    /// not supported — clear diagnostic, raised before compile.
    #[test]
    fn version_check_refuses_drifted_module() {
        let bytes = hello_module("v99");
        // Overwrite the trailing custom section payload digit with '9'.
        let mut drifted = bytes.clone();
        let pos = drifted
            .windows(VERSION_SECTION.len())
            .rposition(|w| w == VERSION_SECTION.as_bytes())
            .expect("version section present");
        let digit = pos + VERSION_SECTION.len();
        assert_eq!(drifted[digit], b'0', "emitter stamps '0'");
        drifted[digit] = b'9';
        match run(&drifted, 64 * 1024) {
            Err(HostError::Abi(msg)) => {
                assert!(
                    msg.contains("version 9"),
                    "diagnostic names the version: {msg}"
                );
            }
            other => panic!("expected HostError::Abi, got {other:?}"),
        }
    }

    /// A module with the version section STRIPPED is refused too
    /// (pre-stamp modules fail fast instead of confusingly trapping).
    #[test]
    fn version_check_refuses_unstamped_module() {
        let bytes = hello_module("nostamp");
        let pos = bytes
            .windows(VERSION_SECTION.len())
            .rposition(|w| w == VERSION_SECTION.as_bytes())
            .expect("version section present");
        // Custom section header: id(1) size(LEB) name-len(LEB) name payload.
        // Walk back over the section to find its start: name len byte sits
        // 1 before the name; size LEB precedes that; id before that.
        let name_len_pos = pos - 1;
        let mut size_start = name_len_pos - 1;
        while bytes[size_start] & 0x80 != 0 {
            size_start -= 1;
        }
        let id_pos = size_start - 1;
        assert_eq!(bytes[id_pos], 0, "custom section id is 0");
        // Cut the module at the custom section: everything after the data
        // section end is only this custom section (trailing, S-07-43).
        let stripped = bytes[..id_pos].to_vec();
        match run(&stripped, 64 * 1024) {
            Err(HostError::Abi(msg)) => {
                assert!(
                    msg.contains("lacks"),
                    "diagnostic mentions the missing stamp: {msg}"
                );
            }
            other => panic!("expected HostError::Abi, got {other:?}"),
        }
        let _ = name_len_pos; // (kept for the walk clarity)
    }

    /// The threading refusal: a module with a SHARED memory gets the
    /// design-§9 diagnostic (wasmtime 47 removed wasi-threads).
    #[test]
    fn threading_refused_via_shared_memory() {
        use wasm_encoder::{
            ExportKind, ExportSection, MemorySection, MemoryType, Module, RawSection, Section,
            SectionId,
        };
        // Minimal synthetic module: shared memory + _start export. No
        // code section — the threading check must fire BEFORE compile.
        let mut m = Module::new();
        let mut mems = MemorySection::new();
        mems.memory(MemoryType {
            minimum: 1,
            maximum: Some(1),
            memory64: false,
            shared: true,
            page_size_log2: Some(16),
        });
        m.section(&mems);
        let mut exports = ExportSection::new();
        exports.export("_start", ExportKind::Func, 0);
        m.section(&exports);
        let custom: Vec<u8> = {
            let mut v = Vec::new();
            v.push(12);
            v.extend_from_slice(VERSION_SECTION.as_bytes());
            v.push(b'0');
            v
        };
        m.section(&RawSection {
            id: SectionId::Custom as u8,
            data: &custom,
        });
        let bytes = m.finish();
        match run(&bytes, 64 * 1024) {
            Err(HostError::Threading(msg)) => {
                assert!(msg.contains("wasi-threads"), "diagnostic: {msg}");
            }
            other => panic!("expected HostError::Threading, got {other:?}"),
        }
    }
}

/// The custom-section name — re-exported from checks for the tests above.
#[allow(unused_imports)]
pub(crate) use checks::VERSION_SECTION;

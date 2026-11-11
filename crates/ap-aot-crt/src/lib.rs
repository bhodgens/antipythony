//! The AoT CRT (leaf 02.06, T3): the static library the system linker
//! combines with the program object into the final binary. This file IS
//! the generated entry point (§02.06 Scope 3):
//!
//! (a) initializes the GC surface (heap sizing per docs/gc-mmtk-swap.md
//!     §6 semantics — the generational cascade + growth-triggered major
//!     GC the runtime already ships; APX_* env knobs honored);
//! (b) registers the AoT manifest through ap-run's `aot_runtime` seam
//!     (root feeds + heap + dispatch ride the seams 02.04/02.05
//!     installed — run_module's own init path);
//! (c) wraps execution in catch_unwind as the SAFETY NET (pon
//!     aot_entry.rs:53 pattern): a Rust panic becomes our
//!     exception-format abort — NEVER a raw Rust panic print;
//! (d) runs the module body through ap_run::run_module (the exact
//!     interpreter entry `antipythony run` uses — byte-identity by
//!     construction with the tree-walker + tier-0 JIT shapes);
//! (e) maps exit code: run_module's outcome IS the exit code (sys.exit /
//!     uncaught / 130 mapped inside the runtime per S-06-11/C-29).
//!
//! Module-level function dispatch: registered AOT bodies are consulted
//! through ap-run's registry seam — the function's module-level def id
//! maps to a manifest entry (qualname "apfn_<name>"), and call_function's
//! tier dispatch arm prefers the AOT entry when present. Unregistered /
//! unsupported bodies tree-walk exactly as tier-0 does.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use std::io::Write;

// The manifest row layout — the ONE definition lives in
// ap-codegen-native/src/link.rs (MANIFEST_ROW_SIZE = 64, 8 u64 words);
// this table mirrors it with a static assert.
const MANIFEST_ROW_SIZE: usize = 64;

#[repr(C)]
struct ManifestRow {
    qualname_ptr: u64,
    qualname_len: u64,
    entry_ptr: u64,
    num_slots: u64,
    names_ptr: u64,
    names_len: u64,
    ic_ptr: u64,
    ic_len: u64,
}

// The linker resolves these against the program object's Export data.
extern "C" {
    static ap_aot_manifest: u8;
    static ap_aot_source: u8;
    /// u64 byte length of ap_aot_source + the "APSRCEND" sentinel (the
    /// length word lets the CRT read exactly the embedded program bytes).
    static ap_aot_source_len: u8;
}

/// Safety-net panic message (the exception-format abort, review box 4).
const PANIC_ABORT_MSG: &str = "antipythony: internal error (interpreter panic caught at the AoT entry boundary)\n";

/// `main` for the produced binary (the staticlib's single exported
/// entry; the linker's start files call it).
///
/// # Safety
/// Called by the C runtime start files; the process environment and the
/// manifest symbol are guaranteed by the link contract (ap_aot_prog.o
/// always defines ap_aot_manifest — even def-less programs emit a
/// zero-row manifest, see emit_program_object).
#[no_mangle]
pub extern "C" fn main() -> i32 {
    // (a) RSS self-guard FIRST (same 2026-11-06 OOM mitigation every
    // interpreter process installs — the binary is an interpreter
    // process).
    ap_run::rss_guard::start();

    // (b) manifest registration — BEFORE any module code runs, so the
    // dispatch arm sees the AOT entries from the first call.
    register_manifest();

    // (c)+(d) catch_unwind safety net around the run.
    let result = std::panic::catch_unwind(aot_run);
    match result {
        Ok(code) => code,
        Err(_payload) => {
            // Never a raw Rust panic print: the payload is dropped and a
            // single stderr line in OUR format goes out (pon aot_entry
            // discipline). Exit 134 = SIGABRT-shaped internal abort.
            let mut err = std::io::stderr();
            let _ = err.write_all(PANIC_ABORT_MSG.as_bytes());
            let _ = err.flush();
            134
        }
    }
}

/// The actual run: a dedicated big-stack thread (the interpreter consumes
/// ~50KB native stack per Python frame — recursion_limit 1000 needs the
/// same headroom `antipythony run` gives itself), then exit-code mapping.
fn aot_run() -> i32 {
    // argv: the binary's own argv — sys.argv inside the program is what
    // is given AFTER the binary name (argv[0] = the script path the
    // runtime reports: the binary's argv[0], matching CPython's
    // sys.argv[0] = script path convention for `./app` runs).
    let args: Vec<String> = std::env::args().collect();
    let script_path = args.first().cloned().unwrap_or_else(|| "app".to_string());

    let thread_args = (script_path.clone(), args.into_iter().skip(1).collect::<Vec<String>>());
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            ap_run::tier_flag::set("aot");
            ap_run::set_argv_tail(thread_args.1.clone());
            let mut out = std::io::stdout();
            let mut err = std::io::stderr();
            // The program body IS the embedded ap_aot_source blob (a
            // static binary carries its program; nothing is read from
            // disk). sys.argv[0] = the binary path (CPython's `./app`
            // convention).
            let s0 = std::ptr::addr_of!(ap_aot_source) as *const u8;
            let lenp = std::ptr::addr_of!(ap_aot_source_len) as *const u64;
            let len = unsafe { lenp.read_unaligned() } as usize;
            let source = unsafe {
                String::from_utf8_lossy(std::slice::from_raw_parts(s0, len)).into_owned()
            };
            let outcome = ap_run::run_module_embedded(
                &source,
                &thread_args.0,
                ap_run::aot_manifest_slice(),
                &mut out,
                &mut err,
            );
            let _ = out.flush();
            let _ = err.flush();
            outcome.exit_code
        })
        .expect("spawn interpreter thread")
        .join()
        .unwrap_or(134); // thread panic → the same internal-abort shape
    code
}

/// Walk the linked manifest and register every row through ap-run's
/// runtime seam. The walk reads ONLY layout-stable words; a count/size
/// drift would misalign the FIRST row — guarded by the static assert
/// above (the assembler's MANIFEST_ROW_SIZE is the same 64).
fn register_manifest() {
    // Manifest symbol address (relocated by the static linker).
    let base = std::ptr::addr_of!(ap_aot_manifest) as *const u64;
    let count = unsafe { base.read_unaligned() } as usize;
    let rows = unsafe { std::slice::from_raw_parts(base.add(1) as *const ManifestRow, count) };
    let mut entries = Vec::with_capacity(count);
    for row in rows {
        let qualname = unsafe {
            let p = row.qualname_ptr as *const u8;
            let n = row.qualname_len as usize;
            if p.is_null() || n == 0 {
                String::new()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(p, n)).into_owned()
            }
        };
        let names: Vec<String> = unsafe {
            let p = row.names_ptr as *const u8;
            let n = row.names_len as usize;
            let mut out = Vec::with_capacity(n);
            let mut cursor = 0usize;
            for _ in 0..n {
                let ptr = (p.add(cursor * 16) as *const u64).read_unaligned();
                let len = (p.add(cursor * 16 + 8) as *const u64).read_unaligned() as usize;
                let ptr = ptr as *const u8;
                if !ptr.is_null() && len > 0 {
                    out.push(
                        String::from_utf8_lossy(std::slice::from_raw_parts(ptr, len))
                            .into_owned(),
                    );
                } else {
                    out.push(String::new());
                }
                cursor += 1;
            }
            out
        };
        entries.push(ap_run::AotFnEntry {
            qualname,
            code_ptr: row.entry_ptr as *const u8,
            num_slots: row.num_slots as u32,
            name_table: names,
            ic_ptr: row.ic_ptr as *mut u8,
            ic_len: row.ic_len as u32,
        });
    }
    if std::env::var_os("APX_AOT_DEBUG").is_some() {
        eprintln!("[AOT-MANIFEST] {count} rows");
        for e in &entries {
            eprintln!(
                "[AOT-MANIFEST] {} slots={} names={:?} code={:p} ic_len={}",
                e.qualname,
                e.num_slots,
                e.name_table,
                e.code_ptr,
                e.ic_len
            );
        }
    }
    ap_run::aot_set_manifest(entries);
}

// Keep the row-size assert compile-visible.
const _: () = assert!(
    MANIFEST_ROW_SIZE == std::mem::size_of::<ManifestRow>(),
    "CRT manifest row layout must match link.rs MANIFEST_ROW_SIZE (64)"
);

// staticlib: keep the panic machinery + std formatting reachable (the
// catch_unwind net needs panic::recover paths; -dead_strip trims the rest).
#[used]
static _KEEP_PANIC: fn() = keep_panic;
fn keep_panic() {
    std::process::abort();
}

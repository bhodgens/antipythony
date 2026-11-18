//! Module-surface validation (leaf 03.05): the aprt ABI version stamp, the
//! threading diagnostic, and the import-surface allowlist — all checked on
//! the RAW bytes BEFORE wasmtime compiles, so ABI drift and unsupported
//! features fail fast with a clear diagnostic instead of a wasmtime
//! instantiate-time trap.
//!
//! Version stamp: the emitter writes a trailing custom section
//! `aprt-version` whose payload is one ASCII digit (the ABI version).
//! ap-host refuses modules from an INCOMPATIBLE version (future major
//! drift) — old modules fail fast on ABI drift, never trap confusingly.
//!
//! Threading: wasmtime 47 removed wasi-threads (design §9). A module that
//! requests shared memory (the thread-proposal prerequisite) gets a clear
//! compile-time-style diagnostic — same treatment as C-extensions in v1.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::HostError;

/// The aprt host-module ABI versions this host understands. The emitter
/// stamps `0` (crates/ap-codegen-wasm lower.rs custom section). A module
/// stamped with a version NOT in this list is refused before compile.
pub const SUPPORTED_APRT_ABI_VERSIONS: &[u32] = &[0];

/// The custom-section name the emitter stamps (S-07-43 section order:
/// custom(`aprt-version`) is the trailing section).
pub(crate) const VERSION_SECTION: &str = "aprt-version";

/// The finalized aprt import surface (design §7, leaf 03.05): the bignum/
/// format/repr helpers are deliberately NOT host imports (they are in-module
/// prelude code — design §4 decision, recorded deviation from the
/// orchestrator text). Everything the module may import from "aprt" is here.
const KNOWN_APRT_IMPORTS: &[&str] = &["abort_msg", "now_ns", "wall_ns"];

/// The only non-aprt import namespace modules may carry. WASI p1 surface:
/// stdio (fd_write/fd_read), proc_exit, args/env getters, and the WASI
/// clocks (wasmtime-wasi p1 provides clock_time_get).
const KNOWN_IMPORT_MODULES: &[&str] = &["aprt", "wasi_snapshot_preview1"];

/// Validate the module surface on raw bytes: ABI version stamp, threading
/// (shared memory), and the import namespace allowlist.
///
/// Runs before [`crate::run_with_options`] compiles anything, so a drifted
/// module fails with THIS message, not a wasmtime import-resolution trap.
pub fn validate_module_surface(wasm_bytes: &[u8]) -> Result<(), HostError> {
    use wasmparser::{Parser, Payload};

    let mut stamped_version: Option<u32> = None;
    let mut shared_memory = false;

    let parser = Parser::new(0);
    for payload in parser.parse_all(wasm_bytes) {
        let payload = payload.map_err(|e| HostError::Wasm(format!("module parse error: {e}")))?;
        match payload {
            Payload::CustomSection(ref c) if c.name() == VERSION_SECTION => {
                // Emitter payload: one ASCII digit ('0' + version).
                stamped_version = c.data().first().map(|b: &u8| (b.wrapping_sub(b'0')) as u32);
            }
            Payload::ImportSection(r) => {
                for group in r {
                    let group =
                        group.map_err(|e| HostError::Wasm(format!("import section error: {e}")))?;
                    match group {
                        wasmparser::Imports::Single(_, imp) => {
                            check_import(imp.module, imp.name)?;
                        }
                        wasmparser::Imports::Compact1 { module, items } => {
                            for item in items {
                                let item = item.map_err(|e| {
                                    HostError::Wasm(format!("import section error: {e}"))
                                })?;
                                check_import(module, item.name)?;
                            }
                        }
                        wasmparser::Imports::Compact2 { module, names, .. } => {
                            for name in names {
                                let name = name.map_err(|e| {
                                    HostError::Wasm(format!("import section error: {e}"))
                                })?;
                                check_import(module, name)?;
                            }
                        }
                    }
                }
            }
            Payload::MemorySection(r) => {
                for mem in r {
                    let mem =
                        mem.map_err(|e| HostError::Wasm(format!("memory section error: {e}")))?;
                    if mem.shared {
                        shared_memory = true;
                    }
                }
            }
            _ => {}
        }
    }

    if shared_memory {
        return Err(HostError::Threading(
            "module requests shared memory (the wasi-threads prerequisite); \
             wasmtime 47 removed wasi-threads — Plan 03 v1 is single-threaded \
             (design §9); `threading` is a clear error in wasm mode, the \
             shared-everything-threads spike is leaf 03.06"
                .to_string(),
        ));
    }

    match stamped_version {
        None => Err(HostError::Abi(
            "module lacks the `aprt-version` custom section — it predates the \
             versioned aprt ABI stamp (leaf 03.05) and is refused: recompile \
             it with the current ap-codegen-wasm emitter"
                .to_string(),
        )),
        Some(v) if SUPPORTED_APRT_ABI_VERSIONS.contains(&v) => Ok(()),
        Some(v) => Err(HostError::Abi(format!(
            "module aprt ABI version {v} is not supported by this host \
             (supported: {SUPPORTED_APRT_ABI_VERSIONS:?}) — the module was \
             built by a NEWER/DIFFERENT emitter; refusing before instantiate \
             so old modules fail fast on ABI drift"
        ))),
    }
}

/// Import namespace allowlist: only `wasi_snapshot_preview1` and `aprt`
/// (with the finalized §7 surface). Unknown aprt imports are an ABI drift
/// signal — clear diagnostic, not an instantiate-time unknown-import trap.
fn check_import(module: &str, name: &str) -> Result<(), HostError> {
    if !KNOWN_IMPORT_MODULES.contains(&module) {
        return Err(HostError::Abi(format!(
            "module imports '{module}::{name}' — not in the finalized import \
             surface (allowed: {KNOWN_IMPORT_MODULES:?}); refusing before \
             instantiate"
        )));
    }
    if module == "aprt" && !KNOWN_APRT_IMPORTS.contains(&name) {
        return Err(HostError::Abi(format!(
            "module imports 'aprt::{name}' — unknown aprt symbol (finalized \
             surface: {KNOWN_APRT_IMPORTS:?}); ABI drift, refusing before \
             instantiate"
        )));
    }
    Ok(())
}

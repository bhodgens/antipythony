//! ap-ir to WasmGC module (Plan 03 skeleton — leaf 03.01/03.02 land the
//! GC representation proper; see docs/03-wasm-representation-design.md).
//!
//! This bootstrap proves the ap-ir → wasm bytes → ap-host pipeline: a
//! module-level `print("...")` lowers to a WASI `fd_write` call. Everything
//! else is an explicit [`CodegenError::Unsupported`] — the skeleton never
//! silently miscompiles.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

pub mod emitter_state;
pub mod lower;
pub mod prelude;
pub mod types;

pub use lower::{lower_module, CodegenError};

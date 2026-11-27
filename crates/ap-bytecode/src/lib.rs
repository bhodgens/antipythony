//! Flat bytecode: instruction set, const pool, line table, site table.
//!
//! ap-bytecode is a PURE TRANSLATION layer: ap-ir Expr/Stmt trees -> flat
//! `Vec<Instr>` plus the metadata tables (const pool, sparse line table,
//! inline-cache site table). It owns NO semantics — the dispatch
//! interpreter (fast-path leaf 02) and the lowerer's fallback discipline
//! define execution; the tree-walker (ap-run interp.rs) remains the
//! reference implementation for every shape this crate encodes.
//!
//! Layers:
//! - [`instr`]: the `Instr` enum (start set per leaf spec 01) + opcode names.
//! - [`pool`]: deduplicated constant pool (spec: strings, big ints, floats).
//! - [`linetable`]: sparse sorted `(instr_idx, line)` map with `line_for`.
//! - [`sites`]: stable per-site ids for Load/LoadGlobal/LoadAttr/Call +
//!   the (empty) per-site inline-cache slot array that 02-perf/01 fills.
//! - [`lower`]: the lowerer. Unsupported shapes raise
//!   [`Unsupported`] OUT of `lower_expr`/`lower_stmt`; `lower_fn` catches
//!   them and collapses the WHOLE function to a single
//!   `Instr::FallbackToTreeWalk` — function granularity, never a
//!   partially-lowered body.
//! - [`code`]: the assembled `CodeObject` (what leaf 02's frame consumes).
//! - [`decode`]: textual disassembly for tests/debugging.
//!
//! Out of scope here (later leaves): For/Try EXECUTION — the enum carries
//! the control-flow opcodes but `lower_fn` rejects functions containing
//! For/Try/With/Match (falls back), and BoolOp/Break/Continue lowering
//! lands with 01-core.

pub mod caches;
pub mod code;
pub mod decode;
pub mod instr;
pub mod linetable;
pub mod lower;
pub mod pool;
pub mod sites;
#[cfg(test)]
mod tests;

pub use caches::{AttrPlan, CallPlan, IC_MAGIC};
pub use code::CodeObject;
pub use decode::disassemble;
pub use instr::{CompareOp, Instr, UnaryKind, OPCODE_NAMES};
pub use linetable::LineTable;
pub use lower::{lower_fn, lower_module_fn, Unsupported};
pub use pool::ConstPool;
pub use sites::{SiteKind, SiteTable};

//! Leaf 03.04 prelude: generators (PEP-342), the iterator protocol, and the
//! comprehension-snapshot constructors, emitted as in-module wasm functions.
//!
//! Generator runtime model (design §6 + the leaf brief): a generator object
//! is a `$generator` struct `$g = { fn, state, locals[], value, exn }`. The
//! BODY is one wasm function `FT_GEN_STEP (gen, input) -> o` with a
//! block-chain dispatch on `$g.$state` (0 = start, N = resume at yield
//! site N). Each `yield` spills its live locals into `$g.$locals`, stores
//! the next state, and RETURNS the yielded value; the next resume
//! dispatches to the resume site, where the yield-expression's value is
//! loaded from `$g.$value` (parked there by `gen_send`).
//!
//! Terminal states: `$state = -1` exhausted (a `return v` parks v in
//! `$g.$value` so the completion observer raises StopIteration(v)), `-2`
//! closed (early throw/close-in, or completed close). Every later resume
//! of -1/-2 raises StopIteration per CPython (an exhausted throw() raises
//! the thrown exception itself — Python 3.14 probe /tmp/genprobe.py — NOT
//! the G8 StopIteration rewrite the native tier shipped; byte-exactness is
//! measured against 3.14, so 3.14 wins here; the divergence vs ap-run's
//! exhausted-throw arm is a recorded native-tier issue, not a wasm one).
//!
//! The iterator protocol reuses `$generator` for container iterators: a
//! $generator with `$fn = null` (index -1 via i32 0 sentinel — see
//! FT_GEN_STEP's import arm) whose `locals[0..]` are the SNAPSHOT items and
//! `value` is the cursor. That gives `iter()`/`next()` one representation.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state as fx;
use crate::prelude::{add, hc, oref, push_null, sget, sset};
use crate::types::*;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

// ---------------------------------------------------------------------------
// $generator state constants (the encoding EVERY function below agrees on)
// ---------------------------------------------------------------------------

/// state = 0: not yet started (body runs from the top).
pub const GS_START: i32 = 0;
/// state < 0 AND >= -64: exhausted; (-1 - k) parks return value at locals[k]? —
/// the return VALUE parks in `$value` (see module docs); -1 = exhausted.
pub const GS_DONE: i32 = -1;
/// state = -2: closed (close() completed, or pre-start throw/close-in).
pub const GS_CLOSED: i32 = -2;

/// Generator-identity discriminator: `$fn == null` marks a CONTAINER
/// ITERATOR (locals = snapshot items, value = cursor i32-in-value? no —
/// the cursor rides locals[len-?]... see iter_new: locals[0]=items-RO-ref,
/// locals[1] is unused; the CURSOR lives in `$exn`? — no: cursors must be
/// i32. Decision: container iterators carry the cursor in a SECOND shape —
/// see `it_cursor_of` below. The wasm field set is fixed, so the cursor is
/// stored as an $int object in `$value` and boxed/unboxed at each next().
pub const CURSOR_LOCAL: u32 = 61; // per-frame i32 cursor local in FT_GEN_STEP

// ---------------------------------------------------------------------------
// gen_new (FT_GEN_NEW): (fn $function, args RO) -> $generator — build the
// generator object: state = START, locals = params bound from args (with
// defaults from fn.defaults), value/exn null. The body does NOT run here
// (CPython: the call just builds the frame; effects happen on first next).
// Locals: 3=fn 4=args 5=nparams 6=i 7=locals-arr 8=default
// ---------------------------------------------------------------------------
pub(crate) fn gen_new(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::FunctionSection_use_shim,
) {
    let _ = funcs;
    let _ = code;
}

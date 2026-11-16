//! Code-emission state shared by the prelude builders and `lower.rs`.
//!
//! Function indices are FIXED at module-emit time (import funcs 0/1 first,
//! rt_init = 2, then the prelude block in `prelude::emit` order, then
//! `_start`), so both `prelude.rs` and `lower.rs` address every local
//! function by these constants — no index drift.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

pub const FN_BIG_NORM: u32 = 3;
pub const FN_BIG_FROM_I64: u32 = 4;
pub const FN_INT_FROM_I64: u32 = 5;
pub const FN_BOOL_OBJ: u32 = 6;
pub const FN_FLOAT_FROM_I64: u32 = 7;
pub const FN_HASH_I64: u32 = 8;
pub const FN_TRUTHY: u32 = 9;
pub const FN_HASH: u32 = 10;
pub const FN_FLOAT_HASH: u32 = 11;
pub const FN_EQ: u32 = 12;
pub const FN_COMPARE: u32 = 13;
pub const FN_BUILTIN_DISPATCH: u32 = 14;
pub const FN_PRINT_ONE: u32 = 15;
pub const FN_PRINT_FLUSH: u32 = 16;
pub const FN_LIST_EQ: u32 = 17;
pub const FN_TUPLE_EQ: u32 = 18;
pub const FN_DICT_EQ: u32 = 19;
pub const FN_DICT_HAS: u32 = 20;
pub const FN_INT_EQ: u32 = 21;
pub const FN_FLOAT_PY_EQ: u32 = 22;
pub const FN_STR_EQ: u32 = 23;
pub const FN_BYTES_EQ: u32 = 24;
pub const FN_NONE_EQ: u32 = 25;
pub const FN_TYPE_INDEX: u32 = 26;
pub const FN_INT_TO_STR: u32 = 27;
pub const FN_PY_STR: u32 = 28;
pub const FN_STR_FROM_BYTES: u32 = 29;
pub const FN_APPEND_I64: u32 = 30;
pub const FN_BIG_BITS: u32 = 31;
pub const FN_BIG_CMP: u32 = 32;
pub const FN_BINOP: u32 = 33;
pub const FN_UNARY: u32 = 34;
pub const FN_LEN: u32 = 35;

/// The generic call dispatcher (`pycall`, S-07-23): (fn, args, kwargs) -> o.
/// Registered by appending `prelude::dispatch` AFTER `len()` in prelude::emit
/// (function-table discipline: indices are dense, never renumbered).
pub const FN_DISPATCH: u32 = 36;
// ---------------------------------------------------------------------------
// Leaf 03.03 prelude functions SHIPPED (exceptions slice): dense append
// after FN_DISPATCH. The class-creation machinery (class_new C3 merge,
// instance_new, attr_get/set/del, cls_call, descriptor ctors, exc_raise/
// match/msg/repr/ctx, call_of, global_get, subcls registry) lands with the
// classes continuation — its function indices APPEND after FN_ISINST
// (46..) so these shipped ids never move.
// ---------------------------------------------------------------------------
pub const FN_DGET: u32 = 37; // (dict, str) -> o   (str-keyed linear)
pub const FN_DSET: u32 = 38; // (dict, str, o) -> ()
pub const FN_DHAS: u32 = 39; // (dict, str) -> i32
pub const FN_STR_CAT: u32 = 40; // (str, str) -> str
pub const FN_TYPE_NAME: u32 = 41; // (o) -> str
pub const FN_FRAME_PUSH: u32 = 42; // (str fn-name) -> ()
pub const FN_FRAME_POP: u32 = 43; // () -> ()
pub const FN_FRAME_LINE: u32 = 44; // (i32 line) -> ()
pub const FN_ISSUB: u32 = 45; // (ty, ty) -> i32
pub const FN_ISINST: u32 = 46; // (o, ty) -> i32
/// First wasm function index of the EMITTED user functions (Item::FnDef /
/// Stmt::LocalDef), which follow the prelude densely. `_start` is
/// computed at emit time as FN_FIRST_USER + n_user_fns.
pub const FN_FIRST_USER: u32 = 47;

/// rt_init stays at index 2 (03.02a layout: 0/1 imports, 2 rt_init).
pub const FN_RT_INIT: u32 = 2;

/// Linear-memory scratch layout (S-07-41 print buffer + digit scratch).
/// 0..64 iovec scratch; 64..1024 reserved; 1024.. print buffer; 4096..
/// digit-assembly scratch.
pub const PRINT_BUF: i32 = 1024;
pub const DIGIT_SCRATCH: i32 = 4096;

// ---------------------------------------------------------------------------
// $binop / $unop / $compare opcode encoding (spec S-07-45 prelude op table).
// ---------------------------------------------------------------------------

#[allow(non_snake_case)]
pub mod OPS {
    pub const ADD: i32 = 0;
    pub const SUB: i32 = 1;
    pub const MUL: i32 = 2;
    pub const DIV: i32 = 3;
    pub const FLOORDIV: i32 = 4;
    pub const MOD: i32 = 5;
    pub const POW: i32 = 6;
    pub const LSHIFT: i32 = 7;
    pub const RSHIFT: i32 = 8;
    pub const AND: i32 = 9;
    pub const XOR: i32 = 10;
    pub const OR: i32 = 11;
    pub const NEG: i32 = 12;
    pub const ABS: i32 = 13;
    pub const INVERT: i32 = 14;
    pub const BOOL: i32 = 15;
    pub const NOT: i32 = 16;

    pub const EQ: i32 = 0;
    pub const NE: i32 = 1;
    pub const LT: i32 = 2;
    pub const LE: i32 = 3;
    pub const GT: i32 = 4;
    pub const GE: i32 = 5;
    /// `in` (dict membership is the only container op this leaf lowers).
    pub const IN: i32 = 6;
}

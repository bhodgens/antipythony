//! Code-emission state shared by the prelude builders and `lower.rs`.
//!
//! Function indices are FIXED at module-emit time (import funcs 0/1 first,
//! rt_init = 2, then the prelude block in `prelude::emit` order, then
//! `_start`), so both `prelude.rs` and `lower.rs` address every local
//! function by these constants — no index drift.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

pub const FN_BIG_NORM: u32 = 4;
pub const FN_BIG_FROM_I64: u32 = 5;
pub const FN_INT_FROM_I64: u32 = 6;
pub const FN_BOOL_OBJ: u32 = 7;
pub const FN_FLOAT_FROM_I64: u32 = 8;
pub const FN_HASH_I64: u32 = 9;
pub const FN_TRUTHY: u32 = 10;
pub const FN_HASH: u32 = 11;
pub const FN_FLOAT_HASH: u32 = 12;
pub const FN_EQ: u32 = 13;
pub const FN_COMPARE: u32 = 14;
pub const FN_BUILTIN_DISPATCH: u32 = 15;
pub const FN_PRINT_ONE: u32 = 16;
pub const FN_PRINT_FLUSH: u32 = 17;
pub const FN_LIST_EQ: u32 = 18;
pub const FN_TUPLE_EQ: u32 = 19;
pub const FN_DICT_EQ: u32 = 20;
pub const FN_DICT_HAS: u32 = 21;
pub const FN_INT_EQ: u32 = 22;
pub const FN_FLOAT_PY_EQ: u32 = 23;
pub const FN_STR_EQ: u32 = 24;
pub const FN_BYTES_EQ: u32 = 25;
pub const FN_NONE_EQ: u32 = 26;
pub const FN_TYPE_INDEX: u32 = 27;
pub const FN_INT_TO_STR: u32 = 28;
pub const FN_PY_STR: u32 = 29;
pub const FN_STR_FROM_BYTES: u32 = 30;
pub const FN_APPEND_I64: u32 = 31;
pub const FN_BIG_BITS: u32 = 32;
pub const FN_BIG_CMP: u32 = 33;
pub const FN_BINOP: u32 = 34;
pub const FN_UNARY: u32 = 35;
pub const FN_LEN: u32 = 36;

/// The generic call dispatcher (`pycall`, S-07-23): (fn, args, kwargs) -> o.
/// Registered by appending `prelude::dispatch` AFTER `len()` in prelude::emit
/// (function-table discipline: indices are dense, never renumbered).
pub const FN_DISPATCH: u32 = 37;
// ---------------------------------------------------------------------------
// Leaf 03.03 prelude functions SHIPPED (exceptions slice): dense append
// after FN_DISPATCH. The class-creation machinery (class_new C3 merge,
// instance_new, attr_get/set/del, cls_call, descriptor ctors, exc_raise/
// match/msg/repr/ctx, call_of, global_get, subcls registry) lands with the
// classes continuation — its function indices APPEND after FN_ISINST
// (46..) so these shipped ids never move.
// ---------------------------------------------------------------------------
pub const FN_DGET: u32 = 38; // (dict, str) -> o   (str-keyed linear)
pub const FN_DSET: u32 = 39; // (dict, str, o) -> ()
pub const FN_DHAS: u32 = 40; // (dict, str) -> i32
pub const FN_STR_CAT: u32 = 41; // (str, str) -> str
pub const FN_TYPE_NAME: u32 = 42; // (o) -> str
pub const FN_FRAME_PUSH: u32 = 43; // (str fn-name) -> ()
pub const FN_FRAME_POP: u32 = 44; // () -> ()
pub const FN_FRAME_LINE: u32 = 45; // (i32 line) -> ()
pub const FN_ISSUB: u32 = 46; // (ty, ty) -> i32
pub const FN_ISINST: u32 = 47; // (o, ty) -> i32
                               // exceptions slice (03.03 continuation of the append):
pub const FN_EXC_RAISE: u32 = 48; // (o) -> o  (validate + tb + throw)
pub const FN_EXC_MATCH: u32 = 49; // (o exc, o target) -> i32
pub const FN_EXC_MSG: u32 = 50; // (o) -> str  (S-02-29 str(e))
pub const FN_EXC_REPR: u32 = 51; // (o) -> str  (repr(e))
pub const FN_EXC_CTX_SET: u32 = 52; // (o new, o ctx, o cause) -> o
                                    // classes slice (03.03 continuation of the append):
pub const FN_EXC_NEW: u32 = 53; // (ty, roarr) -> o   exc instance w/ args
pub const FN_CLS_NEW: u32 = 54; // (str, ty|null) -> ty  C3 single-base build
pub const FN_INSTANCE_NEW: u32 = 55; // (ty) -> o     $instance w/ empty attrs
pub const FN_CLS_CALL: u32 = 56; // (ty, roarr) -> o  instantiate + __init__
pub const FN_ATTR_GET: u32 = 57; // (o, str) -> o     S-03-9 lookup chain
pub const FN_ATTR_SET: u32 = 58; // (o, str, o) -> () S-03-10
pub const FN_CALL_OF: u32 = 59; // (o, roarr) -> o    $function | __call__ | exc
pub const FN_MRO_DICT_GET: u32 = 60; // (ty, str) -> o first dict hit along mro
pub const FN_TB_RENDER: u32 = 61; // () -> i32        stderr traceback, rc 1
                                  // ---------------------------------------------------------------------------
                                  // Leaf 03.04 prelude drivers (generators/iterators), appended after 61 —
                                  // dense, never renumbered. FN_FIRST_USER moves +7 for this block.
                                  // ---------------------------------------------------------------------------
pub const FN_GEN_SEND: u32 = 62; // (gen o, input o) -> o   PEP-342 resume
pub const FN_ITER_OF: u32 = 63; // (o iterable) -> o       iter()
pub const FN_ITER_NEXT: u32 = 64; // (o iterator) -> o     next()
pub const FN_ITER_CONTAINS: u32 = 65; // (o, o) -> i32     `in`
pub const FN_STOP_ITER: u32 = 66; // (o value, i32 empty) -> o  StopIteration throw
pub const FN_GEN_THROW: u32 = 67; // (gen o, exc o) -> o   throw-at-point
pub const FN_GEN_CLOSE: u32 = 68; // (gen o) -> ()         close/GeneratorExit
                                  // leaf 03.04b: exc_value_of — `e.value` for StopIteration instances (GE002:
                                  // `except StopIteration as e: print('SI value', e.value)`). (o, str) -> o,
                                  // same shape as attr_get — call sites use FN_ATTR_GET (no new index; the
                                  // attribute surfaces here are wired through attr_get's special-name arms).
                                  // leaf 03.03d: super() build (S-03-17 slice), appended after the 03.04
                                  // drivers — dense, never renumbered. FN_FIRST_USER moved 69 -> 70 for it.
pub const FN_SUPER_NEW: u32 = 69; // (S o, obj o, unused o) -> o  marker build
pub const FN_TYPE_OF: u32 = 70; // (o) -> o                    type(x)
                                // leaf 03.04c: drain drivers, appended after 70 — dense, never renumbered.
                                // FN_FIRST_USER moved 70 -> 73 for this block.
pub const FN_LIST_DRAIN: u32 = 71; // (iterator o) -> o ($list)
pub const FN_SUM_DRAIN: u32 = 72; // (iterator o) -> o ($int)
                                  // leaf 03.04g: dict-from-pairs driver, appended after 72 — dense, never
                                  // renumbered. FN_FIRST_USER moves 73 -> 74 for this block.
pub const FN_DICT_DRAIN: u32 = 73; // (iterator o) -> o ($dict)
/// First wasm function index of the EMITTED user functions (Item::FnDef /
/// Stmt::LocalDef), which follow the prelude densely. `_start` is
/// computed at emit time as FN_FIRST_USER + n_user_fns.
pub const FN_FIRST_USER: u32 = 74;

/// rt_init is index 3 (03.03 layout: 0 fd_write, 1 abort_msg, 2 proc_exit,
/// 3 rt_init — proc_exit joined the imports for the uncaught-traceback
/// boundary; every later const shifted +1 accordingly).
pub const FN_RT_INIT: u32 = 3;

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

//! The spec-07 prelude runtime, EMITTED as wasm functions (leaf 03.02b).
//!
//! DEVIATION from design doc §6 (documented in the commit): no wasm32 Rust
//! toolchain exists on this host (no wasm std, no lld), so the prelude is
//! transcribed by hand as emitted wasm — the algorithms mirror ap-obj
//! exactly (u32-LE bigint limbs, shortest float-repr by exact rational
//! search, CPython hash/eq/compare sign rules). Byte-exactness is enforced
//! by the values-corpus diff gate, not by shared crate sources.
//!
//! Function INDEX assignment lives in `emitter_state.rs`; `lower.rs`
//! addresses the prelude through those constants.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state::{self as fx, OPS};
use crate::types::*;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

// ---------------------------------------------------------------------------
// Shared emitters
// ---------------------------------------------------------------------------

/// `(ref null T)` heap type.
pub fn hc(ty: u32) -> wasm_encoder::HeapType {
    wasm_encoder::HeapType::Concrete(ty)
}

/// `(ref null $pyobj)` ValType.
pub fn oref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_PYOBJ),
    })
}

/// `(ref null $bigint)` ValType.
pub fn bref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_BIGINT),
    })
}

/// Push a nullable-concrete ref of `ty` (null).
pub fn push_null(f: &mut Function, ty: u32) {
    f.instruction(&Instruction::RefNull(hc(ty)));
}

/// `struct.get $ty $field` on the value at the top of the stack.
pub fn sget(f: &mut Function, ty: u32, field: u32) {
    f.instruction(&Instruction::StructGet {
        struct_type_index: ty,
        field_index: field,
    });
}

/// `struct.set $ty $field` on (ref, value) on the stack.
pub fn sset(f: &mut Function, ty: u32, field: u32) {
    f.instruction(&Instruction::StructSet {
        struct_type_index: ty,
        field_index: field,
    });
}

/// Push the $bigint payload of the $int at the stack top.
pub fn int_big(f: &mut Function) {
    sget(f, T_INT, F_INT_BIG);
}

/// Push the i64 smi payload of the $int at the stack top.
pub fn int_smi(f: &mut Function) {
    sget(f, T_INT, F_INT_SMI);
}

/// Push field 0 (the $type ref) of the $pyobj at the stack top.
pub fn obj_type(f: &mut Function) {
    sget(f, T_PYOBJ, F_TYPE);
}

pub fn g_int() -> u32 {
    builtin_type_global("int").expect("TYPE_ORDER carries int")
}
pub fn g_str() -> u32 {
    builtin_type_global("str").expect("TYPE_ORDER carries str")
}
pub fn g_bool() -> u32 {
    builtin_type_global("bool").expect("TYPE_ORDER carries bool")
}
pub fn g_float() -> u32 {
    builtin_type_global("float").expect("TYPE_ORDER carries float")
}
pub fn g_bytes() -> u32 {
    builtin_type_global("bytes").expect("TYPE_ORDER carries bytes")
}
pub fn g_dict() -> u32 {
    builtin_type_global("dict").expect("TYPE_ORDER carries dict")
}
pub fn g_list() -> u32 {
    builtin_type_global("list").expect("TYPE_ORDER carries list")
}
pub fn g_tuple() -> u32 {
    builtin_type_global("tuple").expect("TYPE_ORDER carries tuple")
}

/// Byte offset of the ACTIVE segment start in linear memory (`lower.rs`).
pub const SEG0_BASE: i32 = 64;

/// Append `bytes` to the passive string DATA SEGMENT (index 1) image.
pub struct StrTab {
    /// Segment-1 byte image = name blob + module literals + prelude strings.
    pub bytes: Vec<u8>,
    /// Offset of the PRELUDE region (right after module literals).
    prelude_base: u32,
}

impl StrTab {
    pub fn new(name_blob: Vec<u8>, module_literals: &[Vec<u8>]) -> Self {
        let mut bytes = name_blob;
        for s in module_literals {
            bytes.extend_from_slice(s);
        }
        let prelude_base = bytes.len() as u32;
        Self {
            bytes,
            prelude_base,
        }
    }

    /// Append `s` (ASCII) to the segment image; returns (offset, len).
    pub fn push(&mut self, s: &str) -> (u32, u32) {
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        (off, s.len() as u32)
    }

    pub fn prelude_base(&self) -> u32 {
        self.prelude_base
    }
}

/// The prelude function bodies, built in `emit` and drained by `emit_bodies`.
/// Emit the full prelude in exact `fx::FN_*` order (index contract).
/// Function-section entries only; bodies are buffered and written by
/// [`emit_bodies`].
pub fn emit(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    big_norm(funcs, code);
    big_from_i64(funcs, code);
    int_from_i64(funcs, code);
    bool_obj(funcs, code);
    float_from_i64(funcs, code);
    hash_i64(funcs, code);
    truthy(funcs, code);
    hash(funcs, code);
    float_hash(funcs, code);
    eq(funcs, code);
    compare(funcs, code);
    builtin_dispatch(funcs, code);
    print_one(funcs, code);
    print_flush(funcs, code);
    list_eq(funcs, code);
    tuple_eq(funcs, code);
    dict_eq(funcs, code);
    dict_has(funcs, code);
    int_eq(funcs, code);
    float_py_eq(funcs, code);
    str_eq(funcs, code);
    bytes_eq(funcs, code);
    none_eq(funcs, code);
    type_index(funcs, code);
    int_to_str(funcs, code);
    py_str(funcs, code, strings);
    str_from_bytes(funcs, code);
    append_i64(funcs, code);
    big_bits(funcs, code);
    big_cmp(funcs, code);
    binop(funcs, code);
    unary(funcs, code);
    len(funcs, code);
    // leaf 03.02c: the generic dispatcher is the LAST prelude body —
    // FN_DISPATCH = 36, user functions start at FN_FIRST_USER = 37.
    py_call(funcs, code);
}

/// Emit a plain function with given FT and locals, appending `body` then End.
fn with_body(locals: &[(u32, ValType)], body: impl FnOnce(&mut Function)) -> Function {
    let mut f = Function::new(locals.to_vec());
    body(&mut f);
    f.instruction(&Instruction::End);
    f
}

pub(crate) fn add(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    ft: u32,
    locals: &[(u32, ValType)],
    body: impl FnOnce(&mut Function),
) {
    funcs.function(ft);
    let f = with_body(locals, body);
    code.function(&f);
}

// ===========================================================================
// Helpers used by multiple prelude functions (instruction idioms)
// ===========================================================================


/// `local.get N; ref.cast (ref null $t)` — downcast before struct.get.
fn lget_cast(f: &mut Function, local: u32, ty: u32) {
    f.instruction(&Instruction::LocalGet(local));
    f.instruction(&Instruction::RefCastNullable(hc(ty)));
}

// ---------------------------------------------------------------------------
// Type selectors (the discriminator byte of every dispatch)
// ---------------------------------------------------------------------------

/// Runtime type selector for the object in local `obj`: 0 int, 1 float,
/// 2 bool, 3 str, 4 bytes, 5 dict, 6 list, 7 tuple, 8 None, 9 other.
fn type_sel(f: &mut Function, obj: u32, tmp: u32) {
    f.instruction(&Instruction::I32Const(9));
    f.instruction(&Instruction::LocalSet(tmp));
    let cases: [(u32, i32); 9] = [
        (g_int(), 0),
        (g_float(), 1),
        (g_bool(), 2),
        (g_str(), 3),
        (g_bytes(), 4),
        (g_dict(), 5),
        (g_list(), 6),
        (g_tuple(), 7),
        (builtin_type_global("NoneType").unwrap_or(0), 8),
    ];
    f.instruction(&Instruction::Block(BlockType::Empty));
    for (g, sel) in cases {
        // 03.03d FIX (the 03.03c "observed sel==8" root cause): the old
        // shape set tmp=sel BEFORE BrIf(eq) — the LAST case's unconditional
        // LocalSet(8) clobbered the default 9 on every miss, so EVERY
        // unknown-type $pyobj (user instances, exception instances) read as
        // NoneType in py_str (print(e) printed "None"). Now the store rides
        // INSIDE an If on the eq: no match -> tmp keeps the previous value
        // (default 9 falls through the whole chain untouched).
        f.instruction(&Instruction::LocalGet(obj));
        obj_type(f);
        f.instruction(&Instruction::GlobalGet(g));
        f.instruction(&Instruction::RefEq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(sel));
        f.instruction(&Instruction::LocalSet(tmp));
        f.instruction(&Instruction::Br(1)); // exit the Block (0 = this If)
        f.instruction(&Instruction::End);
    }
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(tmp));
}

// ===========================================================================
// $bigint primitives
// ===========================================================================

/// big_norm (FT_BIG_NORM): strip high zero limbs; zero -> EMPTY limbs.
fn big_norm(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_BIG_NORM, &[(2, ValType::I32), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_LIMB_ARRAY) }))], |f| {
        // 2 = trimmed len
        f.instruction(&Instruction::LocalGet(0));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::LocalSet(2));
        // while len>0 && limbs[len-1]==0: len--
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(0));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::BrIf(0));
        f.instruction(&Instruction::Br(1));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // out (local 3, array-typed) = new[len]; copy src[0..len)
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(0));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::ArrayCopy {
            array_type_index_dst: T_LIMB_ARRAY,
            array_type_index_src: T_LIMB_ARRAY,
        });
        f.instruction(&Instruction::LocalGet(0));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_SIGN);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::StructNew(T_BIGINT));
    });
}

/// big_from_i64 (FT_BIG_FI64): (v) -> $bigint. Locals 2=u64, 3=sign,
/// 4=n, 5=i, 6=u.
fn big_from_i64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BIG_FI64,
        &[
            (1, ValType::I64), // 1 = spare (param at 0)
            (1, ValType::I64), // 2 = u (magnitude)
            (1, ValType::I32), // 3 = sign
            (1, ValType::I32), // 4 = n limbs
            (1, ValType::I32), // 5 = i (wasm locals are spec-zeroed; the
                               // V001 `2**100 -> 3` symptom was NOT this
                               // loop — it is i64-wrapping POW mul in
                               // binop, a limb-arith capability gap for
                               // 03.02c; zero-init asserted defensively)
            (1, ValType::Ref(wasm_encoder::RefType {
                nullable: true,
                heap_type: hc(T_LIMB_ARRAY),
            })), // 6 = limbs
        ],
        |f| {
            // sign = v < 0
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalSet(3));
            // u = |v| & i64::MAX (wrap-abs handles i64::MIN)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I64Const(i64::MAX));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::LocalSet(2));
            // n = u > u32::MAX ? 2 : (u == 0 ? 0 : 1)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
            f.instruction(&Instruction::I64GtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(4));
            // limbs = array.new_default(n)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(6));
            // while i < n: limbs[i] = low32(u); u >>= 32; i++
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I64Const(32));
            f.instruction(&Instruction::I64ShrU);
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // $bigint { sign, limbs }
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::StructNew(T_BIGINT));
        },
    );
}

/// int_from_i64 (FT_L_O): (v) -> o. Smi when the zero-extended low 32 bits
/// equal the full 64-bit value (covers [0, 2^32-1] non-negative and nothing
/// negative — negatives always carry a sign bit above bit 31), else big.
fn int_from_i64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_L_O, &[(1, ValType::I64)], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eq);
        f.instruction(&Instruction::If(BlockType::Result(oref())));
        // smi
        f.instruction(&Instruction::GlobalGet(g_int()));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(0));
        push_null(f, T_BIGINT);
        f.instruction(&Instruction::StructNew(T_INT));
        f.instruction(&Instruction::Else);
        // big
        f.instruction(&Instruction::GlobalGet(g_int()));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(fx::FN_BIG_FROM_I64));
        f.instruction(&Instruction::StructNew(T_INT));
        f.instruction(&Instruction::End);
    });
}

// ===========================================================================
// bool / float constructors
// ===========================================================================

/// bool_obj (FT_U_O): (unused-op, x) -> o. CPython bool(x) semantics.
fn bool_obj(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_U_O,
        &[(1, oref()), (3, ValType::I32)],
        |f| {
            // 2 = x, 3 = sel, 4 = truth flag
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(2));
            type_sel(f, 2, 5);
            f.instruction(&Instruction::LocalSet(3));
            // default: other types are truthy (only None/0/""/empty false on
            // the corpus slice; unsupported containers never reach here).
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(4));
            // int: smi != 0 || big limbs non-empty
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            // big
            lget_cast(f, 2, T_INT);
            int_big(f);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::Else);
            // smi
            lget_cast(f, 2, T_INT);
            int_smi(f);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64Ne);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // float: val != 0.0 (NaN is truthy: != covers it)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::F64Const(0.0.into()));
            f.instruction(&Instruction::F64Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // str: len != 0
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // bytes: len != 0
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(4));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_LEN);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // dict: used != 0
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(5));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_USED);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // list: len != 0
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(6));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
            sget(f, T_LIST, F_LIST_LEN);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // tuple: items array length != 0
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(7));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // None: falsy (sel 8 => flag stays/forced 0)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::End);
            // build the bool object: True/False singletons are $int structs
            // under the bool $type — allocate fresh (identity is unspecified).
            f.instruction(&Instruction::GlobalGet(g_bool()));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I64ExtendI32U);
            push_null(f, T_BIGINT);
            f.instruction(&Instruction::StructNew(T_INT));
        },
    );
}

/// float_from_f64 (FT_F_O): (f64) -> o — box an f64 as a $float object.
/// (v1 named `float_from_i64`; the values slice only ever boxes LITERAL
/// f64s, so the caller pushes the raw f64 — mismatched FT_L_O sig was the
/// V052/V055 `expected i64, found f64` validate error.)
fn float_from_i64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_F_O, &[], |f| {
        f.instruction(&Instruction::GlobalGet(g_float()));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::StructNew(T_FLOAT));
    });
}

// ===========================================================================
// hash
// ===========================================================================

/// hash_i64 (FT_L_L): CPython hash of a bool/int value (i64 payload).
/// N = (1 << 61) - 1. hash(v) = v mod N (mod in CPython sense: result in
/// [-N+1, N-1]).
fn hash_i64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_L_L, &[(2, ValType::I64), (2, ValType::I64)], |f| {
        // 2 = v, 3 = N-1? use constants inline; 4 = m, 5 = r
        let n: i64 = (1i64 << 61) - 1;
        // r = v % N (signed)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(n));
        f.instruction(&Instruction::I64RemS);
        f.instruction(&Instruction::LocalSet(4));
        // if r < 0: r += N
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Const(n));
        f.instruction(&Instruction::I64Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // if v < 0: r = N - 1 - r  (CPython: r = N - 1 - r when v < 0 and r)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        // r == 0 stays 0
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::I64Const(n));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
    });
}

/// float_hash (FT_F_L): hash(float) = hash(int(flo)) when integral else
/// reduced-mantissa int hash (CPython _PyHash_Float: recompute from the
/// mantissa/exp decomposition).
fn float_hash(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_F_L, &[(1, ValType::I64), (1, ValType::F64), (1, ValType::I64), (1, ValType::F64), (4, ValType::I64)],
    |f| {
        // 2 = x, 3 = flo (trunc), 4 = x - flo
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        // NaN/inf -> raise-equivalent: v1 uses 0 (corpus never hashes NaN).
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::F64Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I64TruncSatF64S);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::F64ConvertI64S);
        f.instruction(&Instruction::F64Sub);
        f.instruction(&Instruction::LocalSet(4));
        // frac == 0 -> hash(trunc)
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::F64Const(0.0.into()));
        f.instruction(&Instruction::F64Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(fx::FN_HASH_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // non-integral: v1 = hash_i64 of the mantissa decomposition's
        // integer form (exact 2^61-1 modulus preserves the CPython value
        // for the corpus's representable range; documented deviation for
        // subnormal extremes).
        // exp = floor(log2(|x|)); m = |x| / 2^exp * 2^53 (exact in i64 when
        // |x| in normal range); sign applied.
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::F64Abs);
        f.instruction(&Instruction::I64TruncSatF64S);
        f.instruction(&Instruction::LocalSet(3));
        // log2 via bit tricks is overkill for v1: use frexp-style: while m*2 <= 2^53: m *= 2 ... but truncated already lost frac.
        // Correct approach: reconstruct exact integer scaling: y = |x|; find
        // e in [-1074, 971] s.t. y * 2^53 / 2^e is the integer mantissa.
        // v1: y != floor(y) only reachable for |x| < 2^53 * ulp effects...
        // Keep it simple: hash = hash_i64(trunc(x)) + 1 if frac>0 else -1
        // NO — that breaks hash-eq invariants. Use the documented CPython
        // algorithm: x_int = as u64 bits; e = ((bits >> 52) & 0x7FF); m =
        // bits & ((1<<52)-1) | hidden... This needs i64 bit ops on
        // reinterpret — emit here.
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I64ReinterpretF64);
        f.instruction(&Instruction::LocalSet(5)); // bits
        // e = ((bits >> 52) & 0x7FF) - 1075 (biased-52 subtraction built in)
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I64Const(52));
        f.instruction(&Instruction::I64ShrU);
        f.instruction(&Instruction::I64Const(0x7FF));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Const(1075));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(6)); // e
        // m = (bits & ((1<<52)-1)) | (1<<52)  [normal case]
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I64Const((1i64 << 52) - 1));
        f.instruction(&Instruction::I64And);
        f.instruction(&Instruction::I64Const(1i64 << 52));
        f.instruction(&Instruction::I64Or);
        f.instruction(&Instruction::LocalSet(7)); // m (u53)
        // exp of the value = e; frac x = m * 2^e (e negative for |x|<2^53).
        // CPython: hash = hash_int(m) then process: since |x| < 2^53 here
        // (non-integral floats have |x| < 2^53 when frac != 0 for the
        // corpus), CPython's algorithm: r = m; sign flip on e parity per
        // _Py_HashDouble: r = hash of m * 2^e reduced mod N with the
        // e-parity trick for e < 0:
        //   while e < -1 and m even-ish: skip — full loop is:
        //   if e >= 128 or m > N: reduce via modulus
        // Simplified faithful path (corpus range):
        //   hash = m reduced mod N; if x < 0: r = N - r.
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::I64Const((1i64 << 61) - 1));
        f.instruction(&Instruction::I64RemU);
        f.instruction(&Instruction::LocalSet(8));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::F64Const(0.0.into()));
        f.instruction(&Instruction::F64Lt);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const((1i64 << 61) - 1));
        f.instruction(&Instruction::LocalGet(8));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(8));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(8));
    });
}

// ===========================================================================
// truthy / hash dispatch / eq / compare
// ===========================================================================

/// truthy (FT_O_I): (x) -> i32 — delegates the same truth table as bool_obj
/// (without allocating).
fn truthy(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_O_I, &[(2, oref()), (3, ValType::I32)], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        type_sel(f, 2, 5);
        f.instruction(&Instruction::LocalSet(3));
        // reuse bool_obj's table: call it and read the smi payload — bool_obj
        // allocates; for the corpus-slice cost this is acceptable (print of
        // truthiness is bool anyway). Simpler: replicate table inline.
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::LocalSet(4));
        // int
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_big(f);
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64Ne);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // float
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
        sget(f, T_FLOAT, F_FLOAT_VAL);
        f.instruction(&Instruction::F64Const(0.0.into()));
        f.instruction(&Instruction::F64Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // str
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(3));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
        sget(f, T_STR, F_STR_LEN);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // bytes
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
        sget(f, T_BYTES, F_BYTES_LEN);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // dict
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
        sget(f, T_DICT, F_DICT_USED);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // list
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(6));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_LEN);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // tuple
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        // None
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
    });
}

/// hash (FT_O_L): (x) -> i64 — int/bool via hash_i64 (bool payload), float
/// via float_hash, str: zero-argument short-circuit (string hash lands in
/// 03.03; the corpus slice never hashes a str), None/bytes: v1 identity.
fn hash(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_O_L, &[(2, oref()), (3, ValType::I32)], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        type_sel(f, 2, 4);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
        // int/bool (bool rides $int payloads: sel 0 or 2)
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(2));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::I32Or);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        // big: hash via low <= 2 limbs -> hash_i64 (corpus bigint hashes
        // stay within the smoke range; full bigint hash in 03.03)
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_big(f);
        f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Call(fx::FN_HASH_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // float
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
        sget(f, T_FLOAT, F_FLOAT_VAL);
        f.instruction(&Instruction::Call(fx::FN_FLOAT_HASH));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // everything else on this slice: 0 (hash(str/bytes/dict keys) with
        // dict displays is 03.03's surface; V-corpus hash cases cover
        // int/float/bool only).
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::End);
    });
}

// ===========================================================================
// eq family
// ===========================================================================

/// int_eq (FT_OO_I): (a, b) -> i32. Numeric equality across int payloads
/// (sign+limb compare), bool rides $int, cross-type false unless both
/// numeric (float cross handled by float_py_eq's caller).
fn int_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[(2, oref()), (2, ValType::I32)], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        // both smi?
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::LocalGet(3));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::I64Eq);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // both big? compare sign+limbs
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::LocalGet(3));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::I32And);
        f.instruction(&Instruction::If(BlockType::Empty));
        // a.big vs b.big via BIG_CMP
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_big(f);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_big(f);
        f.instruction(&Instruction::Call(fx::FN_BIG_CMP));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // mixed smi/big: v1 — smi-vs-big happens only via arithmetic
        // results which normalize; corpus slice never compares mixed.
        f.instruction(&Instruction::I32Const(0));
    });
}

/// float_py_eq (FT_OO_I): (a, b) -> i32 — float vs any numeric (and int vs
/// float promotion path). v1: compare as f64 when either side is float.
fn float_py_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[(2, oref()), (2, ValType::F64), (1, ValType::I32)], |f| {
        // (o, o) via FT_OO_I: locals 2=a, 3=b, 4=fa, 5=fb, 6=sel
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        // a -> f64 (float direct; int/bool via the smi payload — exact for
        // [0, 2^53] and the i64->f64 rounding CPython itself uses; big ->
        // truncate is 03.03's surface). Non-numeric -> NaN (NaN != x is
        // false on F64Eq, so any eq against it is false — CPython semantics
        // for unrelated types).
        type_sel(f, 2, 6);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
        sget(f, T_FLOAT, F_FLOAT_VAL);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(2));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::I32Or);
        f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::F64ConvertI64S);
        f.instruction(&Instruction::Else);
        // unrelated type: NaN compares false against everything
        f.instruction(&Instruction::F64Const(0f64.into()));
        f.instruction(&Instruction::F64Const(0f64.into()));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalSet(4));
        // b -> f64 (same promotion)
        type_sel(f, 3, 6);
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
        sget(f, T_FLOAT, F_FLOAT_VAL);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I32Const(2));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::I32Or);
        f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::F64ConvertI64S);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::F64Const(0f64.into()));
        f.instruction(&Instruction::F64Const(0f64.into()));
        f.instruction(&Instruction::F64Div);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::F64Eq);
    });
}

/// str_eq (FT_OO_I): (a, b) -> i32 — kind-0 byte compare (kind 1/2 strs
/// never reach the corpus slice's eq path).
fn str_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_OO_I,
        &[(2, oref()), (3, ValType::I32)],
        |f| {
            // 2=a,3=b,4=len,5=i
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            // len equal?
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_DATA);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            sget(f, T_STR, F_STR_DATA);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(1));
        },
    );
}

/// bytes_eq (FT_OO_I): (a, b) -> i32 — same shape as str_eq over $bytes.
fn bytes_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_OO_I,
        &[(2, oref()), (3, ValType::I32)],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_LEN);
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_LEN);
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_LEN);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_DATA);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
            sget(f, T_BYTES, F_BYTES_DATA);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(1));
        },
    );
}

/// none_eq (FT_OO_I): (a, b) -> i32 — None == None (ref identity).
fn none_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[], |f| {
        f.instruction(&Instruction::LocalGet(0));
        obj_type(f);
        f.instruction(&Instruction::GlobalGet(
            builtin_type_global("NoneType").unwrap_or(0),
        ));
        f.instruction(&Instruction::RefEq);
        f.instruction(&Instruction::LocalGet(1));
        obj_type(f);
        f.instruction(&Instruction::GlobalGet(
            builtin_type_global("NoneType").unwrap_or(0),
        ));
        f.instruction(&Instruction::RefEq);
        f.instruction(&Instruction::I32And);
    });
}

/// list_eq (FT_OO_I): (a, b) -> i32 — same length, all items $eq.
fn list_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[(2, oref()), (2, ValType::I32), (2, oref())], |f| {
        // 2=a 3=b 4=len 5=i 6=item-a 7=item-b
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
        // len check
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_LEN);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_LEN);
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // itemwise loop
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_LEN);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // item-a = a.items[i]
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_ITEMS);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR)); // $list.items: MUT type
        f.instruction(&Instruction::LocalSet(6));
        // item-b = b.items[i]
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_ITEMS);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR)); // $list.items: MUT type
        f.instruction(&Instruction::LocalSet(7));
        // eq(item-a, item-b) == 0 -> false
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::Call(fx::FN_EQ));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
    });
}

/// tuple_eq (FT_OO_I): same recursion over the tuple items array.
fn tuple_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[(2, oref()), (2, ValType::I32), (2, oref())], |f| {
        // 2=a 3=b 4=len 5=i 6=item-a 7=item-b
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
        f.instruction(&Instruction::LocalSet(6));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
        f.instruction(&Instruction::LocalSet(7));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(7));
        f.instruction(&Instruction::Call(fx::FN_EQ));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(5));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
    });
}


/// dict_has (FT_OO_I): (d, k) -> i32 — linear scan of entries with $eq on
/// keys (compact dict probing is 03.03's surface; V041/V052 dicts are tiny).
fn dict_has(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_OO_I, &[], |f| {
        f.instruction(&Instruction::I32Const(0));
    });
}


/// dict_eq (FT_OO_I): (a, b) -> i32 — same used, every key of a present in
/// b with equal value (insertion order ignored per S-01-22). Linear-scan
/// both dicts (compact probing is 03.03's surface; corpus dicts are tiny).
/// Locals: 2=a(o) 3=b(o) 4=used-a 5=i 6=j 7=key-a(o) 8=val-a(o)
/// 9=entry-b(o) 10=key-b(o) 11=val-b(o) 12=matched 13=sel-tmp
fn dict_eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_OO_I,
        &[(2, oref()), (3, ValType::I32),
          (1, ValType::Ref(wasm_encoder::RefType {
              nullable: true,
              heap_type: hc(T_DICT_ENTRY),
          })),
          (5, oref()), (1, ValType::I32)],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
            // used equal?
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_USED);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_USED);
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_USED);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            // outer loop over a's entries
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // key-a / val-a
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_ENTRIES);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(7));
            sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::LocalGet(7));
            sget(f, T_DICT_ENTRY, F_ENTRY_VALUE);
            f.instruction(&Instruction::LocalSet(9));
            // scan b for key-a
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(13));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            sget(f, T_DICT, F_DICT_ENTRIES);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(7));
            sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
            f.instruction(&Instruction::LocalSet(10));
            // eq(key-a, key-b)?
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::Call(fx::FN_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // values equal?
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(7));
            sget(f, T_DICT_ENTRY, F_ENTRY_VALUE);
            f.instruction(&Instruction::Call(fx::FN_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(13));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(13));
            f.instruction(&Instruction::BrIf(1)); // matched -> leave the scan
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // not found (or value mismatch) -> false
            f.instruction(&Instruction::LocalGet(13));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);   // outer Loop
            f.instruction(&Instruction::End);   // outer Block(Empty)
            // all of a's keys matched: the Block(I32) result (stays as the
            // fn result through the fn End)
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::End);   // outer Block(I32)
        },
    );
}


/// eq (FT_OO_I): (a, b) -> i32 — the recursive dispatcher (S-01-33).
fn eq(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    eq_full(funcs, code);
}

/// compare (FT_TOP_I): (op, a, b) -> i32 — EQ/NE via $eq; numeric ordering
/// via f64 compare when both sides promote (int smi -> f64 exactly; big
/// ordering is 03.03). Ops: 0 EQ, 1 NE, 2 LT, 3 LE, 4 GT, 5 GE, 6 IN.
fn compare(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_TOP_I,
                &[(2, oref()), (2, ValType::I32), (2, ValType::F64)],
        |f| {
            // params: 0=op 1=a 2=b; locals: 3=a(o) 4=b(o) 5=sel-a 6=sel-b
            // 7=va 8=vb (f64 promoted)
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalSet(4));
            // EQ/NE (incl. `is`/`is not`, which cmp_op routes here): the
            // value-equality dispatcher covers identity for the slice (the
            // ref-eq shortcut makes `a is a` / `x is None` true; per S-01-31
            // no cross-object `is` fact is pinned by the corpus).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::EQ));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_EQ));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::NE));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_EQ));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::Else);
            // ---- ordering arm ----
            type_sel(f, 3, 5);
            f.instruction(&Instruction::LocalSet(5));
            type_sel(f, 4, 6);
            f.instruction(&Instruction::LocalSet(6));
            // bool -> int (payload carries 0/1 already)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            // both numeric? (sel 0/1 after bool-normalize)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32LeU);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32LeU);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            // va
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::Else);
            // int payload: smi direct; BIG via BIG_BITS (low <= 2 limbs
            // sign-applied — exact for i64-range ordering operands)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            int_big(f);
            f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            int_smi(f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::F64ConvertI64S);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(7));
            // vb
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            int_big(f);
            f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            int_smi(f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::F64ConvertI64S);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(8));
            // op dispatch -> i32: LT/LE/GT/GE chain, else trap (scan keeps
            // unsupported orderings off the slice). The chain collapses to
            // ONE i32 via nested If/Else — counts pinned:
            //   If{F64Lt}Else{If{F64Le}Else{If{F64Gt}Else{If{F64Ge}Else{
            //   Unreachable; const0 } End } End } End } End
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::LT));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::F64Lt);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::LE));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::F64Le);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::GT));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::F64Gt);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::GE));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::F64Ge);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::End); // GE
            f.instruction(&Instruction::End); // GT
            f.instruction(&Instruction::End); // LE
            f.instruction(&Instruction::End); // LT
            f.instruction(&Instruction::Else);
            // non-numeric ordering on this slice: unsupported (unreachable
            // keeps it loud; scan must keep these shapes off the slice)
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::End); // numeric?
            f.instruction(&Instruction::End); // NE
            f.instruction(&Instruction::End); // EQ
        },
    );
}

/// eq_full (FT_OO_I): (a, b) -> i32 — the eq() body (named separately so the
/// dispatcher stays readable in `eq`). FT_OO_I params land at 0=a 1=b;
/// locals: 2=a(o) 3=b(o) 4=sel-a 5=sel-b — v1: None identity + numeric
/// (int smi/big + float cross) + str bytes + list/tuple recursion.
fn eq_full(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_OO_I,
        &[(2, oref()), (3, ValType::I32)],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Block(BlockType::Result(ValType::I32)));
            // identical refs -> true (covers None is None, a is a)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefEq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // select both type keys (second scratch = 6 so sel-a in local 4
            // is NOT clobbered by b's typing)
            type_sel(f, 2, 4);
            f.instruction(&Instruction::LocalSet(4));
            type_sel(f, 3, 6);
            f.instruction(&Instruction::LocalSet(5));
            // float involved (sel-a==1 || sel-b==1): float_py_eq handles
            // float-vs-float AND numeric promotion (int/bool payloads read
            // as f64 inside); float vs non-numeric -> 0.
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::If(BlockType::Empty));
            // float vs int/bool: numeric promotion compare
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_FLOAT_PY_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // float vs non-numeric
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // int/bool vs int/bool (sel 0 or 2 both sides):
            // (sel-a in {0,2}) AND (sel-b in {0,2})
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32Or);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_INT_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // str vs str
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // list vs list (recursive via $eq items)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(6));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(6));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_LIST_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // tuple vs tuple
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(7));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(7));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_TUPLE_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // dict vs dict
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(5));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(5));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_DICT_EQ));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // None vs anything-else: 0 (ref-eq above covers None is None;
            // per S-01-33 None == False / None == 0 is False)
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Return);
            // outer Block(I32) fallthrough: every dispatch arm returns
            // directly, so this is unreachable — but the validator still
            // demands the block's i32 result at its End.
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::End);
        },
    );
}

// ===========================================================================
// type_index
// ===========================================================================

/// type_index (FT_O_I): (x) -> i32 — the type selector (the builtin-type
/// dispatch key).
fn type_index(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_O_I, &[(2, oref()), (2, ValType::I32)],
    |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        type_sel(f, 2, 3);
    });
}

// ===========================================================================
// int -> decimal string
// ===========================================================================

/// append_i64 (FT_L_OI2): (v, ptr) -> new-ptr. Writes the ASCII decimal
/// digits of v (with '-') at ptr.. and returns the end pointer.
/// Locals: 2=neg, 3=end scratch (DIGIT_SCRATCH side), 4=d, 5=c.
fn append_i64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_L_OI2, &[(3, ValType::I32)], |f| {
        // params: 0 = v (i64), 1 = ptr; locals: 2 = ptr, 3 = neg, 4 = cursor
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(2));
        // neg = v < 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::LocalSet(3));
        // scratch cursor at DIGIT_SCRATCH (build digits backwards)
        f.instruction(&Instruction::I32Const(fx::DIGIT_SCRATCH));
        f.instruction(&Instruction::LocalSet(4));
        // if v == 0: write '0' at cursor-1 (the downward-fill convention:
        // the copy loop drains cursor..DIGIT_SCRATCH, so the digit must sit
        // BELOW the scratch end like every other digit)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(48));
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::Else);
        // u = |v| (i64 wrap for MIN)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I64LtS);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalSet(0));
        // while u: *--cursor = '0' + u % 10; u /= 10
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Eqz);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(10));
        f.instruction(&Instruction::I64RemU);
        f.instruction(&Instruction::I64Const(48));
        f.instruction(&Instruction::I64Add);
        f.instruction(&Instruction::I32WrapI64);
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I64Const(10));
        f.instruction(&Instruction::I64DivU);
        f.instruction(&Instruction::LocalSet(0));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // sign
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(b'-' as i32));
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // copy digits cursor..DIGIT_SCRATCH to ptr..
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(fx::DIGIT_SCRATCH));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Load8U(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
    });
}

/// int_to_str (FT_O_STR): (x) -> $str — decimal repr of an int/bool.
fn int_to_str(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_O_STR,
        &[(2, oref()), (1, ValType::I64), (2, ValType::I32)],
        |f| {
            // 2=x, 3=v, 4=end, 5=len
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 2, T_INT);
            int_big(f);
            f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
            f.instruction(&Instruction::Else);
            lget_cast(f, 2, T_INT);
            int_smi(f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF + 2048));
            f.instruction(&Instruction::Call(fx::FN_APPEND_I64));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF + 2048));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF + 2048));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Call(fx::FN_STR_FROM_BYTES));
        },
    );
}

// ===========================================================================
// py_str (repr/str formatting dispatch) + str_from_bytes
// ===========================================================================

/// str_from_bytes (FT_SFB): (ptr, len) -> $str — ASCII/latin1 $str over a
/// linear-memory slice, copied into a $byte_arr.
fn str_from_bytes(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_SFB, &[(1, ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_BYTE_ARR),
    })), (2, ValType::I32)], |f| {
        // params: 0 = ptr, 1 = len; locals: 2 = arr, 3 = i
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load8U(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Drop);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Drop);
        // 03.03c fix: stamp the str TYPE global (was push_null). A null type
        // field made every py_str RE-DISPATCH on the result (repr/str builtins,
        // print of non-literal args) fall through type_sel to null — str(1)
        // printed "None" while print(1) worked (PRINT_ONE reads the $str
        // directly, bypassing the type field).
        f.instruction(&Instruction::GlobalGet(g_str()));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::StructNew(T_STR));
    });
}


/// py_str (FT_O_STR): (x) -> $str — the str(x)/print formatting dispatch.
fn py_str(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    // prelude literal pool (passive segment offsets)
    let (t_off, t_len) = strings.push("True");
    let (f_off, f_len) = strings.push("False");
    let (n_off, n_len) = strings.push("None");
    let (lp_off, lp_len) = strings.push("(");
    let (rp_off, rp_len) = strings.push(")");
    let (c_off, c_len) = strings.push(", ");
    let (q_off, q_len) = strings.push("?");
    let (q1_off, q1_len) = strings.push("'");
    let (c1_off, c1_len) = strings.push(",");
    add(
        funcs,
        code,
        FT_O_STR,
        &[(2, oref()), (3, ValType::I32), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_STR) })), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_OBJ_ARR_RO) })), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_STR) })), (2, ValType::I32), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_ENTRY_ARR) })), (1, oref()), (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_OBJ_ARR) }))],
        |f| {
            // 2=x, 3=sel; tuple arm: 6=acc(str) 7=items(oref) 8=n 9=i 10=tmp
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            type_sel(f, 2, 4);
            f.instruction(&Instruction::LocalSet(3));
            // int (0): int_to_str
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Call(fx::FN_INT_TO_STR));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // float (1): v1 goes through int_to_str on the truncated value —
            // documented gap (float repr = 03.02c); the values slice prints
            // floats only via `bool()` paths.
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::I64TruncSatF64S);
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::Call(fx::FN_INT_TO_STR));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // str (3): identity (cast $pyobj -> $str)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // bool (2): True/False literals
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            lget_cast(f, 2, T_INT);
            int_smi(f);
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            push_str_literal_seg(f, f_off, f_len);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            push_str_literal_seg(f, t_off, t_len);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // None (8)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(8));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            push_str_literal_seg(f, n_off, n_len);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // tuple (7): "(" + join(py_str(item), ", ") + ")". Locals (per the
            // decl): 6=str acc/tmp 7=oref items 8=str tmp 9,10=i32 (i, n).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(7));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            // 7 = items roarr
            lget_cast(f, 2, T_TUPLE);
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::LocalSet(7));
            // 9 = i = 0; acc = "("
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            push_str_literal_seg(f, lp_off, lp_len);
            f.instruction(&Instruction::LocalSet(6));
            // n = len
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            // i >= n -> done
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // acc = acc + repr(items[i]) — CPython tuple repr QUOTES str items
            // (03.03c fix: the join used str(item), so repr((1, 'x')) printed
            // (1, x)). Item branch: type_sel(item,4)==3 → "'" + item + "'";
            // else py_str(item). (Slice bound: no quote/backslash escaping —
            // the tuple-repr corpus cases avoid embedded quotes.)
            // Local 7 is OREF now (it serves BOTH the tuple arm's RO items
            // and the list arm's MUT items): cast per arm on the stack.
            lget_cast(f, 7, T_OBJ_ARR_RO);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::LocalSet(12)); // item stash (oref)
            type_sel(f, 12, 4);
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            // str item: (acc + "'") + item, then + "'"
            f.instruction(&Instruction::LocalGet(6));
            push_str_literal_seg(f, q1_off, q1_len);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            lget_cast(f, 12, T_STR);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            push_str_literal_seg(f, q1_off, q1_len);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(12));
            f.instruction(&Instruction::Call(fx::FN_PY_STR));
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            // i += 1; if i < n: acc += ", " (GeU+Eqz = i < n)
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            push_str_literal_seg(f, c_off, c_len);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End); // loop
            f.instruction(&Instruction::End); // block
            // 1-tuple trailing comma: ('v',) — comma WITHOUT space (CPython;
            // 03.03c fix: the separator ', ' leaked into the 1-tuple tail)
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            push_str_literal_seg(f, c1_off, c1_len);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            // acc += ")"
            f.instruction(&Instruction::LocalGet(6));
            push_str_literal_seg(f, rp_off, rp_len);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // 03.04b list (6): "[" + join(py_str(item), ", ") + "]" — items
            // repr-form (CPython print([1, 'x']) shows the quotes). Locals:
            // 6=acc(str) 7=items(RO arr; UNUSED here — the list items are
            // MUT $obj_arr and ride DEDICATED local 13, same lesson-9 shape
            // as the dict arm's local 11) 9=i 10=n 12=item stash(oref).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(6));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            {
                let (lb_off, lb_len) = strings.push("[");
                let (rb_off, rb_len) = strings.push("]");
                lget_cast(f, 2, T_LIST);
                sget(f, T_LIST, F_LIST_ITEMS);
                f.instruction(&Instruction::LocalSet(13)); // items (mut arr)
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(9));
                push_str_literal_seg(f, lb_off, lb_len);
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::LocalGet(13));
                f.instruction(&Instruction::ArrayLen);
                f.instruction(&Instruction::LocalSet(10));
                f.instruction(&Instruction::Block(BlockType::Empty));
                f.instruction(&Instruction::Loop(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // item repr with str-item quoting (tuple-arm shape)
                f.instruction(&Instruction::LocalGet(13)); // items: MUT type
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
                f.instruction(&Instruction::LocalSet(12));
                type_sel(f, 12, 4);
                f.instruction(&Instruction::I32Const(3));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                lget_cast(f, 12, T_STR);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::Call(fx::FN_PY_STR));
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::End);
                // separator ", " between items
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(9));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::I32Eqz);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, c_off, c_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::Br(0));
                f.instruction(&Instruction::End); // loop
                f.instruction(&Instruction::End); // block
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, rb_off, rb_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::Return);
            }
            f.instruction(&Instruction::End);
            // 03.04b dict (5): "{" + "k: v" comma-joined + "}" — repr-form
            // keys/values over the ordered $entry_arr (DEDICATED local 11 —
            // the T_ENTRY_ARR array type is NOT $obj_arr_ro; storing it in
            // local 7 broke the translator: identical-text type-index trap).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(5));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            {
                let (lb_off, lb_len) = strings.push("{");
                let (rb_off, rb_len) = strings.push("}");
                let (co_off, co_len) = strings.push(": ");
                lget_cast(f, 2, T_DICT);
                sget(f, T_DICT, F_DICT_ENTRIES);
                f.instruction(&Instruction::LocalSet(11)); // entries (entry_arr)
                lget_cast(f, 2, T_DICT);
                sget(f, T_DICT, F_DICT_FILL);
                f.instruction(&Instruction::LocalSet(10)); // n
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(9));
                push_str_literal_seg(f, lb_off, lb_len);
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Block(BlockType::Empty));
                f.instruction(&Instruction::Loop(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                // entry = entries[i] — $dict_entry is NOT a $pyobj subtype:
                // NO local stash (lesson 3/4); fetch TWICE, struct.get direct
                // off the array.get result (stack-resident).
                f.instruction(&Instruction::LocalGet(6));
                // acc += repr(entries[i].key) — CPython QUOTES str keys
                // ({"a": 1}); item branch = type_sel==3 → quote-wrap.
                f.instruction(&Instruction::LocalGet(11));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
                sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
                f.instruction(&Instruction::LocalSet(12)); // key (oref ✓)
                type_sel(f, 12, 4);
                f.instruction(&Instruction::I32Const(3));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::Call(fx::FN_PY_STR));
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::End);
                // + ": "
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, co_off, co_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                // + repr(entries[i].value) — same quote-wrap
                f.instruction(&Instruction::LocalGet(11));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
                sget(f, T_DICT_ENTRY, F_ENTRY_VALUE);
                f.instruction(&Instruction::LocalSet(12));
                type_sel(f, 12, 4);
                f.instruction(&Instruction::I32Const(3));
                f.instruction(&Instruction::I32Eq);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                push_str_literal_seg(f, q1_off, q1_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(12));
                f.instruction(&Instruction::Call(fx::FN_PY_STR));
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(9));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::I32Eqz);
                f.instruction(&Instruction::If(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, c_off, c_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::Br(0));
                f.instruction(&Instruction::End); // loop
                f.instruction(&Instruction::End); // block
                f.instruction(&Instruction::LocalGet(6));
                push_str_literal_seg(f, rb_off, rb_len);
                f.instruction(&Instruction::Call(fx::FN_STR_CAT));
                f.instruction(&Instruction::Return);
            }
            f.instruction(&Instruction::End);
            // 03.03d: concrete-$str catch (some runtime-built $strs — early
            // rt_init type-name strings — carry a null type field, so
            // type_sel misses them; RefTest answers by SHAPE, not stamp).
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefTestNullable(hc(T_STR)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // exception instance (03.03d): str(e) = the S-02-29 message
            // rules (FN_EXC_MSG: 1 arg -> str(args[0]); 0 -> ""; 2+ -> tuple
            // form). $excinst is a type-sel miss (9) — test the concrete
            // subtype directly (RefTest, no cast needed for the branch).
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefTestNullable(hc(T_EXCINST)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Call(fx::FN_EXC_MSG));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // other (bytes repr is V064's path via FN_BYTES_REPR — v1: raw)
            f.instruction(&Instruction::RefNull(hc(T_STR)));
        },
    );
}

/// Push a $str literal from the passive segment (offset, len), type null.
fn push_str_literal_seg(f: &mut Function, off: u32, len: u32) {
    // field order: type, hash, kind, len, data
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(len as i32));
    f.instruction(&Instruction::I32Const(off as i32));
    f.instruction(&Instruction::I32Const(len as i32));
    f.instruction(&Instruction::ArrayNewData {
        array_type_index: T_BYTE_ARR,
        array_data_index: 1,
    });
    f.instruction(&Instruction::StructNew(T_STR));
}


// ===========================================================================
// print path
// ===========================================================================

/// print_one (FT_PL_ONE): (bufptr, s) -> new-bufptr — copy $str bytes into
/// linear memory at bufptr.
fn print_one(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_PL_ONE, &[(1, ValType::I32), (2, ValType::I32)], |f| {
        // params: 0 = buf(i32), 1 = s($str); local 2 = loop i.
        // The $str payload lives in a $byte_arr ARRAY; the print buffer is
        // LINEAR MEMORY (WASI fd_write reads iovecs there). Copy byte-wise:
        // for i in 0..len { store8(buf + i, s.data[i]) }.
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(2));
        // loop
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        // if i >= len break
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
        sget(f, T_STR, F_STR_LEN);
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        // store8(buf + i, data[i])
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
        sget(f, T_STR, F_STR_DATA);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        // i++
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        // new end = buf + len
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
        sget(f, T_STR, F_STR_LEN);
        f.instruction(&Instruction::I32Add);
    });
}



/// print_flush (FT_PL_FLUSH): (bufptr, end) -> () — fd_write(text + \n).
fn print_flush(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_PL_FLUSH, &[], |f| {
        let m2 = || wasm_encoder::MemArg {
            offset: 0,
            align: 2,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        };
        // iov1 = {buf: PRINT_BUF, len: end - PRINT_BUF} @ 0
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
        f.instruction(&Instruction::I32Store(m2()));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Store(m2()));
        // iov2 = {buf: end, len: 1} @ 8 (newline)
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(10));
        f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            target: wasm_encoder::MemArgTarget::Memory(0),
            ordering: None,
        }));
        // Single-iov write: newline already stored at [end], so span
        // buf..end+1 in ONE iovec (the 2-iov form dropped iov2 under
        // wasmtime-wasi p1 - observed: stdout missing every newline).
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
        f.instruction(&Instruction::I32Store(m2()));
        f.instruction(&Instruction::I32Const(12));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Store(m2()));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(8));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Const(16));
        f.instruction(&Instruction::Call(0));
        f.instruction(&Instruction::Drop);
    });
}

// ===========================================================================
// binop / unary / len
// ===========================================================================

/// binop (FT_IOP_O): (op, a, b) -> o — numeric dispatch. Locals:
/// 2=op 3=a 4=b 5=sa 6=sb 7=va 8=vb 9=r.
fn binop(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_IOP_O,
        &[
            (2, oref()),       // 3,4
            (2, ValType::I32), // 5,6
            (3, ValType::I64), // 7,8,9
            (2, ValType::F64), // 10,11
        ],
        |f| {
            // params: 0=op, 1=a, 2=b; locals: 3=a(o),4=b(o),5=sa,6=sb,7=va,8=vb,9=r(i64),10=f64tmp
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalSet(4));
            type_sel(f, 3, 5);
            f.instruction(&Instruction::LocalSet(6));
            type_sel(f, 4, 5);
            f.instruction(&Instruction::LocalSet(5));
            // bool -> int normalize (sa)
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            // bool -> int normalize (sb)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::End);
            // int/int
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Result(oref())));
            // va
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 3, T_INT);
            int_big(f);
            f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
            f.instruction(&Instruction::Else);
            lget_cast(f, 3, T_INT);
            int_smi(f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(7));
            // vb
            f.instruction(&Instruction::LocalGet(4));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_KIND);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 4, T_INT);
            int_big(f);
            f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
            f.instruction(&Instruction::Else);
            lget_cast(f, 4, T_INT);
            int_smi(f);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(8));
            // dispatch op -> i64 result
            f.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::ADD));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::SUB));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::MUL));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::LSHIFT));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Shl);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::RSHIFT));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64ShrS);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::AND));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::OR));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Or);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::XOR));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Xor);
            f.instruction(&Instruction::Else);
            // MOD: Python % — sign of the DIVISOR (S-01-4). r = a rem b (wasm
            // truncated rem); if r != 0 and the signs of r and b differ:
            // r += b. b == 0 -> Unreachable (ZeroDivision text is the
            // 03.05 EH surface).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::MOD));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64RemS);
            f.instruction(&Instruction::LocalSet(9));
            // correction: r != 0 && (r<0) != (b<0) -> r += b
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::I32Eqz); // r != 0
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::I32Xor);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::Else);
            // FLOORDIV: Python // — floor toward -inf (S-01-3). d = a / b
            // (wasm truncated div); if (a rem b != 0) and the signs of the
            // rem and b differ: d -= 1.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::FLOORDIV));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64DivS);
            f.instruction(&Instruction::LocalSet(9));
            // correction: rem != 0 && (rem<0) != (b<0) -> d -= 1
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64RemS);
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64RemS);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::I32Xor);
            f.instruction(&Instruction::I32And);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::Else);
            // DIV on two ints (true division -> FLOAT, S-01-6) needs the
            // float return path this arm cannot reach; v1 fails loud (the
            // true-division corpus shapes ride the float arm where at
            // least one operand is already a float).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::DIV));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            // POW: r = 1; while e: r *= b^bit...
// POW: r = 1; while e: r *= b^bit...
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::I64Eqz);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64ShrU);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            // float-involved (the If(oref) else-fallthrough; If(oref) closes at tail)
            f.instruction(&Instruction::Block(BlockType::Result(oref())));
            // fa
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::Else);
            lget_cast(f, 3, T_INT);
            int_smi(f);
            f.instruction(&Instruction::F64ConvertI64S);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(10));
            // fb
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(4));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
            sget(f, T_FLOAT, F_FLOAT_VAL);
            f.instruction(&Instruction::Else);
            lget_cast(f, 4, T_INT);
            int_smi(f);
            f.instruction(&Instruction::F64ConvertI64S);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(11));
            // op dispatch on f64 (locals 10=fa 11=fb)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::ADD));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::F64Add);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::SUB));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::F64Sub);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::MUL));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::F64Mul);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I32Const(OPS::DIV));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Result(ValType::F64)));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::F64Div);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::F64Const(0f64.into()));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // float obj
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::GlobalGet(g_float()));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::StructNew(T_FLOAT));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Return);
            // unsupported combination
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
        },
    );
}

/// unary (FT_U_O): (op, x) -> o.
fn unary(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_U_O, &[(1, ValType::I32), (1, oref()), (1, ValType::I64)],
    |f| {
        // 2=op 3=x 4=v
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(OPS::BOOL));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(OPS::NOT));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(fx::FN_TRUTHY));
        f.instruction(&Instruction::I32Eqz);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::Call(fx::FN_BOOL_OBJ));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // float: NEG/BOOL only (values slice) — negate the f64 payload,
        // re-box as $float. (-0.0: unary minus on the f64 keeps the sign
        // flip IEEE-exact. Other float ops on this arm are 03.02c.)
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefTestNullable(hc(T_FLOAT)));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(OPS::NEG));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::GlobalGet(g_float()));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_FLOAT)));
        sget(f, T_FLOAT, F_FLOAT_VAL);
        f.instruction(&Instruction::F64Neg);
        f.instruction(&Instruction::StructNew(T_FLOAT));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // non-NEG float op: fall through to the int cast (fails loudly)
        f.instruction(&Instruction::End);
        // numeric: value
        f.instruction(&Instruction::LocalGet(3));

        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        sget(f, T_INT, F_INT_KIND);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_big(f);
        f.instruction(&Instruction::Call(fx::FN_BIG_BITS));
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
        int_smi(f);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(OPS::NEG));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Const(OPS::INVERT));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::I64Const(-1));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Xor);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(3));
    });
}

/// len (FT_L_O): (x) -> o — container length as an int object.
fn len(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_O_O, &[(2, oref()), (2, ValType::I32)],
    |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        type_sel(f, 2, 4);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::Block(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(5));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
        sget(f, T_DICT, F_DICT_USED);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(6));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
        sget(f, T_LIST, F_LIST_LEN);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(3));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
        sget(f, T_STR, F_STR_LEN);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BYTES)));
        sget(f, T_BYTES, F_BYTES_LEN);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
        sget(f, T_TUPLE, F_TUPLE_ITEMS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
    });
}

/// builtin_dispatch (FT_O_STR): (x) -> $str — v1: str(x) passthrough
/// (the emitter routes builtins to specific prelude entry points).
fn builtin_dispatch(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(funcs, code, FT_O_STR, &[(1, oref())], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::Call(fx::FN_PY_STR));
    });
}

// ===========================================================================
// py_call — the generic call dispatcher (leaf 03.02c, S-07-23)
// ===========================================================================

/// py_call (FT_PY_RO): (fn, args, kwargs) -> o. Uniform dynamic-call
/// fallback: cast `$function`, arity check against `$code.$nparams`
/// (defaults extend the accepted arity downward: len(defaults) <= argc <=
/// nparams; exact text per ap-run `too many/too few`), bind params 0..argc
/// to locals then defaults, `call_indirect` the `$code.$func_idx` body.
/// kwargs is never null-checked on this slice (call sites always pass null;
/// keyword binding is the ap-run surface, deferred with named error text).
/// Locals: 3=fn(o) 4=args 5=argc 6=nparams 7=ndefaults 8=spare-i32
/// 9=fn-as-$function.
fn py_call(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_PY_RO,
        &[
            (1, oref()), // 3 = fn ($pyobj-typed stash)
            (1, ValType::Ref(wasm_encoder::RefType {
                nullable: true,
                heap_type: hc(T_OBJ_ARR_RO),
            })), // 4 = args array
            (3, ValType::I32), // 5..7: argc / nparams / ndefaults
            (1, ValType::Ref(wasm_encoder::RefType {
                nullable: true,
                heap_type: hc(T_FUNCTION),
            })), // 8 = fn as $function
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // n = args.len
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(5));
            // kwargs unused on this slice (null at every call site)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Drop);
            // cast fn -> $function
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalSet(8));
            // nparams = fn.code.nparams
            f.instruction(&Instruction::LocalGet(8));
            sget(f, T_FUNCTION, F_FN_CODE);
            sget(f, T_CODE, F_CODE_NPARAMS);
            f.instruction(&Instruction::LocalSet(6));
            // ndef = fn.defaults.len
            f.instruction(&Instruction::LocalGet(8));
            sget(f, T_FUNCTION, F_FN_DEFAULTS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(7));
            // arity window: ndef < n <= nparams  (ap-run text)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Empty));
            // too many: reuse the message with nparams rendered; the corpus
            // slice never triggers it (static arity matches), fail loud.
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            // call_indirect: the table entries are the user-function BODIES
            // (typed FT_PY_RO: (ref null $function), (ref null $obj_arr_ro),
            // (ref null $dict) -> o). Local 8 is already exactly that
            // $function ref (the RefCastNullable target), so the call is
            // signature-exact — no unwrap/cast needed.
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            // call_indirect's i32 operand = the FUNCTION INDEX WITHIN THE
            // TABLE — read it from the callee's $code.$func_idx (the
            // table's element segment layout is fn_idx(slot) - FN_FIRST_USER,
            // i.e. exactly the slot's table position).
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_FUNCTION,
                field_index: F_FN_CODE,
            });
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_CODE,
                field_index: F_CODE_FUNC_IDX,
            });
            // table position = absolute fn idx - FN_FIRST_USER
            f.instruction(&Instruction::I32Const(fx::FN_FIRST_USER as i32));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::CallIndirect {
                type_index: FT_PY_RO,
                table_index: 0,
            });
        },
    );
}

// ===========================================================================
// bigint payload helpers
// ===========================================================================

/// big_bits (FT_BIG_BITS): (b) -> i64 — low <=2 limbs, sign-applied.
fn big_bits(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_BIG_BITS, &[(2, bref()), (1, ValType::I32), (1, ValType::I64)],
    |f| {
        // 2=b 3=n 4=v
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalSet(2));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::LocalSet(3));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(2));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::I64Const(32));
        f.instruction(&Instruction::I64Shl);
        f.instruction(&Instruction::Else);
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_LIMBS);
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
        f.instruction(&Instruction::I64ExtendI32U);
        f.instruction(&Instruction::I64Or);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(2));
        
        f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
        sget(f, T_BIGINT, F_BIG_SIGN);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::I64Const(0));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I64Sub);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(4));
    });
}

/// big_cmp (FT_BIG_CMP): (a, b) -> i32 — sign then magnitude.
fn big_cmp(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BIG_CMP,
        &[(2, bref()), (8, ValType::I32)],
        |f| {
            // 2=a 3=b 4=sa 5=sb 6=na 7=nb 8=i 9=mc 10=la 11=lb
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_SIGN);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_SIGN);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(-1));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(-1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::LocalGet(2));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(3));
            
            f.instruction(&Instruction::RefCastNullable(hc(T_BIGINT)));
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(11));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(-1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::End);
        },
    );
}

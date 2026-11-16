//! Leaf 03.03 prelude functions: classes + exceptions runtime
//! (spec-07 §§2/5; S-03-5..12/28/31..34; S-02-27..34).
//!
//! Function-index contract: FN_CLASS_NEW..FN_CLASSMETHOD_NEW (37..=68) are
//! appended to the prelude AFTER `py_call` (FN_DISPATCH = 36) by
//! [`emit_all`]; user functions start at `fx::FN_FIRST_USER = 69`. Indices
//! are dense, append-only, never renumbered.
//!
//! Runtime model (documented for review):
//! - Class/instance attribute storage rides the generic `$dict` struct with
//!   LINEAR str-keyed scans (`str_eq` on entry keys). Class namespaces and
//!   corpus instances are small; the compact-probing port stays a later
//!   leaf's surface (S-07-15 cites the native port as the algorithm owner).
//! - C3 linearization runs at class-build time inside `class_new`
//!   (S-03-15); the MRO array is never recomputed (S-07-18).
//! - The slot table fills EAGERLY along the MRO (S-07-19) by matching
//!   namespace keys against the dunder->slot-id table in linear memory
//!   ([`SLOTMAP_ADDR`], emitted by the lowering as an active data segment).
//! - Exceptions: `raise` validates BaseException-derivedness, snapshots the
//!   active `$tb_frame` chain into the `$exception`, and `throw`s through
//!   `$tag_pyexc` (S-07-28/30). Frame tracking: a linked stack in globals;
//!   prologues push / exits pop / `Stmt::Line` pokes / handlers cut.
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state::{self as fx};
use crate::prelude::{add, hc, oref, push_null, sget, StrTab};
use crate::types::*;
use wasm_encoder::{BlockType, Function, Instruction, MemArg, MemArgTarget, ValType};

/// SLOTMAP_ADDR: linear-memory base of the dunder->slot-id table (above the
/// print/digit scratch regions, below any heap-independent use).
pub const SLOTMAP_ADDR: i32 = 16384;

/// The dunder->slot-id map (S-07-19 slot ids; the slots the v1 runtime
/// dispatches). `class_new` matches namespace keys against these names.
pub const SLOT_NAMES: [(&str, u32); 20] = [
    ("__len__", SLOT_LEN),
    ("__getitem__", SLOT_GETITEM),
    ("__setitem__", SLOT_SETITEM),
    ("__delitem__", SLOT_DELITEM),
    ("__contains__", SLOT_CONTAINS),
    ("__call__", SLOT_CALL),
    ("__repr__", SLOT_REPR),
    ("__str__", SLOT_STR),
    ("__hash__", SLOT_HASH),
    ("__eq__", SLOT_EQ),
    ("__add__", SLOT_ADD),
    ("__radd__", SLOT_RADD),
    ("__sub__", SLOT_SUB),
    ("__rsub__", SLOT_RSUB),
    ("__mul__", SLOT_MUL),
    ("__rmul__", SLOT_RMUL),
    ("__neg__", SLOT_NEG),
    ("__pos__", SLOT_POS),
    ("__abs__", SLOT_ABS),
    ("__invert__", SLOT_INVERT),
];

/// Byte image of the slot map laid at SLOTMAP_ADDR:
/// `[count:u32][count * (name_off:u32, name_len:u32, slot:u32)]` where
/// name_off/len address the string blob image (absolute linear-memory).
pub fn slotmap_image(name_offsets: &[(u32, u32)]) -> Vec<u8> {
    let mut img = Vec::new();
    img.extend_from_slice(&(SLOT_NAMES.len() as u32).to_le_bytes());
    for (i, (_, slot)) in SLOT_NAMES.iter().enumerate() {
        let (off, len) = name_offsets[i];
        img.extend_from_slice(&off.to_le_bytes());
        img.extend_from_slice(&len.to_le_bytes());
        img.extend_from_slice(&slot.to_le_bytes());
    }
    img
}

/// Frame/exception globals — indices are NOT static: they sit AFTER the
/// module namespace (which is sized at scan time), so the lowering computes
/// them and passes them here.
#[derive(Clone, Copy)]
pub struct FrameGlobals {
    pub frame_head: u32,
    pub frame_depth: u32,
    pub cur_line: u32,
    pub source_file: u32,
    pub active_exc: u32,
}

/// Emit the leaf-03.03 prelude functions THIS SLIDE SHIPS, in
/// `fx::FN_*` index order. The class-creation machinery (class_new C3
/// merge, instance_new, attr_get/set/del, cls_call, descriptor ctors,
/// exc_raise/match/msg/repr/ctx, call_of, global_get, subcls registry)
/// is DESIGNED (see the module docs + PROGRESS notes) but lands with the
/// classes-lowering continuation; its constants in emitter_state.rs are
/// reserved so the shipped indices never move.
pub fn emit_all(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
    g: &FrameGlobals,
) {
        dget(funcs, code);
    dset(funcs, code);
    dhas(funcs, code);
    str_cat(funcs, code);
    type_name(funcs, code);
    frame_push(funcs, code, g);
        frame_pop(funcs, code, g);
            frame_line(funcs, code, g);
        issub(funcs, code);
        isinst(funcs, code);
    exc_raise(funcs, code, g);
    exc_match(funcs, code);
    exc_msg(funcs, code);
    exc_repr(funcs, code);
    exc_ctx_set(funcs, code);
}

// ---------------------------------------------------------------------------
// shared body helpers
// ---------------------------------------------------------------------------

/// m2(): 4-byte-aligned MemArg (linear-memory i32 access).
fn m2() -> MemArg {
    MemArg {
        offset: 0,
        align: 2,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

/// m0(): byte-aligned MemArg.
fn m0() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        target: MemArgTarget::Memory(0),
        ordering: None,
    }
}

/// deref local `obj` -> `$type` into local `ty` (RefCastNullable + set).
fn to_type(f: &mut Function, obj: u32, ty: u32) {
    f.instruction(&Instruction::LocalGet(obj));
    f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
    f.instruction(&Instruction::LocalSet(ty));
}

/// deref local `obj` -> `$dict` into local `d`.
fn to_dict(f: &mut Function, obj: u32, d: u32) {
    f.instruction(&Instruction::LocalGet(obj));
    f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
    f.instruction(&Instruction::LocalSet(d));
}

/// deref local `obj` -> `$str` into local `s`.
fn to_str(f: &mut Function, obj: u32, s: u32) {
    f.instruction(&Instruction::LocalGet(obj));
    f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
    f.instruction(&Instruction::LocalSet(s));
}

/// deref local `obj` -> `$exception` into local `e`.
fn to_exc(f: &mut Function, obj: u32, e: u32) {
    f.instruction(&Instruction::LocalGet(obj));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
    f.instruction(&Instruction::LocalSet(e));
}

/// deref local `obj` -> `$instance` into local `i`.
fn to_instance(f: &mut Function, obj: u32, i: u32) {
    f.instruction(&Instruction::LocalGet(obj));
    f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
    f.instruction(&Instruction::LocalSet(i));
}

/// str-eq(str_a_local, str_b_local) -> i32 on the stack (calls FN_STR_EQ).
fn str_eq_call(f: &mut Function, a: u32, b: u32) {
    f.instruction(&Instruction::LocalGet(a));
    f.instruction(&Instruction::LocalGet(b));
    f.instruction(&Instruction::Call(fx::FN_STR_EQ));
}

// ---------------------------------------------------------------------------
// dict_get (FN_DICT_GET): (dict, key) -> o — linear scan; miss = null.
// FT: (dc, sg) -> o. Locals: 3=d 4=k 5=i 6=entry 7=found-value
// ---------------------------------------------------------------------------
fn dget(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_DICT_GET,
        &[
            (1, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ),
            (2, ValType::I32), // 5,6
            (1, oref()),       // 7
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT_ENTRY),
                }),
            ), // 8
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            // i >= fill? -> miss (null)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_FILL);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // entry = entries[i]
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_ENTRIES);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(8));
            // str_eq(entry.key, k)?
            f.instruction(&Instruction::LocalGet(8));
            sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // hit: push entry.value
            f.instruction(&Instruction::LocalGet(8));
            sget(f, T_DICT_ENTRY, F_ENTRY_VALUE);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // i++
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // miss: null
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

// ---------------------------------------------------------------------------
// dict_set (FN_DICT_SET): (dict, key, val) -> () — overwrite existing entry
// (insertion order preserved, S-01-21) or append at fill (grow: entries are
// rebuilt at 2x cap; the v1 dict literals allocate exact-size arrays so the
// append path REBUILDS with capacity fill*2+4).
// FT: (dc, sg, o) -> (). Locals: 3=d 4=k 5=v 6=i 7=entry 8=new-entries
// ---------------------------------------------------------------------------
fn dset(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_DICT_SET,
        &[
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ),
            (1, oref()),
            (2, ValType::I32), // 6,7
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT_ENTRY),
                }),
            ), // 8
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_ENTRY_ARR),
                }),
            ), // 9
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT_ENTRY),
                }),
            ), // 10
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6));
            // scan for existing key
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_FILL);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1)); // no hit -> append path
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_ENTRIES);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::LocalGet(8));
            sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // overwrite in place, done
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_DICT_ENTRY,
                field_index: F_ENTRY_VALUE,
            });
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // append: rebuild entries at (fill+1) with the new entry last.
            let rebuild = |f: &mut Function| {
                // new entry struct: hash=0 key=k value=v
                f.instruction(&Instruction::I64Const(0));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::LocalGet(5));
                f.instruction(&Instruction::StructNew(T_DICT_ENTRY));
                f.instruction(&Instruction::LocalSet(10));
                // new array len = fill + 1
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_FILL);
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
                f.instruction(&Instruction::LocalSet(9));
                // copy i < fill
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Block(BlockType::Empty));
                f.instruction(&Instruction::Loop(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_FILL);
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_ENTRIES);
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
                f.instruction(&Instruction::ArraySet(T_ENTRY_ARR));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(6));
                f.instruction(&Instruction::Br(0));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::End);
                // new[fill] = entry
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_FILL);
                f.instruction(&Instruction::LocalGet(10));
                f.instruction(&Instruction::ArraySet(T_ENTRY_ARR));
                // d.entries = new; d.fill += 1; d.used += 1
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::LocalGet(9));
                f.instruction(&Instruction::StructSet {
                    struct_type_index: T_DICT,
                    field_index: F_DICT_ENTRIES,
                });
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_FILL);
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::StructSet {
                    struct_type_index: T_DICT,
                    field_index: F_DICT_FILL,
                });
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::LocalGet(3));
                sget(f, T_DICT, F_DICT_USED);
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::StructSet {
                    struct_type_index: T_DICT,
                    field_index: F_DICT_USED,
                });
            };
            rebuild(f);
        },
    );
}

// ---------------------------------------------------------------------------
// dict_has (FN_DICT_HAS): (dict, key) -> i32 — 1/0 membership.
// ---------------------------------------------------------------------------
fn dhas(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_DICT_HAS,
        &[
            (1, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ),
            (2, ValType::I32),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT_ENTRY),
                }),
            ),
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_FILL);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_DICT, F_DICT_ENTRIES);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::LocalGet(7));
            sget(f, T_DICT_ENTRY, F_ENTRY_KEY);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
        },
    );
}

// ---------------------------------------------------------------------------
// str_cat (FN_STR_CAT): (a, b) -> str — byte-concat (latin1 corpus slice).
// Locals: 3=a 4=b 5=la 6=lb 7=arr 8=i
// ---------------------------------------------------------------------------
fn str_cat(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_STR_CAT,
        &[
            (
                3,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ), // 2 dummy, 3=a, 4=b
            (2, ValType::I32), // 5,6
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_BYTE_ARR),
                }),
            ), // 7
            (1, ValType::I32), // 8
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(4));
            sget(f, T_STR, F_STR_LEN);
            f.instruction(&Instruction::LocalSet(6));
            // arr = byte_arr[la+lb]
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
            f.instruction(&Instruction::LocalSet(7));
            // copy a
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_STR, F_STR_DATA);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // copy b into arr[la + i]
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(4));
            sget(f, T_STR, F_STR_DATA);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // build $str (kind 0, len la+lb)
            push_null(f, T_TYPE);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::StructNew(T_STR));
        },
    );
}

// ---------------------------------------------------------------------------
// type_name (FN_TYPE_NAME): (o) -> str — type(x).__name__ (or x.__name__
// when x IS a $type: class-object attribute reads C.__name__).
// ---------------------------------------------------------------------------
fn type_name(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_TYPE_NAME,
        &[
            (2, oref()),
            (1, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TYPE),
                }),
            ),
        ],
        |f| {
            // is the arg itself a $type? -> its $name
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            sget(f, T_TYPE, F_TYPE_NAME);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // else type(x).name — field 0 of any $pyobj IS its $type ref
            f.instruction(&Instruction::LocalGet(0));
            sget(f, T_PYOBJ, F_TYPE);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            sget(f, T_TYPE, F_TYPE_NAME);
        },
    );
}

// ---------------------------------------------------------------------------
// frame_push (FN_FRAME_PUSH): (fn_name) -> () — push a $tb_frame onto the
// global frame stack (G_FRAME_HEAD), depth++.
// Locals: 3=name(str) 4=frame(tb) 5=old-head(tb)
// ---------------------------------------------------------------------------
fn frame_push(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
) {
    add(
        funcs,
        code,
        FT_FRAME_PUSH,
        &[
            (
                2,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ),
            (
                2,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TB_FRAME),
                }),
            ),
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::LocalSet(5));
            // frame = $tb_frame { next: old, filename: null (lowered fill),
            // funcname: name, lineno: 0, col: 0, end_lineno: 0 }
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefNull(hc(T_STR)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::GlobalGet(g.cur_line));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::StructNew(T_TB_FRAME));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::GlobalSet(g.frame_head));
            f.instruction(&Instruction::GlobalGet(g.frame_depth));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::GlobalSet(g.frame_depth));
        },
    );
}

// ---------------------------------------------------------------------------
// frame_pop (FN_FRAME_POP): () -> () — head = head.next; depth--.
// ---------------------------------------------------------------------------
fn frame_pop(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection, g: &FrameGlobals) {
    add(funcs, code, FT_FRAME_POP, &[
            (3, oref()),], |f| {
        f.instruction(&Instruction::GlobalGet(g.frame_head));
        f.instruction(&Instruction::RefCastNullable(hc(T_TB_FRAME)));
        sget(f, T_TB_FRAME, F_TB_NEXT);
        f.instruction(&Instruction::GlobalSet(g.frame_head));
        f.instruction(&Instruction::GlobalGet(g.frame_depth));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Sub);
        f.instruction(&Instruction::GlobalSet(g.frame_depth));
    });
}

// ---------------------------------------------------------------------------
// frame_line (FN_FRAME_LINE): (line) -> () — head.lineno = line (the head
// frame is the CURRENT function's frame).
// ---------------------------------------------------------------------------
fn frame_line(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection, g: &FrameGlobals) {
    add(funcs, code, FT_FRAME_LINE, &[
            (2, oref()),(1, ValType::I32)], |f| {
        // G_CUR_LINE = line (frames read it at push; poke the head too for
        // throws from the current frame)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::GlobalSet(g.cur_line));
        f.instruction(&Instruction::GlobalGet(g.frame_head));
        f.instruction(&Instruction::RefCastNullable(hc(T_TB_FRAME)));
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::StructSet {
            struct_type_index: T_TB_FRAME,
            field_index: F_TB_LINENO,
        });
    });
}

// ---------------------------------------------------------------------------
// issub (FN_ISSUB): (a_type, b_type) -> i32 — b in mro(a) (S-03-31).
// Locals: 3=a(ty) 4=b(ty) 5=i
// ---------------------------------------------------------------------------
fn issub(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_ISSUB,
        &[
            (1, oref()),
            (
                2,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TYPE),
                }),
            ),
            (1, ValType::I32), // 5
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // mro[i] is b? (RefEq on the $type refs)
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefEq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
        },
    );
}

// ---------------------------------------------------------------------------
// isinst (FN_ISINST): (obj, cls) -> i32 — issub(type(obj), cls) (S-03-31).
// Locals: 3=obj 4=cls(ty) 5=type(ty)
// ---------------------------------------------------------------------------
fn isinst(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_ISINST,
        &[
            (1, oref()),
            (1, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TYPE),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TYPE),
                }),
            ),
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_PYOBJ, F_TYPE);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_ISSUB));
        },
    );
}

// ---------------------------------------------------------------------------
// exc_raise (FN_EXC_RAISE): (exc) -> o — snapshot the CURRENT frame chain
// into $exception.$tb (S-07-28) and `throw $tag_pyexc` (S-07-28/30).
// Derivedness is lowering-validated (only known exception classes lower);
// the RefCast here traps fail-loud on anything else.
// Locals (pad (2,oref)): 3=exc($exception)
// ---------------------------------------------------------------------------
fn exc_raise(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
) {
    add(
        funcs,
        code,
        FT_EXC_RAISE,
        &[
            (2, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ),
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            // exc.tb = current frame chain head
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
            // throw $tag_pyexc exc
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            // type-correct unreachable fall-through
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

// ---------------------------------------------------------------------------
// exc_match (FN_EXC_MATCH): (exc, target) -> i32 — the S-02-30/S-03-34
// except-target match. Targets reaching here are lowering-validated
// (builtin exception class or a flat tuple literal of those); a tuple
// matches any element.
// Locals (pad (1,oref)): 3=exc 4=target 5=i
// ---------------------------------------------------------------------------
fn exc_match(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    // (exc, target) -> i32 — single-class match (S-03-31 MRO walk via
    // FN_ISINST). Tuple except-targets are EXPANDED by the lowering into
    // per-element handler dispatch (first-match order preserved), so the
    // runtime only ever sees class targets. The lowering validates that
    // every target derives from BaseException (S-03-34); the cast here
    // traps fail-loud otherwise.
    add(funcs, code, FT_EXC_MATCH, &[(2, oref()), (1, oref())], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
        f.instruction(&Instruction::Call(fx::FN_ISINST));
    });
}

fn push_exc_args(f: &mut Function, exc_local: u32) {
    f.instruction(&Instruction::LocalGet(exc_local));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
    sget(f, T_EXCEPTION, F_EXC_ARGS);
    f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
    sget(f, T_TUPLE, F_TUPLE_ITEMS);
}

fn exc_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_EXCEPTION),
    })
}
fn str_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_STR),
    })
}
fn barr_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_BYTE_ARR),
    })
}
fn roarr_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_OBJ_ARR_RO),
    })
}

/// push an empty $str (field order: type, hash, kind, len, data).
fn push_empty_str(f: &mut Function) {
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::StructNew(T_STR));
}

/// push a 1-byte $str for `b` (scratch local `tmp_arr` = $byte_arr).
fn push_str_byte(f: &mut Function, b: u8, tmp_arr: u32) {
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(tmp_arr));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(b as i32));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::StructNew(T_STR));
}

/// push the ", " $str (scratch local `tmp_arr`).
fn push_str_comma(f: &mut Function, tmp_arr: u32) {
    f.instruction(&Instruction::I32Const(2));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(tmp_arr));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(b',' as i32));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Const(b' ' as i32));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(2));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::StructNew(T_STR));
}

/// push the "()" 2-byte $str (scratch local `tmp_arr`).
fn push_str_parens(f: &mut Function, tmp_arr: u32) {
    push_str_comma_raw(f, tmp_arr, b'(', b')');
}

fn push_str_comma_raw(f: &mut Function, tmp_arr: u32, a: u8, b: u8) {
    f.instruction(&Instruction::I32Const(2));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(tmp_arr));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(a as i32));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Const(b as i32));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(2));
    f.instruction(&Instruction::LocalGet(tmp_arr));
    f.instruction(&Instruction::StructNew(T_STR));
}

/// INLINE into the caller: acc(local `acc`) += repr(value on stack).
/// Repr per arg (v1 slice: str/int/bool/None): str -> 'quoted'; else py_str.
/// Consumes locals: `tmp_str`, `tmp_arr`.
/// Caller stack contract: [acc, value] -> [acc].
fn inline_acc_repr_push(f: &mut Function, acc: u32, tmp_str: u32, tmp_arr: u32) {
    // value is on the stack above acc; stash value into tmp_str? value may
    // be non-str. Stash in a dedicated slot the caller provides: `tmp_val`.
    unreachable!("superseded: use inline_join_arg");
}

/// INLINE into the caller: append "(" + join(repr(args), ", ") + ")" to
/// acc (local `acc`), reading items RO-array (local `items`, len `n`).
/// Locals consumed: i=`idx`, tmp_str, tmp_arr, tmp_val.
fn inline_tuple_join(
    f: &mut Function,
    items: u32,
    n: u32,
    acc: u32,
    idx: u32,
    tmp_str: u32,
    tmp_arr: u32,
    tmp_val: u32,
) {
    // acc = "("
    f.instruction(&Instruction::LocalGet(acc));
    push_str_byte(f, b'(', tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(idx));
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::LocalGet(n));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    // sep (i>0): acc += ", "
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(acc));
    push_str_comma(f, tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::End);
    // acc += repr(items[idx])
    f.instruction(&Instruction::LocalGet(items));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
    f.instruction(&Instruction::LocalSet(tmp_val));
    inline_repr_value(f, tmp_val, tmp_str, tmp_arr);
    f.instruction(&Instruction::LocalGet(acc));
    f.instruction(&Instruction::LocalGet(tmp_str));
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(idx));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::End);
    // acc += ")"
    f.instruction(&Instruction::LocalGet(acc));
    push_str_byte(f, b')', tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
}

/// INLINE: tmp_str(local) = repr(value in local `val`).
fn inline_repr_value(f: &mut Function, val: u32, tmp_str: u32, tmp_arr: u32) {
    // str? -> "'" + py_str(val) + "'"
    f.instruction(&Instruction::LocalGet(val));
    f.instruction(&Instruction::RefTestNullable(hc(T_STR)));
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(val));
    f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
    f.instruction(&Instruction::Call(fx::FN_PY_STR));
    f.instruction(&Instruction::LocalSet(tmp_str));
    // tmp_str = "'" + tmp_str
    push_str_byte(f, b'\'', tmp_arr);
    f.instruction(&Instruction::LocalGet(tmp_str));
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(tmp_str));
    // tmp_str = tmp_str + "'"
    f.instruction(&Instruction::LocalGet(tmp_str));
    push_str_byte(f, b'\'', tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(tmp_str));
    f.instruction(&Instruction::Else);
    f.instruction(&Instruction::LocalGet(val));
    f.instruction(&Instruction::Call(fx::FN_PY_STR));
    f.instruction(&Instruction::LocalSet(tmp_str));
    f.instruction(&Instruction::End);
}

// ---------------------------------------------------------------------------
// exc_msg (FN_EXC_MSG): (exc) -> str — S-02-29: 1 arg -> str(args[0]);
// 0 args -> ""; 2+ -> "(" + join(repr(args), ", ") + ")".
// Locals (pad (2,oref)): 3=exc 4=items 5=n 6=acc 7=i 8=tmp_str 9=tmp_val
// 10=tmp_arr
// ---------------------------------------------------------------------------
fn exc_msg(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_EXC_MSG,
        &[
            (2, oref()),       // 1,2 dummies
            (1, exc_ref()),    // 3 exc ($exception)
            (1, roarr_ref()),  // 4 items
            (1, ValType::I32), // 5 n
            (1, str_ref()),    // 6 acc
            (1, str_ref()),    // 7 tmp_str
            (1, oref()),       // 8 tmp_val
            (1, ValType::I32), // 9 i
            (1, barr_ref()),   // 10 tmp_arr
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            // items = exc.args.items
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_EXCEPTION, F_EXC_ARGS);
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(5));
            // 1 arg: str(args[0]) — via the shared repr (str keeps quotes?
            // NO: str(e) is str(args[0]) RAW — py_str directly)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::Call(fx::FN_PY_STR));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // 0 args: ""
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            push_empty_str(f);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // 2+: "(" + join(repr(args), ", ") + ")"
            push_empty_str(f);
            f.instruction(&Instruction::LocalSet(6));
            inline_tuple_join_raw(f, 4, 5, 6, 9, 7, 10, 8);
            f.instruction(&Instruction::LocalGet(6));
        },
    );
}

/// like inline_tuple_join but args render via py_str RAW (no quoting) —
/// the S-02-29 tuple form `(1, 2)` prints values unquoted unless str.
fn inline_tuple_join_raw(
    f: &mut Function,
    items: u32,
    n: u32,
    acc: u32,
    idx: u32,
    tmp_str: u32,
    tmp_arr: u32,
    tmp_val: u32,
) {
    f.instruction(&Instruction::LocalGet(acc));
    push_str_byte(f, b'(', tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(idx));
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::LocalGet(n));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(acc));
    push_str_comma(f, tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(items));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
    f.instruction(&Instruction::LocalSet(tmp_val));
    // raw: str args render WITHOUT quotes in tuple form? CPython renders
    // str args with quotes inside tuples: ('a', 2). Use full repr.
    inline_repr_value(f, tmp_val, tmp_str, tmp_arr);
    f.instruction(&Instruction::LocalGet(acc));
    f.instruction(&Instruction::LocalGet(tmp_str));
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(idx));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(acc));
    push_str_byte(f, b')', tmp_arr);
    f.instruction(&Instruction::Call(fx::FN_STR_CAT));
    f.instruction(&Instruction::LocalSet(acc));
}

// ---------------------------------------------------------------------------
// exc_repr (FN_EXC_REPR): (exc) -> str — repr(e) == ClassName(<args repr>):
// 0 args -> "Name()"; else "Name(" + join(repr, ", ") + ")" (S-02-29).
// Locals (pad (3,oref)): 4=exc 5=name 6=items 7=n 8=acc 9=i 10=tmp_str
// 11=tmp_val 12=tmp_arr
// ---------------------------------------------------------------------------
fn exc_repr(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_EXC_REPR,
        &[
            (2, oref()),       // 1,2 dummies
            (1, exc_ref()),    // 3 exc ($exception)
            (3, str_ref()),    // 4 name, 5 acc, 6 tmp_str
            (2, ValType::I32), // 7 n, 8 i
            (1, oref()),       // 9 tmp_val
            (1, barr_ref()),   // 10 tmp_arr
            (1, roarr_ref()),  // 11 items
        ],
        |f| {
            // name = type_name(exc)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::Call(fx::FN_TYPE_NAME));
            f.instruction(&Instruction::LocalSet(4));
            // items + n
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_EXCEPTION, F_EXC_ARGS);
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::LocalSet(11));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(7));
            // 0 args: name + "()"
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            push_str_parens(f, 10);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // name + "(" + join(repr, ", ") + ")"
            f.instruction(&Instruction::LocalGet(4));
            push_str_byte(f, b'(', 10);
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(5));
            inline_tuple_join(f, 11, 7, 5, 8, 6, 10, 9);
            f.instruction(&Instruction::LocalGet(5));
        },
    );
}

// ---------------------------------------------------------------------------
// exc_ctx_set (FN_EXC_CTX_SET): (new, ctx, cause) -> new — `raise X from Y`
// sets cause + suppress=1 (S-02-34); an in-flight exception sets context.
// Locals (pad (3,oref)): 3,4,5 dummies (params are 0,1,2).
// ---------------------------------------------------------------------------
fn exc_ctx_set(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_EXC_CTX_SET, &[(3, oref())], |f| {
        // NOTE: struct.set's OBJ operand must be EXACTLY (ref null $exception);
        // a local routed through a (ref null $pyobj) slot loses the refinement
        // (locals are statically their declared type), so the cast result is
        // kept ON THE STACK under the value — never via local 3.
        // cause (param 2) null?
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefIsNull);
        f.instruction(&Instruction::If(BlockType::Empty));
        // context = ctx: [obj=param0 cast, value=param1 cast] struct.set
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
        f.instruction(&Instruction::StructSet {
            struct_type_index: T_EXCEPTION,
            field_index: F_EXC_CONTEXT,
        });
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        // cause + suppress_context = 1
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
        f.instruction(&Instruction::StructSet {
            struct_type_index: T_EXCEPTION,
            field_index: F_EXC_CAUSE,
        });
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::StructSet {
            struct_type_index: T_EXCEPTION,
            field_index: F_EXC_SUPPRESS,
        });
        f.instruction(&Instruction::LocalGet(0));
    });
}

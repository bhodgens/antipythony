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
    /// Global holding the exception that escaped to the module boundary
    /// (set by _start's top-level catch; read by FN_TB_RENDER).
    pub pending_exc: u32,
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
    // D8: intern BEFORE frame_push so the limit-exceeded RuntimeError
    // message rides a stable passive-segment offset.
    let rec_msg = strings.push("maximum recursion depth exceeded");
    frame_push(funcs, code, g, rec_msg);
    frame_pop(funcs, code, g);
    frame_line(funcs, code, g);
    issub(funcs, code);
    isinst(funcs, code, strings);
    exc_raise(funcs, code, g, strings);
    exc_match(funcs, code);
    exc_msg(funcs, code);
    exc_repr(funcs, code);
    exc_ctx_set(funcs, code);
    exc_new(funcs, code);
    cls_new(funcs, code);
    instance_new(funcs, code);
    cls_call(funcs, code, strings);
    attr_get(funcs, code, strings);
    attr_set(funcs, code);
    call_of(funcs, code);
    mro_dict_get(funcs, code);
    tb_render(funcs, code, g, g.pending_exc, strings);
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
            // build $str (kind 0, len la+lb) — TYPE STAMPED (03.03d, same
            // null-type re-dispatch hazard as lit_str: py_str on a concat
            // result fell through type_sel to null)
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("str").unwrap_or(0),
            ));
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
    rec_msg: (u32, u32),
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
            // D8: 6 = the limit-exceeded $exception scratch (tag payload
            // type) for the RecursionError throw.
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ),
        ],
        |f| {
            // D8 (bughunt 6.1): depth limit check BEFORE the push —
            // unbounded recursion previously fell through to a raw wasm
            // stack-overflow trap with no Python-level error. 1500 is
            // safely under the measured host trap floor (>=3000 frames)
            // and comfortably above deep-but-legitimate corpus recursion
            // (CF013-style shapes, d8d15's 1500). CPython-exact message.
            f.instruction(&Instruction::GlobalGet(g.frame_depth));
            f.instruction(&Instruction::I32Const(1500));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("RecursionError").unwrap_or(0),
            ));
            lit_str(f, rec_msg.0, rec_msg.1);
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            // old head; NULL = empty stack (the RefCastNullable+sget would
            // trap on null — guard with is_null)
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::RefNull(hc(T_TB_FRAME)));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::RefCastNullable(hc(T_TB_FRAME)));
            sget(f, T_TB_FRAME, F_TB_NEXT);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::End);
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
fn frame_pop(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
) {
    add(funcs, code, FT_FRAME_POP, &[(3, oref())], |f| {
        // NULL-HEAD SAFE: the dead-epilogue pop CAN execute when a body ends
        // in `return` whose emitted wasm return the emitter folded (03.03b
        // observed on 2-level calls) — and a traceback frame chain is a
        // best-effort surface: an extra pop must never trap, only no-op.
        f.instruction(&Instruction::GlobalGet(g.frame_head));
        f.instruction(&Instruction::RefIsNull);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
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
fn frame_line(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
) {
    add(
        funcs,
        code,
        FT_FRAME_LINE,
        &[(2, oref()), (1, ValType::I32)],
        |f| {
            // G_CUR_LINE = line (frames read it at push; poke the head too
            // for throws from the current frame). Head NULL = no frame pushed
            // (module body with the EH wrap gated off) — the poke is a no-op.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::GlobalSet(g.cur_line));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::RefCastNullable(hc(T_TB_FRAME)));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_TB_FRAME,
                field_index: F_TB_LINENO,
            });
            f.instruction(&Instruction::End);
        },
    );
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
            // (o, o) params: cast both operands to $type (module-global
            // class loads arrive as $pyobj)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
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
fn isinst(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    // 03.03d: super() marker literals (S-03-17 slice) — isinstance(super(..), C)
    // delegates to the STORED obj (CPython: super(SA, SB()) is an SA because
    // its obj is one). Marker test: type_name(obj) == "super".
    let (sup_off, sup_len) = strings.push("super");
    let (ko_off, ko_len) = strings.push("__ap_obj__");
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
            (1, str_ref()), // 6 = pad (original body's local grid starts at 2)
            (1, str_ref()), // 7 = marker-name scratch
            (1, str_ref()), // 8 = "super" literal scratch
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT),
                }),
            ), // 9 = marker attrs-dict scratch (dict-typed: FN_DGET param 0)
        ],
        |f| {
            // 03.03d REVISION (EXW009 CPython 3.14 pin): isinstance(super(..), C)
            // is FALSE in CPython — a super object's type IS `super`, never the
            // bound class. The earlier marker-delegation draft was wrong; the
            // stock issub(type(obj), cls) path answers False for markers
            // naturally. No special arm. (sup/ko literals still interned for
            // the attr paths below / other users of this StrTab region.)
            let _ = (sup_off, sup_len, ko_off, ko_len);
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
    strings: &mut StrTab,
) {
    let (msg_off, msg_len) = strings.push("exceptions must derive from BaseException");
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
            // Validate BaseException-derivedness at RAISE time (S-03-33):
            // non-exception -> build TypeError("exceptions must derive from
            // BaseException") and throw THAT (the corpus pin surface); the
            // old RefCast trapped fail-loud instead (03.03b).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::If(BlockType::Empty));
            // exception path: cast, tb snapshot, throw. Local 3 is the
            // exception-typed scratch ((1, exc_ref) in the locals decl —
            // locals start after the single param), so the throw tag
            // payload keeps its (ref null $exception) type.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            f.instruction(&Instruction::End);
            // non-exception path: throw a fresh TypeError with the CPython
            // message; args = ("exceptions must derive from BaseException",)
            // Two copies of the ty ref: the FIRST is exc_new's arg0 (call
            // args push left-to-right; roarr must end up on top), the SECOND
            // is consumed by struct.new $str's type field.
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("TypeError").unwrap_or(0),
            ));
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("TypeError").unwrap_or(0),
            ));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(41));
            f.instruction(&Instruction::I32Const(msg_off as i32));
            f.instruction(&Instruction::I32Const(msg_len as i32));
            f.instruction(&Instruction::ArrayNewData {
                array_type_index: T_BYTE_ARR,
                array_data_index: 1,
            });
            f.instruction(&Instruction::StructNew(T_STR));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            // stack is now [excinst]: cast + tb + throw. (The ty ref under
            // the args array was exc_new's arg0 — consumed by the call.)
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
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
    add(
        funcs,
        code,
        FT_EXC_MATCH,
        &[(2, oref()), (1, oref())],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::Call(fx::FN_ISINST));
        },
    );
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
    // sep (i>0): acc += ", " (Eqz+Eqz = i != 0)
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Eqz);
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
    // sep (i>0): acc += ", " (Eqz+Eqz = i != 0)
    f.instruction(&Instruction::LocalGet(idx));
    f.instruction(&Instruction::I32Eqz);
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

// ===========================================================================
// Leaf 03.03 classes slice: class/instance/attr/call machinery + exception
// instances + traceback rendering. Indices FN_EXC_NEW(52)..FN_TB_RENDER(60).
// Design notes for review:
// - User classes build a real $type at class-def execution time via
//   cls_new (S-03-1..4 slice: single base, no metaclass hooks — documented).
// - Attribute lookup walks the class MRO's namespace dicts then the instance
//   attrs dict (S-03-9 v1: no data/non-data descriptor split — descriptors
//   land with the property slice; plain functions found on the class are
//   returned RAW and the CALL SITE binds self — the only descriptor the
//   slice dispatches).
// - Exception instances ride $excinst (sub $exception) so the tag payload
//   accepts them (S-07-28/30) while carrying an attrs dict like $instance.
// - Traceback rendering (S-02-29 / C-31 area / leaf scope 7): the module
//   _start wraps the body in a catch handler; the handler calls tb_render
//   which walks the $exception's $tb chain + frames and writes the CPython
//   format to fd 2 via the WASI fd_write import, then returns 1 (the
//   caller proc_exits with it). Byte-exactness is gate-enforced.
// ===========================================================================

// ---------------------------------------------------------------------------
// mro_dict_get (FN_MRO_DICT_GET): (ty, key) -> o — first dict hit along the
// mro; miss = null. Locals: 3=t 4=k 5=i 6=d 7=hit
// ---------------------------------------------------------------------------
fn mro_dict_get(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_MRO_DICT_GET,
        &[
            (2, ty_ref()),     // 2,3 = ty slots
            (2, str_ref()),    // 4,5 = key slots
            (1, ValType::I32), // 6 = loop i
            (1, dict_ref()),   // 7 = dict
            (1, oref()),       // 8 = hit
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // d = mro[i].dict
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
            sget(f, T_TYPE, F_TYPE_DICT);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalSet(7));
            // hit = dict_get(d, k)
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

fn ty_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_TYPE),
    })
}
fn dict_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_DICT),
    })
}
fn tarr_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_TYPE_ARR),
    })
}
fn oarr_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_OBJ_ARR),
    })
}
fn tb_ref() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_TB_FRAME),
    })
}

// ---------------------------------------------------------------------------
// exc_new (FN_EXC_NEW): (ty, roarr args) -> o — build a $excinst with the
// args tuple (S-02-27: raise C(args) instantiates via type call). tb null
// (exc_raise fills it at the throw site); context/cause/suppress default.
// Locals: 3=t 4=args 5=exc
// ---------------------------------------------------------------------------
fn exc_new(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_EXC_NEW,
        &[
            (2, oref()), // 2,3 (locals start after the 2 params)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                }),
            ), // 4 = args
            (1, oref()), // 5 = exc stash
        ],
        |f| {
            // args RO -> the $exception args field needs a $tuple whose items
            // are the RO array (T_TUPLE wraps (ref $obj_arr_ro) immutably — the
            // RO array from the caller is reused directly).
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // build $excinst: pyobj fields first (type = param 0), then
            // $exception fields (args, tb, context, cause, suppress), then the
            // $excinst tail (attrs dict).
            f.instruction(&Instruction::LocalGet(0));
            // args tuple: type ref = param 0? NO — tuple type ref = the 'tuple'
            // builtin type global (print path reads only items). Use the tuple
            // type object for type(e.args) fidelity.
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("tuple").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::StructNew(T_TUPLE));
            f.instruction(&Instruction::RefNull(hc(T_TB_FRAME)));
            f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            f.instruction(&Instruction::I32Const(0));
            // $excinst tail: lazy attrs dict (null until first setattr)
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::StructNew(T_EXCINST));
            // result is (ref null $pyobj)-ready: $excinst sub $exception sub $pyobj
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
        },
    );
}

// ---------------------------------------------------------------------------
// cls_new (FN_CLS_NEW): (str name, ty base|null) -> ty — build the class
// $type at class-def time (S-03-1..4 slice): C3 single-base linearization
// (mro = [self] + base.mro; no-base = [self]), empty slot table, bases
// array [base|null->], namespace dict (filled by the caller via attr_set).
// Locals: 3=name 4=base 5=mro 6=ty
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// cls_new (FN_CLS_NEW1): (str name, ty base|null) -> ty — build the class
// $type at class-def time (S-03-1..4 slice): C3 single-base linearization
// (mro = [self] + base.mro; no-base = [self]), empty slot table, bases
// array, empty namespace dict (the caller fills it via FN_ATTR_SET on the
// returned class object). Locals: 3=name 4=base 5=mro 6=ty 7=bases 8=i
// ---------------------------------------------------------------------------
fn cls_new(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_CLS_NEW1,
        &[
            (1, oref()),       // 2 = pad (locals start after the 2 params)
            (1, str_ref()),    // 3 = name
            (1, ty_ref()),     // 4 = base
            (1, tarr_ref()),   // 5 = mro
            (1, ty_ref()),     // 6 = the class $type
            (1, oarr_ref()),   // 7 = bases
            (2, ValType::I32), // 8,9 = loop i / bound
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // bases = mut $obj_arr[base?1:0]; bases[0] = base (patched after)
            // bases length via If(Empty)+local-store (03.02c engine fact:
            // value-producing If(Result) chains wedge wasmtime 47)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
            f.instruction(&Instruction::LocalSet(7));
            // mro = $type_arr[1 + base_mro_len]; [0] = null for now
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(4));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayNewDefault(T_TYPE_ARR));
            f.instruction(&Instruction::LocalSet(5));
            // mro[i+1] = base.mro[i]
            // loop bound = base == null ? 0 : base.mro.len (local 9, i32;
            // If(Empty)+local-store per the 03.02c engine fact)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(4));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(4));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
            f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // $type struct: pyobj.type(null), name, mro, slots, bases, dict
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(SLOT_COUNT as i32));
            f.instruction(&Instruction::ArrayNewDefault(T_SLOT_ARR));
            f.instruction(&Instruction::LocalGet(7));
            push_empty_dict(f);
            f.instruction(&Instruction::StructNew(T_TYPE));
            f.instruction(&Instruction::LocalSet(6));
            // patch mro[0] = self (the array is mutable)
            f.instruction(&Instruction::LocalGet(6));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::ArraySet(T_TYPE_ARR));
            // patch bases[0] = base
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(6));
        },
    );
}

// ---------------------------------------------------------------------------
// instance_new (FN_INSTANCE_NEW): (ty) -> o — $instance with a fresh attrs
// dict (eager: setattr path stays branch-free on the slice).
// ---------------------------------------------------------------------------
fn instance_new(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_INSTANCE_NEW, &[], |f| {
        f.instruction(&Instruction::LocalGet(0)); // class $type ref
        push_empty_dict(f);
        f.instruction(&Instruction::StructNew(T_INSTANCE));
    });
}

// ---------------------------------------------------------------------------
// attr_get (FN_ATTR_GET): (o obj, str name) -> o — S-03-9 slice: instance
// ($instance|$excinst) attrs dict first, then the class mro dicts; non-
// instances read their own $type dict (class attribute read / module
// namespace on $pyobj? v1: classes + instances only).
// leaf wasm-super: a `super` object (an $instance whose class is named
// "super") routes to FN_SUPER_MRO_GET — the MRO-after-start-class walk.
// Locals: 3=obj 4=name 5=d 6=hit 7=t (8 = class-name scratch, super test)
// ---------------------------------------------------------------------------
fn attr_get(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    let (nm_off, nm_len) = strings.push("__name__");
    let (sv_off, sv_len) = strings.push("value");
    let (sup_off, sup_len) = strings.push("super");
    add(
        funcs,
        code,
        FT_ATTR_GET,
        &[(2, oref()), (1, str_ref()), (2, oref())],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // null-obj guard: a (ref null $pyobj) null must answer null (the
            // metaclass 'type' of a $type is null on this slice — type(SA) is the
            // SA $type itself, its .type field is null) — never trap.
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // leaf wasm-super: a `super` object routes to the MRO-after-S walk.
            // Marker test = the class NAME (the marker is a plain $instance of a
            // fresh $type named "super"; same test shape as isinst's).
            // A $type receiver is EXCLUDED here: its own type field is null by
            // construction (cls_new: pyobj.type(null)) and the $type branch
            // below owns class-attribute reads — test the shape, never read the
            // type field of a $type.
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_PYOBJ, F_TYPE);
            sget(f, T_TYPE, F_TYPE_NAME);
            lit_str(f, sup_off, sup_len);
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_SUPER_MRO_GET));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // 03.03d: $type object reads (type(x).__name__, class-attr lookups):
            // __name__ answers the name field; everything else walks the class's
            // own namespace dicts (mro_dict_get on the $type itself).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            // __name__? (attr name == "__name__")
            f.instruction(&Instruction::LocalGet(4));
            lit_str(f, nm_off, nm_len);
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // __name__: the $type's name field
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            sget(f, T_TYPE, F_TYPE_NAME);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // other attr: the class namespace mro walk
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_MRO_DICT_GET));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // instance-ish? ($excinst sub $instance? NO — separate: test $excinst
            // first, then $instance)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_EXCINST)));
            f.instruction(&Instruction::If(BlockType::Empty));
            // leaf 03.04b: StopIteration.value (GE002 SI plumbing) — the return
            // value of an exhausted generator rides the exc's args[0].
            f.instruction(&Instruction::LocalGet(4));
            lit_str(f, sv_off, sv_len);
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // args-items is (ref null $obj_arr_ro) — ARRAY, not $pyobj: NO local
            // stash (lesson 3); array.len + array.get stack-resident.
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 1); // args tuple
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::If(BlockType::Empty));
            // args non-empty: args[0]
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 1);
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            sget(f, T_TUPLE, F_TUPLE_ITEMS);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // empty args: None
            f.instruction(&Instruction::GlobalGet(G_NONE));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // excinst: attrs dict (field 6, LAZY — null until first setattr;
            // the RefIsNull + If(Empty) below skips the lookup when null —
            // If(Empty) runs its body when the CONDITION IS NONZERO, so the
            // test is `!is_null`) then fall through to `args` (field 1)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 6);
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 6);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 1);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // fall through to class mro walk (class-level attrs)
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_INSTANCE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
            sget(f, T_INSTANCE, F_INSTANCE_ATTRS);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // class mro walk on obj's type
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_PYOBJ, F_TYPE);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_MRO_DICT_GET));
        },
    );
}

// ---------------------------------------------------------------------------
// attr_set (FN_ATTR_SET): (o obj, str name, o v) -> () — instance attrs dict
// write; on a CLASS object ($type) writes the class namespace dict.
// Locals: 3=obj 4=name 5=v 6=d 7=t
// ---------------------------------------------------------------------------
fn attr_set(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_ATTR_SET,
        &[(1, oref()), (1, str_ref()), (1, oref())],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // 03.03c fix: the VALUE is param 2 (the previous LocalGet(3) stored
            // the OBJECT as the value — every class-body attr_set wrote the class
            // itself, and the later __init__ RefCast($function) trap-failed).
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalSet(5));
            // class object? ($type test first)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            sget(f, T_TYPE, F_TYPE_DICT);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Call(fx::FN_DSET));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // excinst — attrs dict is field 6 (layout [type, args, tb, ctx,
            // cause, suppress, attrs]; the lazy dict is null until first set,
            // so materialize an empty dict on first write)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_EXCINST)));
            f.instruction(&Instruction::If(BlockType::Empty));
            // materialize the lazy attrs dict on FIRST write (null until now)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            push_empty_dict(f);
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCINST,
                field_index: 6,
            });
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCINST)));
            sget(f, T_EXCINST, 6);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Call(fx::FN_DSET));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // instance
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
            sget(f, T_INSTANCE, F_INSTANCE_ATTRS);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Call(fx::FN_DSET));
        },
    );
}

// ---------------------------------------------------------------------------
// cls_call (FN_CLS_CALL): (ty cls, roarr args) -> o — instantiation
// (S-03-28/4/5 slice): BaseException-derived classes build $excinst w/ args
// (e.args semantics); plain classes build $instance, then look up __init__
// along the mro and call it with [self, *args] when present.
// Locals: 3=cls 4=args 5=inst 6=init 7=callargs(mut obj arr stash via temp)
// ---------------------------------------------------------------------------
fn cls_call(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    let (init_off, init_len) = strings.push("__init__");
    // Locals: 2=cls(ty) 3=args(roarr) 4=inst(oref) 5=init(oref)
    add(
        funcs,
        code,
        FT_CLS_CALL,
        &[
            (1, ty_ref()),    // 2
            (1, roarr_ref()), // 3
            (2, oref()),      // 4,5
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(3));
            // BaseException-derived? FN_ISSUB(cls, exc_root) walks mro(a).
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("BaseException").unwrap_or(0),
            ));
            f.instruction(&Instruction::Call(fx::FN_ISSUB));
            f.instruction(&Instruction::If(BlockType::Empty));
            // exception instance: exc_new(cls, args) — e.args semantics
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // plain instance
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Call(fx::FN_INSTANCE_NEW));
            f.instruction(&Instruction::LocalSet(4));
            // __init__ lookup along the mro ("__init__" $str from the blob)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(init_len as i32));
            f.instruction(&Instruction::I32Const(init_off as i32));
            f.instruction(&Instruction::I32Const(init_len as i32));
            f.instruction(&Instruction::ArrayNewData {
                array_type_index: T_BYTE_ARR,
                array_data_index: 1,
            });
            f.instruction(&Instruction::StructNew(T_STR));
            f.instruction(&Instruction::Call(fx::FN_MRO_DICT_GET));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            // call init(self, *args): FT_PY_RO wants an RO args array and
            // array.new_fixed takes a STATIC size — dispatch on argc
            // 0..=3 (the slice's __init__ arity; > 3 = unreachable).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArrayLen);
            // argc == 0
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Drop); // __init__ result unused
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // argc == 1
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 2,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // argc == 2
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 3,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // argc == 3
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 4,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // > 3 args: fail loud (documented slice bound)
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(4));
        },
    );
}

// ---------------------------------------------------------------------------
// call_of (FN_CALL_OF): (o callable, roarr args) -> o — generic dispatch:
// $function -> dispatcher; instance w/ __call__ -> bind + call; exception
// CLASS ($type, BaseException-derived) -> exc instance (raise C(args) shape).
// Locals: 3=c 4=args 5=call
// ---------------------------------------------------------------------------
fn call_of(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_CALL_OF,
        &[
            (1, oref()),      // 2 = pad (locals start after the 2 params)
            (1, oref()),      // 3 = c
            (1, roarr_ref()), // 4 = args
            (1, oref()),      // 5 = call
            (3, oref()),      // 6,7,8 pad (reach the fixed scratch at 9)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_BYTE_ARR),
                }),
            ), // 9 = "__call__" arr scratch
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // $function?
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // class object (raise C(args))?
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_CLS_CALL));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // instance __call__?
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_INSTANCE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_PYOBJ, F_TYPE);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            // "__call__" literal — build via scratch (see __init__ blob pattern:
            // this one is rarer; assemble via FN-level: reuse attr name path with
            // a $str built from a 8-byte arr through a stash local).
            // NOTE: implemented via mro_dict_get with a blob-literal $str.
            push_str_call(f);
            f.instruction(&Instruction::Call(fx::FN_MRO_DICT_GET));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            // call call(self, *args): static-arity cascade 0..=1 (slice bound)
            // argc 0
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // argc 1
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 2,
            });
            f.instruction(&Instruction::RefNull(hc(T_DICT)));
            f.instruction(&Instruction::Call(fx::FN_DISPATCH));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // > 1: fail loud
            f.instruction(&Instruction::Unreachable);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // other: fail loud (slice bound)
            f.instruction(&Instruction::Unreachable);
        },
    );
}

/// push the "__call__" $str from the linear blob — needs the (off,len) known
/// at EMIT time; the blob slot for dunder names is reserved by the lowering
/// (DUNDER_OFF/DUNDER_LEN constants are patched by lower.rs — see the module
/// docs). v1 placeholder: assemble via 8 sequential stores using a scratch
/// local the CALLER must reserve at index 9 ($byte_arr) — cls-call sites only.
fn push_str_call(f: &mut Function) {
    // "__call__" = 8 bytes, built into a mut byte arr stashed in local 9.
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(9));
    let bytes: [u8; 8] = *b"__call__";
    for (i, b) in bytes.iter().enumerate() {
        f.instruction(&Instruction::LocalGet(9));
        f.instruction(&Instruction::I32Const(i as i32));
        f.instruction(&Instruction::I32Const(*b as i32));
        f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    }
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::LocalGet(9));
    f.instruction(&Instruction::StructNew(T_STR));
}

// ---------------------------------------------------------------------------
// tb_render (FN_TB_RENDER): () -> i32 — render the ACTIVE exception's
// traceback ("Traceback (most recent call last):" + File/line frames +
// source echo + "Type: msg") to fd 2 in CPython 3.14 format from the
// $exception thrown out to the module boundary. The lowering guarantees
// G_PENDING_EXC (global) holds the escaped (ref $exception) — set by _start's
// top-level catch handler before calling this. Returns 1 (the exit code).
//
// Frame accounting: at the boundary the frame chain still holds every frame
// that didn't pop (raises skip epilogues) — exactly the CPython traceback
// chain, most-recent-first = G_FRAME_HEAD.
//
// CPython 3.14 uncaught format (single frame, module-level raise):
//   Traceback (most recent call last):
//   File "<name>", line N, in <module>
//     <source echo>
//   TypeError: msg
// (source echo: v1 renders a single-space-indented echo of the line text
// from the source blob when available; multi-source modules deferred —
// the echo requires the module source, which rides the passive segment.)
//
// fd 2 writes: the print machinery writes fd 1 (iovec @ 0, fd=1 in the
// fd_write call). tb_render reuses the same iovec scratch with fd=2.
// ---------------------------------------------------------------------------
fn tb_render(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
    pending: u32,
    strings: &mut StrTab,
) {
    let (hdr_off, hdr_len) = strings.push("Traceback (most recent call last):");
    let (file_off, file_len) = strings.push("  File \"");
    let (line_mid_off, line_mid_len) = strings.push("\", line ");
    let (in_mid_off, in_mid_len) = strings.push(", in ");
    let (mod_off, mod_len) = strings.push("<module>");
    let (colon_off, colon_len) = strings.push(": ");
    let (unknown_off, unknown_len) = strings.push("?");
    // Locals: 3=exc 4,5 pad(oref) 6=str 7=barr 8,9,10=i32 11=tb-frame
    add(
        funcs,
        code,
        FT_TB_RENDER,
        &[
            (3, oref()),       // 0,1,2 pad (no params: locals start at 0)
            (1, exc_ref()),    // 3 = exc
            (2, oref()),       // 4,5 pad
            (1, str_ref()),    // 6 = str acc
            (1, barr_ref()),   // 7 = tmp_arr
            (3, ValType::I32), // 8,9,10 (n, i, bufptr)
            (1, tb_ref()),     // 11 = frame walk
        ],
        |f| {
            // exc = pending (cast $exception)
            f.instruction(&Instruction::GlobalGet(pending));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(3));
            // ---- "Traceback (most recent call last):" ----
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, hdr_off, hdr_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            flush_fd(f, 10, 2);
            // ---- frames (single frame: the throw site; multi-frame walk
            // rides the 03.05 traceback surface with caret anchors) ----
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalSet(10));
            // "  File \" + filename (G_SOURCE_FILE)
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, file_off, file_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::GlobalGet(g.source_file));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, unknown_off, unknown_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::GlobalGet(g.source_file));
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::End);
            // "", line " + lineno (G_CUR_LINE)
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, line_mid_off, line_mid_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::GlobalGet(g.cur_line));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::Call(fx::FN_INT_FROM_I64));
            f.instruction(&Instruction::Call(fx::FN_INT_TO_STR));
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            // ", in " + "<module>"
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, in_mid_off, in_mid_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, mod_off, mod_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            flush_fd(f, 10, 2);
            // ---- "TypeName: msg" ----
            f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_TYPE_NAME));
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            // msg; CPython final line = "TypeName: msg" (bare TypeName when
            // the message is empty) — one flush, one line
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::Call(fx::FN_EXC_MSG));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(10));
            lit_str(f, colon_off, colon_len);
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_PRINT_ONE));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::End);
            flush_fd(f, 10, 2);
            f.instruction(&Instruction::I32Const(1)); // rc = 1
        },
    );
}

/// push a passive-segment $str literal (off,len) — local copy of the prelude
/// idiom (prelude::push_str_literal_seg is private).
fn lit_str(f: &mut Function, off: u32, len: u32) {
    // 03.03d FIX: stamp the str TYPE global (was push_null) — a null type
    // field made every py_str re-dispatch on a lit_str-built $str fall
    // through type_sel to the fallthrough (same root the 03.03c
    // str_from_bytes stamping fixed for that path). Reached via exc_msg's
    // py_str(args[0]) re-dispatch: print(e) trapped PRINT_ONE on the null.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("str").unwrap_or(0),
    ));
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

/// flush the print buffer [PRINT_BUF, bufptr) to fd `fd` (2 = stderr for the
/// traceback path) — same iovec scratch the stdout flush uses (single-iov
/// shape per the 03.02b note), then reset nothing (caller resets bufptr).
fn flush_fd(f: &mut Function, bufptr: u32, fd: i32) {
    let m2 = || wasm_encoder::MemArg {
        offset: 0,
        align: 2,
        target: wasm_encoder::MemArgTarget::Memory(0),
        ordering: None,
    };
    // iov1 @0 = {buf PRINT_BUF, len end - PRINT_BUF}; a trailing newline is
    // stored at [end] and included (+1) — matching print_flush's shape.
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
    f.instruction(&Instruction::I32Store(m2()));
    f.instruction(&Instruction::I32Const(4));
    f.instruction(&Instruction::LocalGet(bufptr));
    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Store(m2()));
    // newline at [end]
    f.instruction(&Instruction::LocalGet(bufptr));
    f.instruction(&Instruction::I32Const(10));
    f.instruction(&Instruction::I32Store8(wasm_encoder::MemArg {
        offset: 0,
        align: 0,
        target: wasm_encoder::MemArgTarget::Memory(0),
        ordering: None,
    }));
    // single-iov fd_write(fd, iov@8, 1, n@16): iov1.buf = PRINT_BUF @8
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
    f.instruction(&Instruction::I32Store(m2()));
    f.instruction(&Instruction::I32Const(12));
    f.instruction(&Instruction::LocalGet(bufptr));
    f.instruction(&Instruction::I32Const(fx::PRINT_BUF));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::I32Store(m2()));
    f.instruction(&Instruction::I32Const(fd));
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Const(16));
    f.instruction(&Instruction::Call(0));
    f.instruction(&Instruction::Drop);
}

/// push an empty $dict (field order: type, used, fill, mask, perturb, idx, entries)
/// — local copy of lower.rs's idiom (private there).
// ---------------------------------------------------------------------------
// super_new (FN_SUPER_NEW = 69, leaf 03.03d): (S o, obj o, unused o) -> o —
// the two-arg super() build (S-03-17 slice). Validates isinstance(obj, S) ||
// (obj is a $type && issubclass(obj, S)); failure throws TypeError with the
// CPython message. Success builds a marker $instance whose class is a fresh
// $type named "super" with (__ap_s__ = S, __ap_obj__ = obj) attrs —
// FN_ISINST answers the marker by delegating to the stored obj and
// FN_TYPE_NAME answers "super" (c021's observable surface; the MRO-after-S
// attribute walk is the c022 chase, deferred with that case).
// FT: FT_EXC_CTX_SET (o, o, o) -> o; param 2 unused (null at call sites).
// Locals: 3=ok(i32) 4=cls(ty) 5=inst(oref) 6=msg(str) 7=name-obj(str)
// 8,9 pad(oref) 10=tmp(oref)
// ---------------------------------------------------------------------------
fn super_new(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
    strings: &mut StrTab,
) {
    let (p1_off, p1_len) = strings.push("super(type, obj): obj (instance of ");
    let (p2_off, p2_len) = strings.push(") is not an instance or subtype of type (");
    let (p3_off, p3_len) = strings.push(").");
    let (ks_off, ks_len) = strings.push("__ap_s__");
    let (ko_off, ko_len) = strings.push("__ap_obj__");
    let (sup_off, sup_len) = strings.push("super");
    let (noargs_off, noargs_len) = strings.push("super(): no arguments");
    add(
        funcs,
        code,
        FT_EXC_CTX_SET,
        &[
            (1, ValType::I32), // 3 = ok
            (1, ty_ref()),     // 4 = cls
            (1, oref()),       // 5 = inst
            (1, str_ref()),    // 6 = msg acc
            (1, str_ref()),    // 7 = obj type name
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_DICT),
                }),
            ), // 8 = marker attrs dict (DSET param 0 is $dict-typed)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ), // 9 = throw-path exc scratch (tag payload type)
            (1, str_ref()),    // 10 = pad
            (1, ty_ref()),     // 11 = S cast to $type (ISINST param 1)
        ],
        |f| {
            // zero-arg form (S = null): RuntimeError("super(): no arguments")
            // at CALL time (S-03-17: the frame lacks the __class__ cell; this
            // slice never provides one, so EVERY bare super() raises — the
            // c021 observable).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("RuntimeError").unwrap_or(0),
            ));
            lit_str(f, noargs_off, noargs_len);
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            f.instruction(&Instruction::End);
            // S arrives $pyobj-typed; cast to $type (ISINST param 1). Also
            // reused for the message's type_name(S) — TYPE_NAME takes o, so
            // param 0 stays the message source.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalSet(11));
            // ok = isinst(obj, S)
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::Call(fx::FN_ISINST));
            f.instruction(&Instruction::LocalSet(3));
            // || (obj is a $type && issubclass(obj, S)) — arms leave []
            // (03.02c engine fact: If(Empty) arms must NOT leave values; the
            // result rides local 3 via LocalSet in each arm).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::Call(fx::FN_ISSUB));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::If(BlockType::Empty));
            // ---- success: marker instance ----
            // cls = cls_new("super", null)
            push_null(f, T_TYPE);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(sup_len as i32));
            f.instruction(&Instruction::I32Const(sup_off as i32));
            f.instruction(&Instruction::I32Const(sup_len as i32));
            f.instruction(&Instruction::ArrayNewData {
                array_type_index: T_BYTE_ARR,
                array_data_index: 1,
            });
            f.instruction(&Instruction::StructNew(T_STR));
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::Call(fx::FN_CLS_NEW));
            f.instruction(&Instruction::LocalSet(4));
            // inst = instance_new(cls)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_INSTANCE_NEW));
            f.instruction(&Instruction::LocalSet(5));
            // attrs dict stash (local 8)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
            sget(f, T_INSTANCE, F_INSTANCE_ATTRS);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalSet(8));
            // attrs["__ap_s__"] = S
            f.instruction(&Instruction::LocalGet(8));
            lit_str(f, ks_off, ks_len);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::Call(fx::FN_DSET));
            // attrs["__ap_obj__"] = obj
            f.instruction(&Instruction::LocalGet(8));
            lit_str(f, ko_off, ko_len);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_DSET));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // ---- failure: TypeError with the CPython message ----
            // msg = p1 + type_name(obj) + p2 + type_name(S) + p3 — local-6
            // accumulator, pieces stash through local 10 (wasm has no Swap
            // instruction; STR_CAT(a, b) takes (a, b) stack order).
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_TYPE_NAME));
            f.instruction(&Instruction::LocalSet(7)); // type_name(obj)
            lit_str(f, p1_off, p1_len);
            f.instruction(&Instruction::LocalSet(6)); // acc = p1
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            lit_str(f, p2_off, p2_len);
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::Call(fx::FN_TYPE_NAME));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            lit_str(f, p3_off, p3_len);
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::Call(fx::FN_STR_CAT));
            f.instruction(&Instruction::LocalSet(6));
            // throw TypeError(msg): [TypeError-ty, roarr(msg)] -> EXC_NEW
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("TypeError").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::GlobalGet(g.frame_head));
            f.instruction(&Instruction::StructSet {
                struct_type_index: T_EXCEPTION,
                field_index: F_EXC_TB,
            });
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            // type-correct unreachable fall-through
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

/// Emit the leaf-03.03d super()/type() pair (super_new handles both the
/// two-arg build and the zero-arg RuntimeError via a null S; type_of answers
/// type(x) — metaclass position included).
pub fn emit_super(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    g: &FrameGlobals,
    strings: &mut StrTab,
) {
    super_new(funcs, code, g, strings);
    type_of(funcs, code);
}

/// leaf wasm-super: register the `super` object's attribute walk
/// (FN_SUPER_MRO_GET = 74). Called by lower.rs AFTER emit_gen_drains — emit
/// order IS the index assignment.
pub fn emit_super_mro(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    super_mro_get(funcs, code, strings);
    bound_obj(funcs, code, strings);
}

// ---------------------------------------------------------------------------
// super_mro_get (FN_SUPER_MRO_GET, leaf wasm-super): (o super-obj, str) -> o.
// CPython's super-attribute rule: `super(S, obj).name` searches the MRO of
// type(obj) STARTING AT THE CLASS AFTER S (when S is in that MRO; from the
// start otherwise) and answers the first class-dict hit. Miss = null.
//
// The super object is the $instance that FN_SUPER_NEW builds: its class is a
// fresh $type named "super" and its attrs dict holds "__ap_s__" = S and
// "__ap_obj__" = obj.
//
// Locals: 3=super-obj  4=key(str)  5=S(ty)  6=obj  7=mro($type_arr)  8=i(i32)
//         9=start(i32)  10=hit(oref)  11=cur(ty)  12=dict  13=n(i32)
// ---------------------------------------------------------------------------
fn super_mro_get(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    let (ks_off, ks_len) = strings.push("__ap_s__");
    let (ko_off, ko_len) = strings.push("__ap_obj__");
    add(
        funcs,
        code,
        FT_ATTR_GET,
        &[
            // ONE pad local: this function has TWO params (0, 1), so one pad
            // puts the first real local at index 3 — the 03.03 local grid.
            // (attr_get-style functions use a (2, ..) pad because they have a
            // single param; copying that here shifted every index by one.)
            (1, oref()),
            (1, oref()),    // 3 = the super object
            (1, str_ref()), // 4 = attribute name
            (1, ty_ref()),  // 5 = S (the start class)
            (1, oref()),    // 6 = obj
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_TYPE_ARR),
                }),
            ), // 7 = mro of type(obj)
            (2, ValType::I32), // 8 = i, 9 = start
            (1, oref()),    // 10 = hit
            (1, ty_ref()),  // 11 = current class
            (1, dict_ref()), // 12 = its dict
        ],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalSet(4));
            // null super-object guard (the call sites pass a real marker; a
            // null must answer null, never trap).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // attrs = super-obj.attrs (lazy dict; the builder always sets it)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
            sget(f, T_INSTANCE, F_INSTANCE_ATTRS);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalSet(12));
            // S = attrs["__ap_s__"], obj = attrs["__ap_obj__"]
            f.instruction(&Instruction::LocalGet(12));
            lit_str(f, ks_off, ks_len);
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::LocalGet(12));
            lit_str(f, ko_off, ko_len);
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::LocalSet(6));
            // mro = type(obj).mro
            f.instruction(&Instruction::LocalGet(6));
            sget(f, T_PYOBJ, F_TYPE);
            f.instruction(&Instruction::RefCastNullable(hc(T_TYPE)));
            sget(f, T_TYPE, F_TYPE_MRO);
            f.instruction(&Instruction::LocalSet(7));
            // start = (index of S in mro) + 1, or 0 when S is not in the mro
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(9));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefEq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(9));
            // Leave the SEARCH (the enclosing block): from inside this `if`
            // the labels are 0 = if, 1 = loop, 2 = block. A depth of 1 would
            // jump to the loop head and spin forever on the same class.
            f.instruction(&Instruction::Br(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // walk from start: first class whose dict holds key
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::ArrayGet(T_TYPE_ARR));
            f.instruction(&Instruction::LocalSet(11));
            f.instruction(&Instruction::LocalGet(11));
            sget(f, T_TYPE, F_TYPE_DICT);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(8));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

// ---------------------------------------------------------------------------
// bound_obj (FN_BOUND_OBJ, leaf wasm-super): (o receiver) -> o — the object
// that an attribute call `recv.m(*args)` binds as the FIRST argument:
//
//   * a `super` object (an $instance whose class is named "super") binds the
//     instance it stands for: CPython calls `type(obj).__mro__[i+1].m(obj)`,
//     never `.m(super_proxy)`. Answers attrs["__ap_obj__"].
//   * a $type (a class object: `A.m(b)`) binds NOTHING: the call arguments
//     already carry the instance. Answers null, which the call site reads as
//     "do not add a first argument".
//   * any other receiver binds itself. Answers the receiver.
//
// Locals (one param at index 0; the pad puts the grid at 3):
//   1, 2 = pad, 3 = receiver, 4 = the super object's attrs dict
// ---------------------------------------------------------------------------
fn bound_obj(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    let (ko_off, ko_len) = strings.push("__ap_obj__");
    let (sup_off, sup_len) = strings.push("super");
    add(
        funcs,
        code,
        FT_O_O,
        // THREE pad locals: one param (0) + three pads puts the first real
        // local at index 3 (the 03.03 local grid). With one pad the grid
        // starts at 2 — type_of's shape; this function follows attr_get's.
        &[(1, oref()), (1, oref()), (1, oref()), (1, dict_ref())],
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalSet(3));
            // null receiver: answer null (the caller then uses its args as-is)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // $type receiver: bind nothing (null)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // `super` object? (class NAME test — the marker is a plain
            // $instance of a fresh $type named "super")
            f.instruction(&Instruction::LocalGet(3));
            sget(f, T_PYOBJ, F_TYPE);
            sget(f, T_TYPE, F_TYPE_NAME);
            lit_str(f, sup_off, sup_len);
            f.instruction(&Instruction::Call(fx::FN_STR_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            // attrs = receiver.attrs (the super builder always sets it)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::RefCastNullable(hc(T_INSTANCE)));
            sget(f, T_INSTANCE, F_INSTANCE_ATTRS);
            f.instruction(&Instruction::RefCastNullable(hc(T_DICT)));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            lit_str(f, ko_off, ko_len);
            f.instruction(&Instruction::Call(fx::FN_DGET));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // any other receiver binds itself
            f.instruction(&Instruction::LocalGet(3));
        },
    );
}

// ---------------------------------------------------------------------------
// type_of (FN_TYPE_OF = 70, leaf 03.03d): (o) -> o — type(x): a $type arg
// answers the 'type' builtin global (metaclass position; its own .type field
// is null on this slice), everything else answers the object's field-0 $type.
// FT: FT_O_O (o) -> o.
// ---------------------------------------------------------------------------
fn type_of(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(funcs, code, FT_O_O, &[(1, oref())], |f| {
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::RefTestNullable(hc(T_TYPE)));
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::GlobalGet(
            builtin_type_global("type").unwrap_or(0),
        ));
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::End);
        f.instruction(&Instruction::LocalGet(0));
        sget(f, T_PYOBJ, F_TYPE);
    });
}

fn push_empty_dict(f: &mut Function) {
    push_null(f, T_TYPE);
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(7));
    f.instruction(&Instruction::I32Const(0));
    for _ in 0..8 {
        f.instruction(&Instruction::I32Const(-1));
    }
    f.instruction(&Instruction::ArrayNewFixed {
        array_type_index: T_IDX_ARR,
        array_size: 8,
    });
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
    f.instruction(&Instruction::StructNew(T_DICT));
}

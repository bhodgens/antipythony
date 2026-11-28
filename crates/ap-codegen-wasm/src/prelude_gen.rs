//! Leaf 03.04 prelude: generators (PEP-342) + the iterator protocol, emitted
//! as in-module wasm functions.
//!
//! Generator runtime model (design §6 + the leaf brief): a generator object
//! is a `$generator` struct `$g = { fn, state, locals[], value, exn }`. The
//! BODY is one wasm function `FT_GEN_STEP (gen, input) -> o` emitted by
//! `lower.rs` (one per generator def). Each `yield` spills live locals into
//! `$g.$locals`, stores the next state, RETURNS the yielded value; the next
//! resume dispatches on `$g.$state`, reloads the spilled locals, and the
//! yield-expression's value is read from `$g.$value` (parked by gen_send).
//!
//! This module emits the DRIVER functions (indices fx::FN_GEN_SEND /
//! FN_ITER_OF / FN_ITER_NEXT / FN_ITER_CONTAINS, appended after FN_TB_RENDER
//! =61 by `lower.rs`'s `emit_gen_prelude` call — dense, never renumbered).
//!
//! Container iterators reuse `$generator`: `$fn` = NULL $function, `locals`
//! = a MUT obj_arr snapshot copy of the container's items, `$value` = an $int
//! carrying the cursor (boxed per next — the field set is fixed by the type
//! section), `state` = the snapshot LENGTH. `gen_new` needs no driver: the
//! lowering builds the $generator struct inline at the call site (params are
//! bound into a locals array exactly like a plain call, then parked).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state as fx;
use crate::prelude::{add, hc, oref, sget, sset, StrTab};
use crate::types::*;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

// ---------------------------------------------------------------------------
// $generator state constants (the encoding every function agrees on)
// ---------------------------------------------------------------------------

/// state = 0: not yet started (body runs from the top).
pub const GS_START: i32 = 0;
/// state = -1: exhausted (a `return v` parks v in `$value` first).
pub const GS_DONE: i32 = -1;
/// state = -2: closed (close() completed, or pre-start throw/close-in).
pub const GS_CLOSED: i32 = -2;

// ---------------------------------------------------------------------------
// gen_send (FN_GEN_SEND / FT_GEN_SEND): (gen o, input o) -> o
//   state >= 0: live generator -> park input in $value, call the step fn
//   (call_indirect through the dispatch table, idx from $fn.$code.func_idx;
//   the step shares table 0 with the user fns). After the step: state < 0
//   means the generator COMPLETED => raise StopIteration($value) (3.14:
//   the return value rides StopIteration). Else return the yielded value.
//   state < 0 (exhausted/closed): raise StopIteration (empty).
// Locals: 0=gen(param) 1=input(param) 2=result(oref) 3=step-idx(i32)
// 4=state-scratch(i32) 5/6 pad(oref)
// ---------------------------------------------------------------------------
pub(crate) fn gen_send(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_GEN_SEND,
        &[(2, oref()), (2, ValType::I32)], // 2,3=oref(result,pad) 4,5=i32(state,step)
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            sget(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::LocalSet(4)); // state scratch
                                                      // step idx lives in local 5 (i32); fix the later LocalSet(3)
                                                      // by using 5 here:
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32GeS);
            f.instruction(&Instruction::If(BlockType::Empty));
            // park input
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::LocalGet(1));
            sset(f, T_GENERATOR, F_GEN_VALUE);
            // step idx = gen.fn.code.func_idx
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_FN,
            });
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_FUNCTION,
                field_index: F_FN_CODE,
            });
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_CODE,
                field_index: F_CODE_FUNC_IDX,
            });
            f.instruction(&Instruction::LocalSet(5)); // step idx (i32)
                                                      // result = call_indirect step(gen, input)
                                                      // NOTE: call_indirect needs the FUNCTION INDEX as an i32 on the
                                                      // stack AFTER the params (stack order: params..., idx). The step
                                                      // fn is reached via the TABLE (position = abs idx - FN_FIRST_USER
                                                      // ... no: the step fn is registered as a PLAIN function, not in
                                                      // the dispatch table — call it DIRECTLY (the idx is static at
                                                      // runtime but the CallIndirect requires a table entry). Simplest
                                                      // correct route on this slice: reserve table position = step idx
                                                      // - FN_FIRST_USER via the element segment (user fns ARE the
                                                      // table), so table_idx = step_abs - FN_FIRST_USER.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(fx::FN_FIRST_USER as i32));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::CallIndirect {
                type_index: FT_GEN_STEP,
                table_index: 0,
            });
            f.instruction(&Instruction::LocalSet(2)); // result
                                                      // completed?
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            sget(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            // raise StopIteration($value) — FN_STOP_ITER throws (never
            // returns at runtime, but the validator still checks the
            // fall-through stack: it returns o => Drop the phantom result
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            sget(f, T_GENERATOR, F_GEN_VALUE);
            f.instruction(&Instruction::I32Const(0)); // has-value flag
            f.instruction(&Instruction::Call(fx::FN_STOP_ITER));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            // exhausted/closed: raise StopIteration (empty)
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::I32Const(1)); // empty flag
            f.instruction(&Instruction::Call(fx::FN_STOP_ITER));
            f.instruction(&Instruction::Drop); // phantom result (never runs)
            f.instruction(&Instruction::End);
            // fn result for the fall-through path (unreachable at runtime:
            // both arms end in Return or the throw — the validator needs it)
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

/// `local.get N; ref.cast (ref null $generator)` — cast-on-use (the ENGINE
/// FACT from 35f2ace0: a refinement routed through a local slot is LOST;
/// struct.get/set need the refined ref STACK-RESIDENT at each use).
fn lget_gen(f: &mut Function, local: u32) {
    f.instruction(&Instruction::LocalGet(local));
    f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
}

// ---------------------------------------------------------------------------
// iter_of (FN_ITER_OF / FT_ITER_OF): (o iterable) -> o
// A $generator returns itself. A $list/$tuple builds a snapshot iterator:
// $generator { fn: null, state: len, locals: [items...], value: $int(0),
// exn: null }. Other shapes: the lowering materializes them to $list first
// (corpus-slice comprehension sources are lists/tuples/str).
// Locals: 0=it 1=i 2=n 3=items-stash 4=sel 5=items-arr 6=cursor
// ---------------------------------------------------------------------------
pub(crate) fn iter_of(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    let (msg_off, msg_len) = strings.push("object is not iterable");
    // Locals (FT has ONE param at 0; lesson 1: param-free slots start at
    // 1): 1,2=oref (value box / str stash) 3=mut $obj_arr snapshot (ARRAY
    // ref, NOT $pyobj: lesson 3) 4=RO items arr 5=$byte_arr stash 6,7,8=i32.
    // (n,i,b).
    add(
        funcs,
        code,
        FT_ITER_OF,
        &[
            (2, oref()),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                }),
            ),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_BYTE_ARR),
                }),
            ),
            (3, ValType::I32),
        ],
        |f| {
            // already an iterator ($generator — user gens AND container
            // iterators): identity (iter(iter(x)) is x).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // $list: snapshot copy into a fresh mut $obj_arr
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_LIST)));
            f.instruction(&Instruction::If(BlockType::Empty));
            {
                emit_items_snapshot(f, SnapshotSrc::List);
                emit_snapshot_iterator(f);
            }
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // $tuple: same snapshot over the tuple items
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            {
                emit_items_snapshot(f, SnapshotSrc::Tuple);
                emit_snapshot_iterator(f);
            }
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // $str: one 1-char $str per BYTE (latin1 corpus slice)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_STR)));
            f.instruction(&Instruction::If(BlockType::Empty));
            {
                emit_str_snapshot(f);
                emit_snapshot_iterator(f);
            }
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // unsupported iterable: TypeError("... object is not iterable")
            f.instruction(&Instruction::GlobalGet(
                exc_type_global("TypeError").unwrap_or(0),
            ));
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("str").unwrap_or(0),
            ));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32Const(msg_len as i32));
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
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

/// Which container field the snapshot reads (cast-on-stack per lesson 4).
#[derive(Clone, Copy)]
enum SnapshotSrc {
    List,
    Tuple,
}

/// $list/$tuple arm body: items array -> local 4 (typed per source), n ->
/// local 7, the mut $obj_arr snapshot -> local 3 (cast-on-stack at every
/// struct.get — lesson 4). $list: the items mut $obj_arr IS the snapshot
/// (a $list owns its fresh array — no copy needed; local 3 = items too).
/// $tuple: the RO items still copy into a fresh mut array (iter_next
/// mutates the cursor... no, the cursor rides $value — but the locals
/// array is shared with user generators, so the RO array still copies).
fn emit_items_snapshot(f: &mut Function, src: SnapshotSrc) {
    // ACTUAL param-free locals (FT_ITER_OF has ONE param, so locals start
    // at 1 — lesson 1): 3=mut $obj_arr snapshot, 4=items arr (RO local:
    // TUPLE path; the LIST path stashes its MUT items via a direct
    // cast-to-mut on the stack — a mut array ref cannot ride the RO local)
    // 6/7=i32 (n, i).
    let (ty, field) = match src {
        SnapshotSrc::List => (T_LIST, F_LIST_ITEMS),
        SnapshotSrc::Tuple => (T_TUPLE, F_TUPLE_ITEMS),
    };
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::RefCastNullable(hc(ty)));
    sget(f, ty, field);
    if matches!(src, SnapshotSrc::List) {
        // $list.items is already a mut $obj_arr ($list owns it — nobody
        // else mutates): use it AS the snapshot, no copy loop. (Local 4 is
        // an RO-typed local in the FT — a mut ref cannot ride it; the list
        // path touches ONLY local 3 = snapshot + local 6 = n.)
        f.instruction(&Instruction::RefCastNullable(hc(T_OBJ_ARR)));
        f.instruction(&Instruction::LocalSet(3)); // snapshot = items
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::ArrayLen);
        f.instruction(&Instruction::LocalSet(6)); // n
        return;
    }
    f.instruction(&Instruction::LocalSet(4)); // items RO (tuple path)
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::ArrayLen);
    f.instruction(&Instruction::LocalSet(6)); // n
    f.instruction(&Instruction::LocalGet(6));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(3)); // snapshot
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(7)); // i
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::LocalGet(6));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::LocalGet(4));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
    f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(7));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End); // loop
    f.instruction(&Instruction::End); // block
}

/// $str arm body: one 1-char $str per byte -> the snapshot in local 4.
fn emit_str_snapshot(f: &mut Function) {
    // locals (param-free from 1): 2=oref str/char stash, 3=mut obj_arr snapshot,
    // 4=RO arr spare, 5=$byte_arr data, 6/7/8=i32 (n, i, b).
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
    f.instruction(&Instruction::LocalSet(2)); // str stash
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
    sget(f, T_STR, F_STR_LEN);
    f.instruction(&Instruction::LocalSet(6)); // n
    f.instruction(&Instruction::LocalGet(6));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(3)); // snapshot
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(7)); // i
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::LocalGet(6));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    // b = str.data[i]
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
    sget(f, T_STR, F_STR_DATA);
    f.instruction(&Instruction::LocalSet(5)); // data byte arr
    f.instruction(&Instruction::LocalGet(5));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(8)); // b
                                              // fresh 1-byte $byte_arr
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::ArrayNewDefault(T_BYTE_ARR));
    f.instruction(&Instruction::LocalSet(5));
    f.instruction(&Instruction::LocalGet(5));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalGet(8));
    f.instruction(&Instruction::ArraySet(T_BYTE_ARR));
    // char = $str{ type=g_str, hash=0, kind=0, len=1, data }
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("str").unwrap_or(0),
    ));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::LocalGet(5));
    f.instruction(&Instruction::StructNew(T_STR));
    f.instruction(&Instruction::LocalSet(2)); // stash the char
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
    f.instruction(&Instruction::LocalGet(7));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(7));
    f.instruction(&Instruction::Br(0));
    f.instruction(&Instruction::End); // loop
    f.instruction(&Instruction::End); // block
}

/// Tail of every iter_of container arm: locals snapshot in local 4, length in
/// local 7 — build the $generator { fn: null, state: len, locals, value:
/// $int(0), exn: null } and return it.
fn emit_snapshot_iterator(f: &mut Function) {
    // locals: 2=oref (value box stash), 3=mut obj_arr snapshot, 6=n.
    f.instruction(&Instruction::GlobalGet(
        builtin_type_global("int").unwrap_or(0),
    ));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
    f.instruction(&Instruction::StructNew(T_INT));
    f.instruction(&Instruction::LocalSet(2));
    // $generator fields: (type, fn, state, locals, value, exn)
    f.instruction(&Instruction::RefNull(hc(T_TYPE))); // $pyobj field 0
    f.instruction(&Instruction::RefNull(hc(T_FUNCTION)));
    f.instruction(&Instruction::LocalGet(6)); // state = snapshot length
    f.instruction(&Instruction::LocalGet(3)); // locals snapshot
    f.instruction(&Instruction::LocalGet(2)); // value cursor box
    f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
    f.instruction(&Instruction::StructNew(T_GENERATOR));
    f.instruction(&Instruction::Return);
}

// ---------------------------------------------------------------------------
// iter_next (FN_ITER_NEXT / FT_ITER_NEXT): (o iterator) -> o
// A $generator: gen_send(it, None). A container iterator (fn == null):
// cursor >= state => StopIteration (empty); else items[cursor], cursor+1.
// Locals: 0=it 1=gen 2=result 3=cursor-i32 4=scratch-i32
// ---------------------------------------------------------------------------
pub(crate) fn iter_next(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_ITER_NEXT,
        &[(2, oref()), (2, ValType::I32)], // 2,3=oref(item,pad) 4,5=i32(cursor,scratch)
        |f| {
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::LocalSet(1)); // gen
                                                      // fn == null? -> container path (cast-on-use per 35f2ace0)
            lget_gen(f, 1);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_FN,
            });
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            // container: cursor = unbox($value.$smi)
            lget_gen(f, 1);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_VALUE,
            });
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_INT,
                field_index: F_INT_SMI,
            });
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::LocalSet(4)); // cursor
            f.instruction(&Instruction::LocalGet(4));
            lget_gen(f, 1);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_STATE,
            });
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(BlockType::Empty));
            // exhausted
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::Call(fx::FN_STOP_ITER));
            f.instruction(&Instruction::Drop); // phantom result (validator)
            f.instruction(&Instruction::End);
            // item = locals[cursor]; cursor++
            lget_gen(f, 1);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_LOCALS,
            });
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
            f.instruction(&Instruction::LocalSet(2)); // item
                                                      // update the cursor box: $value = $int(cursor+1)
            lget_gen(f, 1);
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("int").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
            f.instruction(&Instruction::StructNew(T_INT));
            sset(f, T_GENERATOR, F_GEN_VALUE);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            // generator: gen_send(gen, None)
            lget_gen(f, 1);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::Call(fx::FN_GEN_SEND));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // fn result
        },
    );
}

// ---------------------------------------------------------------------------
// iter_contains (FN_ITER_CONTAINS / FT_ITER_CONTAINS): (o iterable, o item)
// -> i32 — `in` dispatch: a $str/$str pair is the CPython SUBSTRING test
// (leaf wasm-w2w3: the old body RefTest'd $tuple/$list only, so any str
// container RefCast to $tuple TRAPPED — probe pa: 'a' in 'abc' trap at
// fn 65); everything else keeps the linear eq() scan over $list/$tuple
// items. dict/set ride FN_DICT_HAS separately (not routed here).
// Locals: 0=it 1=item 2=i 3=n 4=items(RO arr) 5=cur 6=found
// ---------------------------------------------------------------------------
pub(crate) fn iter_contains(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_ITER_CONTAINS,
        &[
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                }),
            ), // local 2 = items RO-arr (tuples) — arrays are NOT $pyobj
            (1, oref()),       // local 3 = cur
            (1, ValType::I32), // local 4 = n
            (1, ValType::I32), // local 5 = i
            (1, ValType::I32), // local 6 = found
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ), // local 7 = items MUT-arr ($list.items type — lesson 9:
            // a mut array ref cannot ride the RO local)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ), // local 8 = haystack ($str; null = container is not a $str)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_STR),
                }),
            ), // local 9 = needle ($str; null = item is not a $str)
            (1, ValType::I32), // local 10 = window start
            (1, ValType::I32), // local 11 = walk cursor
            (1, ValType::I32), // local 12 = substring verdict (0 miss / 1 hit)
        ],
        |f| {
            // ---- leaf wasm-w2w3: str-in-str substring arm ----
            // CPython: item in str -> substring test. Both operands must be
            // $str (haystack AND needle — `'a' in ['a','b']` is the list
            // eq scan, `'a' in ('a',)` the tuple one; those fall through).
            // kind 0 (latin1 byte) strings only: nothing in the emitter
            // builds $str16/$str32 yet (types.rs declares the subtypes for
            // G-4; no StructNew(T_STR16/T_STR32) emission site exists), so
            // RefCastNullable($str) + the base byte arrays cover the real
            // surface. A multi-byte operand pair would need the data16/
            // data32 arms — recorded, not built.
            //
            // CPython semantics: '' in s -> True; x in '' -> False (only ''
            // matches); the naive walk terminates at n_hay - n_needle.
            // Locals 8/9 are NULLABLE $str stashes (NOT bare casts — a
            // non-str container must reach the list/tuple path unharmed; a
            // cast here would trap on exactly the shapes W2 fixed).
            // ENGINE FACT (learned the hard way, probe pa): Br depth is
            // counted from INSIDE the If — a `Br(2)` inside an If(Empty)
            // targets the enclosing Block at depth 1 only when one Block
            // sits between; the honest multi-level exit is a VERDICT LOCAL
            // (12) + a plain fall-through, no cross-level Br at all.
            // haystack stash — REFTEST-GUARDED: RefCastNullable TRAPS on a
            // non-null wrong type (pin2 probe: the [1,2] list container hit
            // exactly that at the first instruction), so the test arm does
            // the cast and the else arm stashes null.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_STR)));
            f.instruction(&Instruction::If(BlockType::Result(str_ref_val())));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::RefNull(hc(T_STR)));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(8));
            // needle stash (null when the item is not a $str) — same guard
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefTestNullable(hc(T_STR)));
            f.instruction(&Instruction::If(BlockType::Result(str_ref_val())));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::RefNull(hc(T_STR)));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(9));
            // verdict = 1 (substring) only when BOTH stashes are non-null;
            // 0 means "not a str/str pair -> the list/tuple scan below".
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(12));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            // haystack not a $str: verdict stays 0 -> fall to the scan
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            // $str container but non-$str item: CPython scans with __eq__ —
            // the eq() scan below covers it; verdict stays 0.
            f.instruction(&Instruction::Else);
            // ---- real str/str substring test -> verdict 1 on match ----
            // needle longer than haystack -> no match (verdict stays 0;
            // covers 'x' in '')
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Else);
            // needle == "" -> '' in s is True (set the verdict directly;
            // the window bound n_hay - 0 would otherwise run one window
            // too many and the cursor test >= 0 fires inside it anyway —
            // the direct arm is clearer and branch-exact).
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(12));
            f.instruction(&Instruction::Else);
            // windows: start = 0 ..= n_hay - n_needle (inclusive)
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(10)); // start = 0
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            // start > n_hay - n_needle -> every window failed -> done (0)
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::BrIf(1)); // -> outer Block end
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(11)); // cursor = 0
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            // cursor >= needle len -> full match -> verdict 1, break out
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_LEN,
            });
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(12));
            f.instruction(&Instruction::Br(2)); // break: inner Loop+Block
            f.instruction(&Instruction::End);
            // hay[start+cursor] != needle[cursor] -> next window
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_DATA,
            });
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::RefCastNullable(hc(T_STR)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_STR,
                field_index: F_STR_DATA,
            });
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::ArrayGetU(T_BYTE_ARR));
            f.instruction(&Instruction::I32Ne);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Br(2)); // break to the windows Loop
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(11));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End); // inner Loop
            f.instruction(&Instruction::End); // inner Block
            f.instruction(&Instruction::LocalGet(12));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Br(2)); // matched: exit windows Loop+Block
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(10));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End); // windows Loop
            f.instruction(&Instruction::End); // windows Block
            f.instruction(&Instruction::End); // needle-empty If
            f.instruction(&Instruction::End); // needle-longer If
            f.instruction(&Instruction::End); // needle-null If
            f.instruction(&Instruction::End); // haystack-null If
                                              // $str container (haystash non-null)? -> the verdict IS the
                                              // answer either way: 1 = substring hit, 0 = miss (CPython `not
                                              // in` negates at the use site). Falling into the list/tuple
                                              // scan would RefCast the $str to $tuple and TRAP — the exact
                                              // W2 shape (probes pa/pi hit it on the 'z' not in 'abc' link).
                                              // Both arms of this If leave exactly ONE i32 (the fn result;
                                              // 03.02c: If arms must agree with the block result type).
                                              // is a $str container? -> the verdict IS the answer (1 hit /
                                              // 0 miss; `not in` negates at the use site). Then-arm RETURNS;
                                              // fall-through = the $list/$tuple eq() scan (unchanged shape).
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(12));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // $dict container? -> FN_DICT_HAS (str-keyed membership; the
            // CF021 dict row — a dict here used to RefCast to $tuple and
            // trap, pre-existing on main).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_DICT)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_DICT_HAS));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // ---- $list/$tuple scan (not a $str container) ----

            // items + n from $list or $tuple (If/Else probe — the Block+Br
            // shape branches out of the nesting level: wasmtime-47 rejects
            // br-to-fn-label stack shapes; ENGINE FACT 03.02b notes).
            // $list.items (MUT) and $tuple.items (RO) are DIFFERENT array
            // types — they ride separate locals (2=RO, 7=MUT; lesson 9) and
            // the item read branches on the source. n unifies in local 4.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_TUPLE,
                field_index: F_TUPLE_ITEMS,
            });
            f.instruction(&Instruction::LocalSet(2)); // RO items (tuple)
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_LIST,
                field_index: F_LIST_ITEMS,
            });
            f.instruction(&Instruction::LocalSet(7)); // MUT items (list)
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(4)); // n
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5)); // i
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(6)); // found
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // cur = is_list ? items_mut[i] : items_ro[i]
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_LIST)));
            f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(3)); // cur
                                                      // found = eq(cur, item); if found -> break with 1
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_EQ));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Br(2)); // break out of Loop+Block
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End); // Loop
            f.instruction(&Instruction::End); // Block
            f.instruction(&Instruction::LocalGet(6));
        },
    );
}

// ---------------------------------------------------------------------------
// stop_iter (FN_STOP_ITER / FT_STOP_ITER): (value o, empty-flag i32) -> o
// Build + throw StopIteration: empty flag => args = empty tuple; else args
// = (value,). Thrown through $tag_pyexc — never returns (wasm validates the
// trailing stack with a null placeholder the validator proves unreachable
// via the post-throw unreachable... wasm throw IS a stack-polymorphic instr,
// so a RefNull filler after it satisfies the FT and never executes).
// Locals: 0=value 1=empty-flag 2=args-arr
// ---------------------------------------------------------------------------
pub(crate) fn stop_iter(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_STOP_ITER,
        &[
            (1, oref()),
            (1, ValType::I32),
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                }),
            ), // 4 = args stash ($obj_arr_ro is NOT $pyobj — lesson 3)
        ],
        |f| {
            // CPython arg semantics: a VALUED stop => args=(value,); a BARE
            // stop (value == null, the empty-flag input) => args=() — a null
            // in args[0] would trap py_str in str(e) paths (found via the
            // For-exhaustion probe: tb_render -> exc_msg -> py_str(null)).
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Result(ValType::Ref(
                wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                },
            ))));
            // bare: empty args
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 0,
            });
            f.instruction(&Instruction::Else);
            // valued: args = [value]
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(4)); // args
                                                      // exc = exc_new(StopIteration $type, args); raise (never returns;
                                                      // trailing null filler satisfies the FT fall-through).
            f.instruction(&Instruction::GlobalGet(
                exc_type_global("StopIteration").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

#[allow(dead_code)]
pub(crate) fn stop_iter_dead(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_STOP_ITER,
        &[(1, oref()), (1, ValType::I32)],
        |f| {
            // args = always [value] (the empty-flag input is IGNORED on this
            // slice: StopIteration-with-args vs bare differs only in str(e),
            // which no emitted call site on this slice renders; the flag is
            // kept in the FT for the 03.05 traceback surface). The earlier
            // If(Result(RO-arr)) shape failed validation in ways the
            // offset-bisect could not localize within the timebox; this
            // straight-line shape is validator-clean.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 1,
            });
            f.instruction(&Instruction::LocalSet(2)); // args
                                                      // exc = exc_new(StopIteration-type, args); raise via FN_EXC_RAISE.
                                                      // DEBUG BISECT: RefNull instead of GlobalGet (param0 is nullable;
                                                      // if the module now validates, the GlobalGet slot/type is wrong).
            f.instruction(&Instruction::RefNull(hc(T_TYPE)));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop);
            // unreachable filler (throw never returns)
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
}

// ---------------------------------------------------------------------------
// gen_throw (FN_GEN_THROW / FT_GEN_THROW): (gen o, exc o) -> o
// 3.14 semantics: state<0 (exhausted/closed) => the thrown exception itself
// propagates (probe: exhausted throw() raises the thrown exc). Live =>
// stage exc in $exn and call the step: the step's resume path sees $exn
// non-null and throws it at the suspension point.
// Locals: 0=gen 1=exc 2=state-scratch 3=step-idx 4=result
// ---------------------------------------------------------------------------
pub(crate) fn gen_throw(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_GEN_THROW,
        &[(2, oref()), (2, ValType::I32)], // 2=result(oref) 3=pad 4=state 5=idx
        |f| {
            lget_gen(f, 0);
            sget(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::LocalSet(4)); // state
                                                      // state < 0 (dead) OR state == 0 (not started): 3.14 raises the
                                                      // thrown exc AT THE CALL SITE without running any body.
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LeS);
            f.instruction(&Instruction::If(BlockType::Empty));
            // dead/not-started generator: raise the thrown exc directly
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_EXC_RAISE));
            f.instruction(&Instruction::Drop); // phantom result (validator)
            f.instruction(&Instruction::Else);
            // stage + resume: $exn = exc; step sees it at the resume site
            lget_gen(f, 0);
            f.instruction(&Instruction::LocalGet(1));
            // $exn field = (ref null $exception) ≠ param oref: cast on stack
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            sset(f, T_GENERATOR, F_GEN_EXN);
            lget_gen(f, 0);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_FN,
            });
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_FUNCTION,
                field_index: F_FN_CODE,
            });
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_CODE,
                field_index: F_CODE_FUNC_IDX,
            });
            f.instruction(&Instruction::LocalSet(5)); // step idx
            lget_gen(f, 0);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            // table idx = step abs idx - FN_FIRST_USER (the element segment
            // stores absolute fn indices; gen_send does the same subtract).
            f.instruction(&Instruction::LocalGet(5)); // abs idx (i32, on top)
            f.instruction(&Instruction::I32Const(fx::FN_FIRST_USER as i32));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::CallIndirect {
                type_index: FT_GEN_STEP,
                table_index: 0,
            });
            f.instruction(&Instruction::LocalSet(2)); // result
                                                      // completed? => StopIteration($value) (throw-in ran to completion)
            lget_gen(f, 0);
            sget(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            lget_gen(f, 0);
            sget(f, T_GENERATOR, F_GEN_VALUE);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Call(fx::FN_STOP_ITER));
            f.instruction(&Instruction::Drop); // phantom result (validator)
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ))); // fn result
        },
    );
}

// ---------------------------------------------------------------------------
// gen_close (FN_GEN_CLOSE / FT_GEN_CLOSE): (gen o) -> () — 3.14: throw
// GeneratorExit at the point; a GeneratorExit escaping = close OK (silent);
// already-exhausted/closed = no-op; any OTHER exception propagates.
// Locals: 0=gen 1=state-scratch 2=step-idx 3=result 4=exc
// ---------------------------------------------------------------------------
pub(crate) fn gen_close(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_GEN_CLOSE,
        &[(2, oref()), (2, ValType::I32)], // 1,2=oref(exc,pad) 3,4=i32(state,idx)
        |f| {
            lget_gen(f, 0);
            sget(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::LocalSet(3)); // state
                                                      // already dead (exhausted/closed): silent no-op
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // state == 0 (not started): close() completes WITHOUT running the
            // body — just park closed (3.14 semantics).
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(GS_START));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            lget_gen(f, 0);
            f.instruction(&Instruction::I32Const(GS_CLOSED));
            sset(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            // build GeneratorExit() and stage it, then run one step
            f.instruction(&Instruction::GlobalGet(
                exc_type_global("GeneratorExit").unwrap_or(0),
            ));
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_OBJ_ARR_RO,
                array_size: 0,
            });
            f.instruction(&Instruction::Call(fx::FN_EXC_NEW));
            f.instruction(&Instruction::LocalSet(1)); // the exit exc
            lget_gen(f, 0);
            f.instruction(&Instruction::LocalGet(1));
            // $exn field = (ref null $exception): cast on stack (35f2ace0)
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            sset(f, T_GENERATOR, F_GEN_EXN);
            lget_gen(f, 0);
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_GENERATOR,
                field_index: F_GEN_FN,
            });
            f.instruction(&Instruction::RefCastNullable(hc(T_FUNCTION)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_FUNCTION,
                field_index: F_FN_CODE,
            });
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_CODE,
                field_index: F_CODE_FUNC_IDX,
            });
            f.instruction(&Instruction::LocalSet(4)); // step idx
                                                      // 3.14 close(): a GeneratorExit escaping the body = silent OK;
                                                      // any OTHER exception propagates. Catch the resume's throw,
                                                      // match GE, swallow-or-rethrow (03.04f: the re-raised GE flew
                                                      // through this frame to the host before the wrap landed).
                                                      // NOTE: the resume call's operands MUST be pushed INSIDE the
                                                      // result-block — a result-block starts its inner stack EMPTY,
                                                      // and a call_indirect outside would starve (03.04f).
            f.instruction(&Instruction::Block(BlockType::Result(ValType::Ref(
                wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                },
            )))); // $cx
            {
                let catches = [wasm_encoder::Catch::One {
                    tag: TAG_PYEXC,
                    label: 0,
                }];
                f.instruction(&Instruction::TryTable(
                    BlockType::Empty,
                    std::borrow::Cow::Owned(catches.to_vec()),
                ));
            }
            lget_gen(f, 0);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::LocalGet(4)); // abs idx
            f.instruction(&Instruction::I32Const(fx::FN_FIRST_USER as i32));
            f.instruction(&Instruction::I32Sub);
            f.instruction(&Instruction::CallIndirect {
                type_index: FT_GEN_STEP,
                table_index: 0,
            });
            f.instruction(&Instruction::Drop); // yielded-if-any (discard)
                                               // body swallowed GeneratorExit and suspended (state >= 0) —
                                               // corpus never swallows: park exhausted so later resumes raise
                                               // StopIteration (the old unconditional close).
            lget_gen(f, 0);
            f.instruction(&Instruction::I32Const(GS_CLOSED));
            sset(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::End); // try_table End
            f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            f.instruction(&Instruction::End); // $cx: no-throw path null
                                              // thrown-through: GE escape = silent close OK (park closed);
                                              // any other exception rethrown to the caller. NULL payload =
                                              // the step completed normally (handler swallowed the GE) —
                                              // same silent close (03.04f: matching a null payload trapped
                                              // inside the matcher).
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            // null payload: the step completed (handler swallowed the GE)
            lget_gen(f, 0);
            f.instruction(&Instruction::I32Const(GS_CLOSED));
            sset(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::GlobalGet(
                exc_type_global("GeneratorExit").unwrap_or(0),
            ));
            f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
            f.instruction(&Instruction::If(BlockType::Empty));
            // GeneratorExit escaping = silent close OK
            lget_gen(f, 0);
            f.instruction(&Instruction::I32Const(GS_CLOSED));
            sset(f, T_GENERATOR, F_GEN_STATE);
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
        },
    );
}

/// Emit every 03.04 driver, in `fx::FN_*` index order (dense append after
/// FN_TB_RENDER; user fns still start at FN_FIRST_USER — bump that const
/// once these land).
pub fn emit_gen_prelude(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    strings: &mut StrTab,
) {
    gen_send(funcs, code);
    iter_of(funcs, code, strings);
    iter_next(funcs, code);
    iter_contains(funcs, code);
    stop_iter(funcs, code);
    gen_throw(funcs, code);
    gen_close(funcs, code);
}

/// leaf 03.04c: the drain drivers (FN_LIST_DRAIN=71 / FN_SUM_DRAIN=72).
/// SEPARATE from emit_gen_prelude: emit_super lands at 69/70 BETWEEN the
/// two blocks (lower.rs calls emit_gen_prelude, then emit_super, then this) —
/// the fn-index constants in emitter_state MUST match the emit order.
pub fn emit_gen_drains(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    crate::prelude_gen_drain::list_drain(funcs, code);
    crate::prelude_gen_drain::sum_drain(funcs, code);
    // 03.04g: dict-from-pairs (FN_DICT_DRAIN=73) — FN_FIRST_USER moves 73->74.
    crate::prelude_gen_drain::dict_drain(funcs, code);
}

/// nullable `(ref null $str)` ValType (iter_contains substring stash arms).
fn str_ref_val() -> ValType {
    ValType::Ref(wasm_encoder::RefType {
        nullable: true,
        heap_type: hc(T_STR),
    })
}

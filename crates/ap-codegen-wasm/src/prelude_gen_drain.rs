//! Leaf 03.04c append: the DRAIN drivers (iter -> list / iter -> sum).
//! Dense append after FN_TYPE_OF=70; FN_FIRST_USER moves 71 -> 73 (emitter_state).
//!
//! list_drain (FN_LIST_DRAIN / FT_O_O): (iterator) -> $list — next() until
//! StopIteration, accumulating into a MUT $obj_arr, then T_LIST{len, cap,
//! items-RO}. The items field is a MUT $obj_arr REUSED as-is (the $list owns
//! it; no aliasing hazard exists on this slice because the $generator's
//! locals array is never re-issued).
//!
//! sum_drain (FN_SUM_DRAIN / FT_O_O): (iterator) -> $int — acc = $int(0),
//! next() until StopIteration, acc = BINOP(ADD, acc, item).
//!
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use crate::emitter_state as fx;
use crate::prelude::{add, hc, oref, sget};
use crate::types::*;
use wasm_encoder::{BlockType, Function, Instruction, ValType};

/// Local-slot contract for both drains (FT has ONE param at 0; lesson 1:
/// param-free locals start at 1):
/// 1 = mut $obj_arr acc (ARRAY ref — NOT $pyobj: dedicated slot, lesson 3)
/// 2 = oref (item / value box)
/// 3 = i32 (n)
/// 4 = i32 (i)
/// 5 = i64 (sum acc for sum_drain)
fn push_drain_common(f: &mut Function) {
    // acc = mut $obj_arr, len 0
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
    f.instruction(&Instruction::LocalSet(1));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::LocalSet(3)); // n
}

/// drain loop: while next(it) succeeds: item -> acc[n]; n++. Raises TAG out
/// on real exceptions (exhaustion caught here via try_table -> payload null
/// vs real; rethrow non-null).
fn emit_drain_loop(f: &mut Function, accumulate: Accumulate) {
    // Block $out
    //   Block $catch (result exc)
    //     try_table [catch -> $catch]
    //       Loop { item = iter_next(0); acc append; acc-sum; br Loop }
    //     RefNull exc
    //   end
    //   null? -> br past rethrow ; else rethrow
    // end
    f.instruction(&Instruction::Block(BlockType::Empty)); // $out
    f.instruction(&Instruction::Block(BlockType::Result(
        ValType::Ref(wasm_encoder::RefType {
            nullable: true,
            heap_type: hc(T_EXCEPTION),
        }),
    ))); // $catch
    {
        let catches = [wasm_encoder::Catch::One {
            tag: TAG_PYEXC,
            label: 0,
        }];
        f.instruction(&Instruction::TryTable(
            BlockType::Empty,
            std::borrow::Cow::Owned(catches.to_vec()),
        ));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        // item = iter_next(it)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
        f.instruction(&Instruction::LocalSet(2)); // item
        // acc = rebuild-append(acc, item)  (growable-list pattern)
        // n+1 new array, copy 0..n, [n] = item
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
        f.instruction(&Instruction::LocalSet(6)); // next buffer (local 6)
        // copy 0..n
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::LocalSet(4)); // i
        f.instruction(&Instruction::Block(BlockType::Empty));
        f.instruction(&Instruction::Loop(BlockType::Empty));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32GeU);
        f.instruction(&Instruction::BrIf(1));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
        f.instruction(&Instruction::LocalGet(4));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // loop
        f.instruction(&Instruction::End); // block
        // next[n] = item; acc = next; n += 1
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::LocalSet(1));
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::LocalSet(3));
        // sum acc: local5 = local5 + item.smi (int-only corpus slice)
        if matches!(accumulate, Accumulate::Sum) {
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_INT)));
            sget(f, T_INT, F_INT_SMI);
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(5));
        }
        f.instruction(&Instruction::Br(0));
        f.instruction(&Instruction::End); // Loop
        f.instruction(&Instruction::End); // try_table
        f.instruction(&Instruction::RefNull(hc(T_EXCEPTION))); // success payload
    }
    f.instruction(&Instruction::End); // $catch: exc | null
    f.instruction(&Instruction::LocalSet(2)); // reuse oref slot for payload
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::RefIsNull);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::Br(1)); // null = success -> $out end
    f.instruction(&Instruction::End);
    // payload non-null: StopIteration = NORMAL exhaustion (exit $out with the
    // accumulator as-is); anything else = a real error -> rethrow. (The
    // first draft rethrew EVERY non-null payload — list()/sum() always died
    // with an uncaught StopIteration; found by the ge_run_probe drain cases.)
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::GlobalGet(
        crate::types::exc_type_global("StopIteration").unwrap_or(0),
    ));
    f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::Br(1)); // exhaustion -> $out end (If=@2, $out=@1)
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
    f.instruction(&Instruction::Throw(TAG_PYEXC));
    f.instruction(&Instruction::End); // $out
}

#[derive(Clone, Copy)]
enum Accumulate {
    Items,
    Sum,
}

pub(crate) fn list_drain(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_O_O,
        &[
            // body locals (FT has ONE param at 0 — lesson 1: start at 1):
            // 1 = mut $obj_arr acc (lesson 3: NOT $pyobj — dedicated slot)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ),
            (1, oref()), // 2 = item/payload
            (2, ValType::I32), // 3,4 = n, i
            (1, ValType::I64), // 5 = sum acc
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ), // 6 = next buffer
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR_RO),
                }),
            ), // 7 = RO items copy (the $list items field)
        ],
        |f| {
            push_drain_common(f);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(5));
            emit_drain_loop(f, Accumulate::Items);
            // $list { type(0), len(1), cap(2), items(3) } — StructNew pops in
            // REVERSE field order: push type first, then len, then cap, then
            // items LAST. The accumulator mut $obj_arr IS the $list.items
            // type now (types.rs: items flipped to T_OBJ_ARR when the drain
            // probes exposed ref.cast(mut->RO) as a runtime trap).
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("list").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(1)); // len (py_str/eq read cap)
            f.instruction(&Instruction::LocalGet(3)); // cap = n
            f.instruction(&Instruction::LocalGet(1)); // items
            f.instruction(&Instruction::StructNew(T_LIST));
        },
    );
}

pub(crate) fn sum_drain(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_O_O,
        &[
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ), // 1 = acc
            (1, oref()),       // 2
            (2, ValType::I32), // 3,4
            (1, ValType::I64), // 5
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_OBJ_ARR),
                }),
            ), // 6
        ],
        |f| {
            push_drain_common(f);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(5));
            emit_drain_loop(f, Accumulate::Sum);
            // $int{ type: int-g, kind: 0, smi: acc, big: null }
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("int").unwrap_or(0),
            ));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::RefNull(hc(T_BIGINT)));
            f.instruction(&Instruction::StructNew(T_INT));
        },
    );
}

/// 03.04g dict_drain (FN_DICT_DRAIN / FT_O_O): (iterator of [k, v] pairs)
/// -> $dict — pairs-gen output drained into insertion-ordered entries.
/// Insertion = rebuild the entries array (immutable) + fill/used bump; the
/// pair items come from the pairs-gen's 2-element $list (the dummy-Tuple
/// collect yields a Tuple; the machine parks its RO items — normalize per
/// iteration by reading items from either $list or $tuple shape via the
/// RefTest branch).
pub(crate) fn dict_drain(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    add(
        funcs,
        code,
        FT_O_O,
        &[
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_ENTRY_ARR),
                }),
            ), // 1 = entries acc (DEDICATED type — lesson 3)
            (1, oref()), // 2 = item / payload / cur entry
            (2, ValType::I32), // 3,4 = n, i
            (1, ValType::I64), // 5 (unused, layout parity)
            (
                1,
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_ENTRY_ARR),
                }),
            ), // 6 = next buffer
            (1, oref()), // 7 = k
            (1, oref()), // 8 = v
            (1, oref()), // 9 = items (mut or RO array, cast at use)
        ],
        |f| {
            // dict skeleton fields are built by the CALLER (push_empty_dict
            // shape); here: acc = empty entries, n = 0
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
            f.instruction(&Instruction::LocalSet(1));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(3)); // n
            // drain loop (same control shape as emit_drain_loop, entries-typed)
            f.instruction(&Instruction::Block(BlockType::Empty)); // $out
            f.instruction(&Instruction::Block(BlockType::Result(
                ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: hc(T_EXCEPTION),
                }),
            ))); // $catch
            {
                let catches = [wasm_encoder::Catch::One {
                    tag: TAG_PYEXC,
                    label: 0,
                }];
                f.instruction(&Instruction::TryTable(
                    BlockType::Empty,
                    std::borrow::Cow::Owned(catches.to_vec()),
                ));
                f.instruction(&Instruction::Loop(BlockType::Empty));
                // item = iter_next(0)
                f.instruction(&Instruction::LocalGet(0));
                f.instruction(&Instruction::Call(fx::FN_ITER_NEXT));
                f.instruction(&Instruction::LocalSet(2)); // pair
                // items = pair.items — $list.items (mut $obj_arr) or
                // $tuple.items (RO): branch by shape (arrays are NOT
                // $pyobj; no oref local, read direct off the cast)
                // items -> local 9 as MUT (list) or... two locals would
                // double the shape; simplest: read k/v TWICE like the dict
                // repr does (fetch direct, no stash).
                // k = items[0]
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefTestNullable(hc(T_LIST)));
                f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
                sget(f, T_LIST, F_LIST_ITEMS);
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
                sget(f, T_TUPLE, F_TUPLE_ITEMS);
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalSet(7)); // k
                // v = items[1]
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefTestNullable(hc(T_LIST)));
                f.instruction(&Instruction::If(BlockType::Result(pyobj_ref())));
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
                sget(f, T_LIST, F_LIST_ITEMS);
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
                f.instruction(&Instruction::Else);
                f.instruction(&Instruction::LocalGet(2));
                f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
                sget(f, T_TUPLE, F_TUPLE_ITEMS);
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
                f.instruction(&Instruction::End);
                f.instruction(&Instruction::LocalSet(8)); // v
                // acc rebuild-append {k, v} (entry_arr-typed)
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::ArrayNewDefault(T_ENTRY_ARR));
                f.instruction(&Instruction::LocalSet(6)); // next buffer
                f.instruction(&Instruction::I32Const(0));
                f.instruction(&Instruction::LocalSet(4)); // i
                f.instruction(&Instruction::Block(BlockType::Empty));
                f.instruction(&Instruction::Loop(BlockType::Empty));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32GeU);
                f.instruction(&Instruction::BrIf(1));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::LocalGet(1));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::ArrayGet(T_ENTRY_ARR));
                f.instruction(&Instruction::ArraySet(T_ENTRY_ARR));
                f.instruction(&Instruction::LocalGet(4));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(4));
                f.instruction(&Instruction::Br(0));
                f.instruction(&Instruction::End); // loop
                f.instruction(&Instruction::End); // block
                // next[n] = {hash:0, k, v}; acc = next; n += 1
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I64Const(0)); // hash (v1)
                f.instruction(&Instruction::LocalGet(7));
                f.instruction(&Instruction::LocalGet(8));
                f.instruction(&Instruction::StructNew(T_DICT_ENTRY));
                f.instruction(&Instruction::ArraySet(T_ENTRY_ARR));
                f.instruction(&Instruction::LocalGet(6));
                f.instruction(&Instruction::LocalSet(1));
                f.instruction(&Instruction::LocalGet(3));
                f.instruction(&Instruction::I32Const(1));
                f.instruction(&Instruction::I32Add);
                f.instruction(&Instruction::LocalSet(3));
                f.instruction(&Instruction::Br(0));
                f.instruction(&Instruction::End); // Loop
                f.instruction(&Instruction::End); // try_table
                f.instruction(&Instruction::RefNull(hc(T_EXCEPTION)));
            }
            f.instruction(&Instruction::End); // $catch: exc | null
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefIsNull);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Br(1)); // null = success
            f.instruction(&Instruction::End);
            // SI = exhaustion; anything else rethrow
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::GlobalGet(
                crate::types::exc_type_global("StopIteration").unwrap_or(0),
            ));
            f.instruction(&Instruction::Call(fx::FN_EXC_MATCH));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::Br(1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::RefCastNullable(hc(T_EXCEPTION)));
            f.instruction(&Instruction::Throw(TAG_PYEXC));
            f.instruction(&Instruction::End); // $out
            // $dict { type, used=n, fill=n, mask=7, perturb=0, idx=[-1;8],
            //         entries=acc } — StructNew pops in REVERSE field order.
            f.instruction(&Instruction::GlobalGet(
                builtin_type_global("dict").unwrap_or(0),
            ));
            f.instruction(&Instruction::LocalGet(3)); // used
            f.instruction(&Instruction::LocalGet(3)); // fill
            f.instruction(&Instruction::I32Const(7)); // mask
            f.instruction(&Instruction::I32Const(0)); // perturb
            for _ in 0..8 {
                f.instruction(&Instruction::I32Const(-1));
            }
            f.instruction(&Instruction::ArrayNewFixed {
                array_type_index: T_IDX_ARR,
                array_size: 8,
            });
            f.instruction(&Instruction::LocalGet(1)); // entries
            f.instruction(&Instruction::StructNew(T_DICT));
        },
    );
}

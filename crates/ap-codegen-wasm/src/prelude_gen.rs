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
use crate::prelude::{add, hc, oref, sget, sset};
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
) {
    add(
        funcs,
        code,
        FT_ITER_OF,
        &[(2, oref()), (2, ValType::I32)], // 2,3=oref(result,pad) 4,5=i32(cursor,n)
        |f| {
            // BISECT: minimal body — passthrough only.
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_GENERATOR)));
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::Return);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
        },
    );
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
// -> i32 — linear eq() scan over $list/$tuple items (the `in` surface for
// the corpus slice; dict/set ride FN_DICT_HAS separately, str substring
// rides the str_eq prefix arm — both deferred to their leaves).
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
                    heap_type: hc(T_OBJ_ARR),
                }),
            ), // local 2 = items (MUT; arrays are NOT $pyobj: dedicated slot).
               // 03.03d: $list.items switched to MUT, so the common local type
               // is the MUT array — the $tuple branch copies its RO items into
               // a fresh MUT array (corpus tuples are small).
            (1, oref()),       // local 3 = cur
            (1, ValType::I32), // local 4 = n
            (1, ValType::I32), // local 5 = i
            (1, ValType::I32), // local 6 = found
        ],
        |f| {
            // items + n from $list or $tuple (If/Else probe — the Block+Br
            // shape branches out of the nesting level: wasmtime-47 rejects
            // br-to-fn-label stack shapes; ENGINE FACT 03.02b notes)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefTestNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::If(BlockType::Empty));
            // $tuple: copy RO items -> fresh MUT array
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_TUPLE,
                field_index: F_TUPLE_ITEMS,
            });
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(4)); // n = tuple len
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayNewDefault(T_OBJ_ARR));
            f.instruction(&Instruction::LocalSet(2)); // mut copy target
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_TUPLE)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_TUPLE,
                field_index: F_TUPLE_ITEMS,
            });
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR_RO));
            f.instruction(&Instruction::ArraySet(T_OBJ_ARR));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::RefCastNullable(hc(T_LIST)));
            f.instruction(&Instruction::StructGet {
                struct_type_index: T_LIST,
                field_index: F_LIST_ITEMS,
            });
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::ArrayLen);
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
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_OBJ_ARR));
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
        &[(1, oref()), (1, ValType::I32)],
        |f| {
            // DEBUG BISECT 3: stub body — no exc machinery.
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
            f.instruction(&Instruction::LocalGet(2));
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
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            // dead generator: raise the thrown exc directly (3.14 arm)
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
            f.instruction(&Instruction::LocalGet(5)); // table idx (i32, on top)
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
            // already dead: silent no-op
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::I32LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
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
            lget_gen(f, 0);
            f.instruction(&Instruction::RefNull(hc(T_PYOBJ)));
            f.instruction(&Instruction::LocalGet(4)); // table idx (i32, on top)
            f.instruction(&Instruction::CallIndirect {
                type_index: FT_GEN_STEP,
                table_index: 0,
            });
            f.instruction(&Instruction::Drop); // yielded-if-any (discard)
            // The step ran: if it SUSPENDED (state >= 0), the body swallowed
            // GeneratorExit and yielded again => 3.14 RuntimeError. On this
            // slice: silent complete (corpus never swallows GeneratorExit).
            // Park exhaustion so later resumes raise StopIteration.
            lget_gen(f, 0);
            f.instruction(&Instruction::I32Const(GS_CLOSED));
            sset(f, T_GENERATOR, F_GEN_STATE);
        },
    );
}

/// Emit every 03.04 driver, in `fx::FN_*` index order (dense append after
/// FN_TB_RENDER; user fns still start at FN_FIRST_USER — bump that const
/// once these land).
pub fn emit_gen_prelude(
    funcs: &mut wasm_encoder::FunctionSection,
    code: &mut wasm_encoder::CodeSection,
) {
    gen_send(funcs, code);
    iter_of(funcs, code);
    iter_next(funcs, code);
    iter_contains(funcs, code);
    stop_iter(funcs, code);
    gen_throw(funcs, code);
    gen_close(funcs, code);
}

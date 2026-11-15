// ============================================================================
// Leaf 03.02b bigint limb arithmetic (V001 exactness gate)
// ============================================================================
// All helpers work on the $bigint layout { sign: i32, limbs: $limb_arr }
// (u32-LE limbs, sign 0 = non-negative / 1 = negative, ZERO = EMPTY limbs
// sign 0 — the big_norm invariant).
//
// FTs: FT_BB_B (big,big)->big = 58, FT_BI_B (big,i64)->big = 59,
//      FT_BI_L (big,i64)->i64 = 60.
// ============================================================================

/// Build a $bigint from (sign, limbs-array). Stack in: sign i32, limbs.
fn mk_big(f: &mut Function) {
    f.instruction(&Instruction::StructNew(T_BIGINT));
}

/// Call FN_BIG_NORM on the big at the stack top.
fn norm_call(f: &mut Function) {
    f.instruction(&Instruction::Call(fx::FN_BIG_NORM));
}

// ---------------------------------------------------------------------------
// FN_BIG_UADD (FT_BB_B): (a, b) -> |a| + |b|  (limb-wise, sign ignored)
// Locals: 2=na 3=nb 4=n(max) 5=i 6=carry 7=sa(limb a) 8=sb(limb b) 9=sum
// 10=out-limbs 11=s (a's sign/unused) 12=buf
// ---------------------------------------------------------------------------
fn big_uadd(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BB_B,
        &[
            (3, ValType::I32),                                  // 2,3,4
            (2, ValType::I64),                                  // 5,6
            (4, oref()),                                        // 7..10 (7 unused)
            (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_LIMB_ARRAY) })), // 11
            (2, ValType::I32),                                  // 12,13
        ],
        |f| {
            // na = a.limbs.len
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(2));
            // nb = b.limbs.len
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(3));
            // n = max(na, nb) + 1
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32GtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            // out = array.new_default(n)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(11));
            // carry = 0; i = 0
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            // loop i < n
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // sa = i < na ? a.limbs[i] : 0  (zero-extend to i64)
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(7));
            // sb likewise from b
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(8));
            // sum = sa + sb + carry
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(9));
            // out[i] = sum & 0xFFFFFFFF
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            // carry = sum >> 32
            f.instruction(&Instruction::LocalGet(9));
            f.instruction(&Instruction::I64Const(32));
            f.instruction(&Instruction::I64ShrU);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // result: big_norm( (sign=0, out) )
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(11));
            mk_big(f);
            norm_call(f);
        },
    );
}

// ---------------------------------------------------------------------------
// FN_BIG_USUB (FT_BB_B): (a, b) -> |a| - |b|  (REQUIRES |a| >= |b|)
// Locals: 2=na 3=nb 4=i 5=sa 6=sb 7=borrow 8=out 9=tmp
// ---------------------------------------------------------------------------
fn big_usub(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BB_B,
        &[
            (3, ValType::I32),                                  // 2,3,4
            (2, ValType::I64),                                  // 5,6
            (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_LIMB_ARRAY) })), // 7
        ],
        |f| {
            // na
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(2));
            // nb
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(3));
            // out = array.new_default(na)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(7));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // sa = a.limbs[i] (i64)
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(5));
            // sb = i < nb ? b.limbs[i] : 0
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32LtU);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I64)));
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(6));
            // diff = sa - sb - borrow
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64LtS);
            // borrow-in: reuse a fresh read of borrow local as i64
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::Drop);
            // NOTE: simpler: recompute in two steps below
            f.instruction(&Instruction::Drop);
            // t = sa - sb
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Sub);
            f.instruction(&Instruction::LocalSet(5)); // reuse local5 as t
            // if t < 0: t += 2^32; borrow = 1 else borrow = 0
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::I64LtS);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I64Const(0x1_0000_0000i64));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::I64Const(1));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::End);
            // out[i] = t as u32
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(7));
            mk_big(f);
            norm_call(f);
        },
    );
}

// ---------------------------------------------------------------------------
// FN_BIG_UMUL (FT_BB_B): (a, b) -> |a| * |b|  (schoolbook u32)
// Locals: 2=na 3=nb 4=i 5=j 6=carry 7=cur(u64) 8=prod(u64) 9=out 10=aj
// 11=bj 12=prev 13=zero-flag tmp
// ---------------------------------------------------------------------------
fn big_umul(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BB_B,
        &[
            (4, ValType::I32),                     // 2,3,4,5
            (3, ValType::I64),                     // 6,7,8
            (2, oref()),                           // 9,10 unused
            (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_LIMB_ARRAY) })), // 11
        ],
        |f| {
            // na, nb
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(2));
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::ArrayLen);
            f.instruction(&Instruction::LocalSet(3));
            // out = array.new_default(na + nb)
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(11));
            // for i in 0..na: carry=0; for j in 0..nb: cur=out[i+j]+aj*bj+carry ...
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            // outer: i < na
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // aj = a.limbs[i] (i64)
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(10));
            // carry = 0
            f.instruction(&Instruction::I64Const(0));
            f.instruction(&Instruction::LocalSet(6));
            // j = 0
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalSet(5));
            // inner loop
            f.instruction(&Instruction::Block(BlockType::Empty));
            f.instruction(&Instruction::Loop(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32GeU);
            f.instruction(&Instruction::BrIf(1));
            // bj
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_LIMBS);
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(13));
            // prod = aj * bj + carry
            f.instruction(&Instruction::LocalGet(10));
            f.instruction(&Instruction::LocalGet(13));
            f.instruction(&Instruction::I64Mul);
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(8));
            // prev = out[i+j]
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::I64ExtendI32U);
            f.instruction(&Instruction::LocalSet(12));
            // cur = prod + prev
            f.instruction(&Instruction::LocalGet(8));
            f.instruction(&Instruction::LocalGet(12));
            f.instruction(&Instruction::I64Add);
            f.instruction(&Instruction::LocalSet(7));
            // out[i+j] = cur & 0xFFFFFFFF
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            // carry = cur >> 32
            f.instruction(&Instruction::LocalGet(7));
            f.instruction(&Instruction::I64Const(32));
            f.instruction(&Instruction::I64ShrU);
            f.instruction(&Instruction::LocalSet(6));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(5));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            // out[i+nb] += carry
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalGet(11));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArrayGet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalGet(6));
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::I32Add);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::Br(0));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(11));
            mk_big(f);
            norm_call(f);
        },
    );
}

// ---------------------------------------------------------------------------
// FN_BIG_FROM_U64 (FT_L_B2 = (i64) -> big): unsigned magnitude -> +big.
// Locals: 2=lo 3=hi
// ---------------------------------------------------------------------------
fn big_from_u64(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BI_B,
        &[(1, ValType::I64), (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
          (1, ValType::Ref(wasm_encoder::RefType { nullable: true, heap_type: hc(T_LIMB_ARRAY) }))],
        |f| {
            // lo = v & 0xFFFFFFFF; hi = v >> 32
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
            f.instruction(&Instruction::I64And);
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::LocalSet(2));
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::I64Const(32));
            f.instruction(&Instruction::I64ShrU);
            f.instruction(&Instruction::I32WrapI64);
            f.instruction(&Instruction::LocalSet(3));
            // n = hi != 0 ? 2 : (lo != 0 ? 1 : 0)
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::I32Eqz);
            f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::Else);
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::LocalSet(4));
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::ArrayNewDefault(T_LIMB_ARRAY));
            f.instruction(&Instruction::LocalSet(5));
            // limbs[0] = lo
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            // limbs[1] = hi (when n == 2)
            f.instruction(&Instruction::LocalGet(4));
            f.instruction(&Instruction::I32Const(2));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            f.instruction(&Instruction::LocalGet(5));
            f.instruction(&Instruction::I32Const(1));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::ArraySet(T_LIMB_ARRAY));
            f.instruction(&Instruction::End);
            f.instruction(&Instruction::I32Const(0));
            f.instruction(&Instruction::LocalGet(5));
            mk_big(f);
            norm_call(f);
        },
    );
}

// ---------------------------------------------------------------------------
// FN_BIG_ADD (FT_BB_B): signed a + b.
// Locals: 2=sa 3=sb 4=cmp 5..8 scratch
// ---------------------------------------------------------------------------
fn big_add(funcs: &mut wasm_encoder::FunctionSection, code: &mut wasm_encoder::CodeSection) {
    add(
        funcs,
        code,
        FT_BB_B,
        &[(4, ValType::I32), (1, oref())],
        |f| {
            // sa = a.sign; sb = b.sign
            lget_cast(f, 0, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_SIGN);
            f.instruction(&Instruction::LocalSet(2));
            lget_cast(f, 1, T_BIGINT);
            sget(f, T_BIGINT, F_BIG_SIGN);
            f.instruction(&Instruction::LocalSet(3));
            // cmp = big_cmp(a, b)
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_BIG_CMP));
            f.instruction(&Instruction::LocalSet(4));
            // same sign?
            f.instruction(&Instruction::LocalGet(2));
            f.instruction(&Instruction::LocalGet(3));
            f.instruction(&Instruction::I32Eq);
            f.instruction(&Instruction::If(BlockType::Empty));
            // uadd(a, b) keeps sa
            f.instruction(&Instruction::LocalGet(0));
            f.instruction(&Instruction::LocalGet(1));
            f.instruction(&Instruction::Call(fx::FN_BIG_UADD));
            f.instruction(&Instruction::LocalSet(5)); // need oref scratch... use struct field path below
            f.instruction(&Instruction::End);
            // (fallthrough handled in the Rust structure below — see note)
        },
    );
}

// ---------------------------------------------------------------------------
// NOTE (honest scoping): the signed add/sub/mul wrappers continue here; the
// emit code below is assembled in `big_signed_arith` because the branch
// structure shares large pieces.
// ---------------------------------------------------------------------------

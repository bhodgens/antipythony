# Leaf 03.02b resume — WasmGC values/arithmetic/containers (2026-11-15 closeout)

## DONE (byte-exact vs expected/, verified via diag_run + ap-host)
- V051-eq-vs-identity, V052-cross-type-numeric-eq, V055-hash-eq-invariant:
  BYTE-EXACT. Commits 019020ff, b6cdceb0, f1859d4f on tier1-0302b-resume.
- Root causes fixed en route (see git log for details):
  (1) eq() was a bisect stub -> wired to eq_full; eq_full's local map was
      WRONG (assumed 1=a 2=b 3=sel-a 4=sel-b; FT_OO_I params land 0=a 1=b,
      locals now 2=a 3=b 4=sel-a 5=sel-b, second type_sel scratch=6 so
      sel-a is NOT clobbered — the clobber made None==False dispatch into
      int_eq -> ref.cast trap). Outer Block(I32) needs a fallthrough const.
  (2) compare() rewritten: EQ/NE via FN_EQ; ordering via f64 promotion
      (bool normalize, I32LeU sel guards, LT/LE/GT/GE If-chain with
      Unreachable fillers); locals (2,oref),(2,I32),(2,F64)=7,8.
  (3) float_py_eq: old body cast BOTH sides to $float unconditionally
      (null-cast trap); rewrote with type_sel promotion: float direct,
      int/bool via smi->f64, non-numeric -> 0/0 NaN (compares false).
      Locals (2,oref),(2,F64),(1,I32)=6=sel.
  (4) int_to_str digit DIRECTION: append_i64 copy loop was fine — the
      real symptom was V001 lines 2-4 (bigints), NOT the digit order;
      line 1 multi-digit prints byte-exact (1000000, 57005, 3735928559).
  (5) dict_eq was a stub -> real linear-scan implementation (entry local
      typed (ref null $dict_entry) — $dict_entry is a BARE struct, NOT a
      $pyobj, so oref locals cannot hold it; matched flag = local 13).
  (6) hash(): bool (sel 2) now rides the $int payload path (sel 0||2).
  (7) unary(): float arm must use RefTestNullable (ref.cast traps on
      non-float operands); big NEG/INVERT removed (see below).
  (8) lower.rs: list/tuple literals must build items via ArrayNewFixed
      over T_OBJ_ARR_RO (WasmGC: array.new_fixed requires immutable
      array type); $list.items field = T_OBJ_ARR_RO; stash in local 48
      (limb/array refs are not $pyobj subtypes). The compare bool-build
      spills the folded i32 into local 47 (TEMP_LOCAL_CMP) before
      pushing bool-type/kind — the b13ed8f3 shape left the folded i32
      buried under type/kind (the V051/V052/V055 validate errors).

## V001 status (NOT byte-exact, fail-closed acceptable per notes scope)
- Line 1 (smi path: 1000000, bin/oct/hex, 0xDEAD/0Xdead_beef) byte-exact.
- Lines 2-4 need MULTI-LIMB BIGINT arithmetic (2**100, n*n, x<<200>>200,
  ~x, 123456789012345678901234567890 literal, 2**200*3**150 % m).
- Attempted this session: full limb core (uadd/usub/umul/add/sub/mul/
  shl/shr/and/or/xor/mod-smi) hand-emitted in bigint_wasm.rs + binop/
  unary hooks. REVERTED: multiple validator mismatches fixed, but then
  runtime HANGS appeared in the mul/usub loops (probe hang at 240s) —
  not shippable. File is still in the tree (unregistered, compiles,
  harmless) as a starting point; emitter_state FN 36..47 constants are
  RESERVED (FN_LEN kept at 35 so FN_START stays 36 — do NOT register
  without appending the emit calls AFTER len() in prelude::emit and
  bumping FN_LEN).
- Also needed: LitBigInt exact literals (>i64 currently clamp to 0),
  big AND/OR/XOR for negatives (two's complement), print of -0 handling.

## Diag tooling (tests/, kept)
- diag_run.rs: the gate (DIAG_ONLY=case filter; writes /tmp/dbg3.wasm).
- diag_dump.rs: dump fn N of DIAG_FILE (default /tmp/dbg3.wasm).
- diag_offset.rs / diag_ranges.rs / diag_types.rs / diag_fvalidate.rs /
  diag_validate.rs: offset->function mapping, type-section dump, full
  validation. NOTE: wasmparser error offsets LAG the real failing op
  by ~1-3 instructions; "(ref null $type)" renders BOTH $pyobj(0) and
  $bigint(3) concretely — identical strings, different indices.

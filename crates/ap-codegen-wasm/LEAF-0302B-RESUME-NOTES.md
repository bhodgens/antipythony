# Leaf 03.02b resume — WasmGC values/arithmetic/containers

Corpus slice scope (diag_run.rs, the byte-exact gate set):
- MUST be green byte-exact: V001, V051, V052, V055 (the parked blockers) —
  plus V056/V060 IF reachable with current lowerable surface.
- Explicitly UNSUPPORTED (fail-closed, fine): V047 (set/frozenset calls),
  V054/V012 (Try = 03.02c), V003 (For = 03.02c), V056 (Assign-aug forms?),
  V050/V059/V058 (class def / subscript calls), V005/V002/V004/V007/V010/
  V011 (divmod/round/int()/format builtins not on the slice), V008/V060
  (isinstance/format/f-string).

## Root causes found (2026-11-14 resume)

1. V052/V055 "expected i32, found (ref null $type)" @ _start: the bool
   $int build after FN_COMPARE (b13ed8f3) emits GlobalGet(bool),
   I32Const(0), I64ExtendI32U, RefNull(big), StructNew($int) — FIVE
   operands for a FOUR-field struct ($int = type,kind,smi,big). The smi
   payload is I64 but the code extends the unused-arg I32Const(0) and
   omits the kind field. The compare result i32 is left stranded on the
   stack; the validator reports the shift at the NEXT GlobalGet.
2. V051 "expected (ref null $type), found (ref $type)": the SAME block
   emits GlobalGet(<bool global>) whose global TYPE is `(ref null $type)`
   — GlobalGet returns non-nullable matching the declared type; the
   StructNew field-0 needs (ref null $type)... actually: validator says
   found (ref $type) vs expected (ref null $type) — global.get on a
   `(ref null $type)`-declared global yields the declared type; the
   mismatch is the StructNew(T_INT) field 0 expecting `(ref null $type)`.
   Resolved by matching the working emit shape (see bool_obj/truthy).
3. compare() stub (FT_TOP_I) returns constant 0 — V051/V052/V055 need
   the real dispatch (eq via FN_EQ + eq_full re-enabled; ordering ops
   stay unsupported on this slice but V051 uses `is`/== only).
4. int_to_str multi-digit bug: append_i64 writes digits BACKWARDS into
   scratch from DIGIT_SCRATCH-1 down, then copies cursor..DIGIT_SCRATCH
   — but stores '-' at ptr AFTER digits in output order... verified by
   V001 output "3 -3 9" for "1000000 -1000000 511": single digit ok,
   multi-digit reversed. Root: the copy loop emits cursor..END (most
   significant first at cursor) — check loop direction (br_if polarity).
5. eq()/compare() were left as BISECT STUBS at 8477cf2a (I32Const(0));
   eq_full() + real compare() bodies exist dead in the file.

## Engine notes
- wasmparser::TypeSection has no into_iter_count in 0.263 — collect
  into Vec then index (diag_types.rs).
- wasmparser 0.252 (wasmtime's vendored) vs crate 0.263 agree on the
  errors above; offsets differ slightly (validator frame attribution).

## Session log (2026-11-14, continued)
- diag tooling: diag_dump.rs (dump fn N of /tmp/dbg3.wasm) + diag_types.rs
  (type-section dump) landed as test files.
- Hand-decoded _start (V052): @1aa0 error sits at the FIRST
  `GlobalGet(g_bool)` of the compare-result bool build. The strand is the
  compare-link i32s: `for links { call FN_COMPARE }` pushes N i32s, the
  AND-reduce folds them, but the bool build (added in b13ed8f3) starts
  GlobalGet WITHOUT consuming the folded result — the emitted sequence is
  [i32(leftover)] GlobalGet I32Const I64ExtendI32U RefNull StructNew.
  I.e. SIX stack items for a 4-field struct: the folded i32 was never
  consumed as the smi payload; instead a FRESH I32Const(0) is extended.
  The V051 variant reports the same site one field earlier.
- Fix plan (next): in Expr::Compare, order = GlobalGet(bool-type), kind 0,
  <folded i32 -> I64ExtendI32U> as smi, RefNull big, StructNew(T_INT).
  The folded result must be stacked AFTER the GlobalGet/kind pushes.

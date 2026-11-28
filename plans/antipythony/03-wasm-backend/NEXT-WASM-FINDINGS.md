# NEXT-WASM-FINDINGS — bughunt 2026-11-26 addendum section 6.1 (wasm backend)

- Worktree: ~/.git/worktrees/leaf-wasm-findings (branch fix-wasm-findings off main@468e24d)
- Date: 2026-11-27
- Input: .hermes/audits/2026-11-26-bughunt-addendum.md section 6.1, findings
  D1, D3, D5, D6, D8, D11, D12 (D2/D9/D10 deferred, priced below).
- Session note: this leaf ran twice (an earlier session left WIP on the
  branch); a `git checkout -- .` recovery mistake in THIS session discarded
  the uncommitted tree — recovered losslessly from the stash commit
  0afd77df (git fsck), re-applied the two post-stash fixes (str-walk Br(2),
  emit_try_finally duplicate-End / success-Br depth), then committed.
  Lesson: commit at every green probe, never `git checkout -- .` with a
  dirty tree.

## Per-finding records

### D1 — boxed bool always truthy — FIXED (reproduced, root-caused, pinned)
- Auditor claim: type_sel assigns selector 2 to bool; truthy (~785-917) and
  bool_obj (~464-603) test selectors 0,1,3,4,5,6,7,8 only; a boxed False
  keeps the default truthy=1.
- Verification: REPRODUCED on main binary. `b = False; if b:` printed "then";
  `bool(b), not b` printed "True False".
- Root cause: exact as claimed — no sel==2 arm in either function.
- Fix: both functions grow an `If(sel==2)` arm casting to $int and reading
  the smi payload (I64Ne 0 -> flag). No new prelude fn.
- Evidence: probe d1_bool now prints else/False/True identical to CPython
  3.14.7; pinned as classes_wasm/CLW013-bool-truthy-and-floateq (GREEN).

### D3 — i64::MIN / zero-limb $bigint traps in big_bits — FIXED
- Auditor claim: i64::MIN (and any zero-limbed $bigint from big_norm) has
  EMPTY limbs; big_bits reads limbs[0] unconditionally -> array.get OOB trap;
  `print(-(2**63))` traps.
- Verification: REPRODUCED (`-(2**63)` -> wasm trap on main).
- Root cause: big_from_i64 wrap-abs ANDs i64::MAX FIRST, so MIN -> 0 -> empty
  limbs; big_bits has no n==0 guard.
- Fix: (a) big_bits returns I64Const(0) when limbs.len()==0; (b) big_from_i64
  reorders wrap-abs so MIN's magnitude stays 2^63 = limb (0, 0x8000000000000000).
- Evidence: `-(2**63)` prints CPython-identically (probe d3_i64min).

### D5 — bare `raise` null-deref trap — FIXED
- Auditor claim: bare raise reads global active_exc which only generator
  paths set -> null -> exc_raise RefTestNullable(true-for-null) -> StructSet
  on null -> trap. `try: raise ValueError('v') / except ValueError: raise`
  traps.
- Verification: REPRODUCED (wasm trap on main).
- Root cause: emit_try_handlers copied the payload only to TEMP_LOCAL_A;
  G_ACTIVE_EXC stayed null outside generator lowering.
- Fix: emit_try_handlers GlobalSets active_exc with the caught payload on
  handler entry; emit_raise, when active_exc is null, builds and throws
  RuntimeError("No active exception to reraise") (message interned once per
  module; the FN_EXC_RAISE return value is Dropped inside the If(Empty) arm —
  arms leave no values, 03.02c engine fact; the first draft of this fix had
  both a duplicated type-push and a missing Drop and failed module
  validation until both were corrected).
- Evidence: d5_reraise reproduces CPython's "ValueError: v" traceback rc=1;
  d5_top_raise raises the RuntimeError; `except KeyError as e` binding works
  (d5_bind). Pinned EXW010/EXW011 (GREEN, stdout-observable shapes).

### D6 — float==float always False across objects — FIXED
- Auditor claim: eq_full's float arm requires one side selector 0 or 2;
  float/float falls to I32Const(0).
- Verification: REPRODUCED (`x=1.5; y=x+0.0; x==y` -> False).
- Root cause: exact as claimed.
- Fix: a both-sel==1 arm before the mixed-numeric arm: cast both to $float,
  struct.get F_FLOAT_VAL, F64Eq, Return. f64.eq gives -0.0==0.0 true and
  NaN!=NaN false — both CPython-exact by construction.
- Evidence: d6_floateq prints True True False / True. Pinned in CLW013.

### D8 — infinite recursion = raw wasm trap — FIXED
- Auditor claim: frame_depth global incremented, no limit check; host trap
  measured between 3000 and 5000 frames.
- Verification: REPRODUCED (wasm stack-overflow backtrace on main).
- Root cause: exact as claimed.
- Fix: frame_push compares frame_depth >= 1500 (safely under the measured
  host trap floor, comfortably above legitimate corpus recursion) and throws
  RecursionError("maximum recursion depth exceeded") inline — the
  super_new RuntimeError construction pattern, no new prelude fn. The
  message string is interned in emit_all before frame_push. frame_pop's
  existing decrement was already correct; recovery verified with a 1000-call
  returning loop (d8b prints 499500).
- En-route discovery: EXC_TREE had no RecursionError row, so
  exc_type_global("RecursionError") returned None and the unwrap_or(0) fed
  the None GLOBAL to exc_new's $type param — a module-validation failure
  ("expected (ref null $type), found (ref null $type)" on distinct types 0/1).
  Fixed by adding ("RecursionError", "RuntimeError") to types.rs EXC_TREE
  (matches ap-run exc_hier.rs).
- Evidence: d8_rec catches RecursionError; d8d15 raises it with the
  CPython message at depth ~1500 (CPython's own limit ~1000 differs in
  DEPTH NUMBER — message text exact, the numeric limit is a separate pin
  surface; documented honestly).

### D11 — try/finally skips finally on exception path — FIXED
- Auditor claim: try/finally emits finalbody straight-line after the handler
  chain, so a raise in the body never runs it; `return` in try must also run
  finally; mirror c633a0b's bytecode semantics.
- Verification: REPRODUCED (d11_finally printed no "cleanup" before the
  traceback on main).
- Root cause: exact as claimed.
- Fix: emit_try_finally — Block($fin-done Empty) > Block($fin-catch, result
  exc-ref) > try_table Catch::One(TAG_PYEXC, label 0) over the guarded region
  (body/handler chain; try/else routes orelse INSIDE the guarded region so an
  else-raise also runs the finally). On catch: payload -> active_exc
  (bare raise in finally re-raises), finalbody runs with ctx.fin_bodies
  DISABLED, payload rethrown. On success: RefNull payload, null-test, Br to
  $fin-done. Stmt::Return inlines pending ctx.fin_bodies (innermost first)
  before emitting the return — the c633a0b inlining discipline.
- Three emitter-contract corrections hit en route: (1) Catch::All cannot
  target a block WITH a result type ("catch_all label must have no result
  types") -> Catch::One(TAG_PYEXC) covers the same surface since every
  Python exception here throws via that tag; (2) the success Br depth was
  wrong twice (the $fin-catch End sits BEFORE the null-test, so the success
  Br targets $fin-done at depth 1, not 2); (3) a duplicated $fin-catch End
  made the EH-wrap payload ref.null land OUTSIDE its block ("expected (ref
  null $type) but nothing on stack"). All diagnosed from wasmparser
  Validator output + wasmprinter wat dumps (diag_fvalidate/dump_wat_full).
- Evidence: d11_finally prints "cleanup" then the ValueError traceback
  (CPython-identical); d11_ret prints fin/1. Pinned CF019 (GREEN).

### D12 — shapes that trap instead of raising — FIXED (a,b,c,d) / 1 sibling-owned residual
- (a) `in`/`not in`: REPRODUCED (trap). cmp_op maps In/NotIn -> OPS::IN but
  FN_COMPARE's opcode chain has no IN arm -> Unreachable. Fix: emit_compare_link
  routes In/NotIn to FN_ITER_CONTAINS (arg order container,item) with I32Eqz
  for NotIn, at BOTH compare sites (emit_expr Expr::Compare and
  emit_cond_i32). GREEN (d12_in: True False).
- (b) str ordering: REPRODUCED (trap). compare's ordering arm handled only
  numerics (bool-normalized sel<=1) and Unreachable'd otherwise. Fix: a
  str/str arm (both sel==3) before the numeric check: byte-walk verdict
  local (-1/0/1) with early exit, prefix tie-break on lengths, then a
  LT/LE/GT/GE dispatch chain on the verdict; FT_TOP_I grew 4 i32 pad locals.
  The arm/End/Else plumbing cost three validation rounds (missing numeric
  If-Else, then a missing strIf End, then the walk's mismatch-exit Br hit the
  Loop label instead of $wend — an infinite loop caught by the 60s probe
  timeout, fixed Br(2)). GREEN (d12_strcmp True == CPython).
- (c) tuple except-target: REPRODUCED (trap — exc_match RefCasts its target
  to $type). Fix: emit_try_handlers expands Some(Expr::Tuple(items)) into an
  OR-chain of exc_match tests accumulating into TEMP_LOCAL_CMP. GREEN
  (EXW012: both elements match, ValueError falls through).
- (d) missing attribute call: REPRODUCED (trap — attr_get null then
  RefCastNullable($function)). Fix: null-test after attr_get; on null build
  + throw AttributeError("' object has no attribute '<name>'") via
  FN_EXC_NEW/FN_EXC_RAISE (message interned through scan.intern_str at the
  Expr::Attribute scan site — ctx.tab is not mutably reachable at emit); the
  non-null path re-loads the attribute before the $function cast. Message
  text HONEST DIVERGENCE: CPython says "'C' object has no attribute
  'missing'" (type name interpolated); ours omits the type name (full
  message assembly needs str_cat chains — filed, see deferred pricing).
  GREEN (EXW013 pins the catchable-raise surface).
- Binop ZeroDivision (`1 // 0`): VERIFIED SIBLING-OWNED — the trap lives in
  the binop function body (prelude.rs binop ~3023; MOD/FLOORDIV zero checks
  are bare Unreachable at ~3175/~3207, DIV true-division Unreachable at
  ~3247). NOT TOUCHED per region ownership; see deferred pricing below.

### D2 / D9 / D10 — DEFERRED (report only; priced patches for the parent)

#### D2 — int overflow -> bigint promotion (DIAGNOSED-DEFERRED)
- Site: prelude.rs binop (~3023), the smi I64Add/Sub/Mul arms (~3100-3140).
- Shape: arms compute I64Add etc. directly; any overflow wraps silently
  (CPython promotes to $bigint via big_from_i64/FN_BIG_NORM). Probe:
  `print(2**62 + 2**62)` wraps instead of printing 2^63.
- Required patch (sibling-owned region): in each of ADD/SUB/MUL smi arms,
  wrap the raw op in an overflow check — wasm has no i64 overflow trap, so
  the check is arithmetic: for ADD `b>0 && a>i64::MAX-b || b<0 && a<i64::MIN-b`
  (5-7 instructions via I64Const/LocalGet comparisons); on overflow call
  FN_BIG_FROM_I64 on both operands and route to a bigint ADD path (big_norm
  limb-wise add ~20 instructions; SUB likewise; MUL needs the full limb
  schoolbook — the limb core that 03.02b deferred). Estimate: ADD/SUB ~60
  emitted instructions each + a shared bigint add/sub helper ~120; MUL needs
  the limb core first (the priced follow-up leaf "limb core" in
  plans/antipythony/03-wasm-backend). If the sibling's binop work lands
  first, this rides it.

#### D9 — float repr shortest-roundtrip (DIAGNOSED-DEFERRED)
- Site: prelude.rs py_str (~2458) float arm ~2533-2546: casts F64 -> I64
  via I64TruncSatF64S and prints through FN_INT_TO_STR — prints the
  TRUNCATED int (1.5 -> "1", 0.5 -> "0").
- Required patch: a shortest-roundtrip formatter. Options priced: (a) Ryu
  algorithm inlined as a prelude fn (~300-400 instructions + a 700-entry
  double-precision table in the data segment, exact CPython repr for all
  doubles); (b) bind Rust's ryu crate offline: format at MODULE BUILD time
  for literals only — insufficient (runtime float math needs runtime
  formatting). Realistic shape: (a), one new prelude fn FN_F64_TO_STR
  appended at the tail (COORDINATION: the no-new-prelude-fns freeze in this
  leaf does not bind the next leaf; index = next after FN_BOUND_OBJ(75),
  FT table append). Estimate ~1 leaf.

#### D10 — int ordering above 2^53 (DIAGNOSED-DEFERRED)
- Site: prelude.rs compare (~1682) numeric arm: va/vb load via int_smi OR
  FN_BIG_BITS then F64ConvertI64S (~3310/3325 for the f64 arms; ~1927/1952
  in eq_full's float_py_eq region) — bigints >2^53 lose low-bit exactness in
  the f64 conversion, so `2**53` vs `2**53+1` compares EQUAL.
- Required patch: a BIG/BIG ordering arm comparing via FN_BIG_CMP (33, already
  in the prelude surface, currently used by... check) instead of the f64
  conversion: both-sel-int + either-BIG -> call FN_BIG_CMP, map -1/0/1
  through the existing LT/LE/GT/GE chain (the str arm's dispatch shape is
  reusable verbatim). ~40 instructions + the mixed BIG/smi cases via
  FN_BIG_FROM_I64 normalization. Estimate: half a leaf; only after the D2
  limb core (BIG_CMP exists but bigint construction beyond i64::MIN range
  still needs it).

## Honest scope
- All seven targeted findings reproduce, fix, and pin GREEN. No finding was
  weakened, no pin weakened; the corpus only grew (7 new cases).
- Known residual divergences, all documented above: RecursionError fires at
  depth 1500 vs CPython's ~1000 (message exact, number differs); missing-attr
  message omits the type name; KeyError repr-quoting in print(e) (pre-existing
  exc_msg surface, avoided in EXW012's pin shape); traceback stderr surface
  (filename .py suffix + source-line echo) remains unpinned family-wide
  (pre-existing; all new pins keep rc=0 or catch+print shapes).
- `1 // 0` ZeroDivisionError and `1 / 2` true division: sibling-owned binop
  body; NOT edited. Diagnosis + pricing above.

## Process notes
- Validation loop: wasmparser Validator (diag_fvalidate) + wasmprinter wat
  dumps (dump_wat_full, offsets prefixed) localized every emitter-contract
  violation; the ap_host error string ("failed to compile: function[N]")
  never identifies the instruction.
- Local-grid contract held: FT_TOP_I's new locals were appended to the
  locals decl (params 0-2, named locals 3-8, new 9-12); frame_push's new
  exception scratch is local 6 (after its declared pads).
- Stale-binary gotcha hit twice: `cargo build -p ap-cli` finishing in
  <0.5s without relinking the bin after a lib-only change produced identical
  wasm bytes; `touch lib.rs` + rebuild, or verifying bin mtime, before
  concluding "bytes didn't change".

# Leaf 03.04c session record (2026-11-17, tier1-0304c worktree)

## What landed (3 commits on tier1-0304c off 6e638ad5)
1. Increment A: scan accepts generators/For/Match/comps (gates + capture
   collection, fail-closed inner-slice checks). CF 18/18 + classes/exc 16/16 held.
2. Increment B: Stmt::For emit, range/list/sum/next/iter builtins,
   list_drain+sum_drain drivers (FN 71/72 emitted AFTER emit_super so
   FN_SUPER_NEW=69/FN_TYPE_OF=70 consts stay true), TEMP_LOCAL_IT=52 +
   range scratch 53-59 in every frame.
3. Increment C: For exhaustion/else/break byte-exact; stop_iter bare-vs-valued
   args fix (null value => args=(), valued => args=(value,)); for_probe 5/5.

## Probe verdicts (linchpins)
- array.new_fixed VALIDATES on mutable array types (nfprobe) — dynamic lists OK.
- wasm-encoder/wasmparser 0.263 RecSection API: types.ty() returns the RecSection
  directly; wasmprinter 0.263 Print trait = write_str/newline methods over any
  fmt::Write... (sink struct with write_str prefixing line numbers + start_line
  offset map). Tools live in tests/: dump_validate_locate.rs (validate+wat-locate),
  dump_wat_full.rs (full wat to /tmp/full.wat), dump_wat_offset.rs.

## For-loop emit shape (increment C, byte-exact 5/5)
$exit{ $brk{ Loop{ $cx(result exc){ try_table(catch->label0=$cx){ next; bind
target } RefNull(exc) } A=payload; $chk{ A null? If{ br 1 } /*->body*/;
SI? exc_match(A,G_SI) If{ br 3 /*-> $brk = else*/ }; rethrow A } body;
br 0(Loop) } } else } — branch depths: null-guard br = chk_depth+1 from
inside the If (br 0 inside If = fall-through no-op, the classic wasm trap);
SI-exhaust br = brk_depth+1 (If adds a label level); break = ctx.br_break =
exit_depth (skips else); continue = head_depth.

## Gotchas hit (recorded for the next session)
- FT consts vs emit order: the drains MUST be emitted after emit_super or
  FN_SUPER_NEW/FN_TYPE_OF consts point at the wrong fns (cost: 40min of
  validation hell) — emit order IS the index contract.
- add() locals-decl first entry lands at (first param index + nparams): for
  FT_O_O param-free locals start at local 1 — pass (1, ...) not (2,...).
- printer stops at the first malformed function: dump_validate_locate gives
  the offset, but the wat tail may be missing; use function-index + separate
  dump_wat_full pass.
- is_supported_builtin MUST include every name the emit arm handles — a
  missing entry routes to generic py_call dispatch on a null global (silent
  trap, no validation error).
- for-range prints were empty while rc=0: the shape silently iterated to
  exhaustion without ever running the body (null-guard no-op) — empty output
  + rc=0 is the signature of a swallowed control-flow bug, not a compile skip.

## NEXT-SESSION DESIGN (continuation — GE/MA gates)
1. GE001 (yield/next/exhaust): generator lowering. is_generator flag threads
   through scan_fn_common (signature already has the param; Item::FnDef +
   lambda call sites updated). Step-fn emit: FT_GEN_STEP has TWO params
   (gen, send-value) so mirror locals start at 2 (lesson 1). Flatten the body
   CFG: walk stmts assigning each yield a state id; emit if-chain on
   gen.state (br_table equivalent; the if-chain variant validated fine in
   03.04b's gen_send). Yield = store value -> gen.value, state = resume id,
   return; resume = jump into the matching arm. StopIteration on exhaustion:
   state 0 + FN_STOP_ITER. EH-inside-step (GE003): the gen_throw prelude
   driver handles state<=0; body exceptions store to gen.exn and force
   state=0 (caller sees the raise on next resume).
2. GE002 (send): FN_GEN_SEND prelude driver is REAL (d15b3a4f) — the Call
   side just needs emit: args (gen, value) -> FT_GEN_SEND. g.send(v) method
   arm in emit_attr_call or builtin slice.
3. GE004 (yield from): de-sugar to a loop of next()+send relay, or fail-closed
   for this slice if GE004's expected pins use it — CHECK the pins first.
4. GE005 (comprehensions): ListComp/DictComp inline as hidden fns already
   scan (<listcomp> via scan_fn_common); emit via the For shape + drain into
   list_drain; Genexp = hidden generator fn (is_generator=true, one yield).
   SetComp stays fail-closed.
5. MA001 (match): scan gates landed (collect_match); emit = chain of
   FN_DGET/FN_DICT_HAS + sequence len checks per the PROGRESS design slice;
   Class/Value/Mapping-**rest fail-closed (scan already rejects).
6. Gates to add: crates/ap-codegen-wasm/tests/generators_wasm_gate.rs +
   match_wasm_gate.rs following controlflow_wasm_gate.rs (ap_host::run,
   byte-exact vs tests/corpus/<suite>/expected/ pins).
7. Baselines to hold: controlflow_wasm_gate 18/18, classes_exceptions 16/16,
   ap-host 3/3 (all green at increment C).

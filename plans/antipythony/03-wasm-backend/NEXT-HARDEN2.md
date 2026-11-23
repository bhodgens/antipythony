# Leaf HARDEN2 — scan-gating extension: the REMAINING planner fail_closed_gen arms

**Worktree:** `~/.git/worktrees/leaf-harden2` (branch `tier1-harden2` off
main@1cd873c)
**Date:** 2026-11-22
**Input:** NEXT-HARDEN.md honest gap #3 (lines 69-72/80-84): the OTHER
planner `fail_closed_gen` panics — gen if/while/for condition yields,
handler bind-yield — have no corpus case and no scan-time gate. Commit(s)
in this leaf; branch `tier1-harden2`.

## The contract (unchanged from HARDEN)

A scan (`collect_*`) must NEVER accept a source shape the emitter cannot
lower (planner `fail_closed_gen` panic / emit `unreachable!("scan
vetted ...")`). Disposition discipline: REJECT AT SCAN with an
`Unsupported` reason naming the owning post-03 slice (03.04e generator
segment slice), matching HARDEN's reason-string style. No emit arm was
implemented anywhere (all gated shapes need planner/segment-machine
surface that does not exist).

## Probe-first baselines (probe_harden2_baselines.rs)

21 minimal .py shapes lowered behind `catch_unwind` at BASE (pre-gate):
**18 LOWER-PANIC** with these planner reasons (lower.rs:2420 / 6147):

- `yield in a gen if-condition` — `if x + (yield 1):`
- `yield in gen while-condition / while-else` — `while x + (yield 1):`,
  and `while ...: ... else:` (the else-clause alone trips the same arm)
- `yield in gen for-iter / for-else` — `for i in (yield 1):`,
  `for i in ...: ... else:`
- `handler bind-yield beyond the slice` — `except E as e: y = yield 2`
- `yield nested in a handler beyond the slice` — `except E: if True: yield 2`
- `yield in gen return` — `return (yield 1)`
- `break/continue outside a gen loop` — `break` whose gen loop the
  planner never entered (yield parked inside the try between head and ctl)
- `yield nested beyond a top-level ExprStmt` — `print((yield 1))`
- `yield nested in assign value beyond the slice` — `x = [(yield 1)]`
- `generator assign target beyond a name` — `o.k = (yield 2)`
- `gen stmt beyond the segment slice: LocalDef(...)` — `def` inside a gen
- `gen stmt beyond the segment slice: Match(...)` — `match` inside a gen
- emit-side `scan vetted expr: Yield(..)` (lower.rs:6147) — split-yield
  with non-Name lhs (`x = f(1) + (yield 2)`, `x = o.k + (yield 2)`)
- yield-lambda — `lambda: (yield 1)` registers `<lambda>` with
  `is_generator=true`; the emit-side lambda lookup filters
  `is_generator == false` -> `rposition().expect("scan registered this
  lambda")` panic

Plus 3 shapes ALREADY fail-closed (pre-existing scan rejects, unchanged):
tuple unpacking assign target, AnnAssign, and the unresolvable-free-name
pass (which was absorbing the split-lhs shapes with a less precise reason).

## Gates landed (crates/ap-codegen-wasm/src/lower.rs, scan pass only)

All keyed off the HARDEN `parent_stack` generator-ness pattern, now
factored into `scan_in_generator(scan)` (+ `gen_loop_bump` for loop-depth
tracking, `gen_gate(fires, what)` for the reason strings):

1. **ExprStmt nested yield** -> "yield nested beyond a top-level ExprStmt
   (03.04e segment slice)" (top-level Yield/YieldFrom still allowed)
2. **Assign yield shapes**: non-Yield/YieldFrom/split-BinOp value -> assign
   value; non-Name split lhs -> split-lhs; non-Name targets -> bind-target
   (all "(03.04e segment slice)")
3. **AugAssign yield** -> "yield in a generator augassign (03.04e segment
   slice)" (the gen AugAssign arm is Seq-only — no split exists)
4. **Return yield** -> "yield in a generator return (03.04e segment slice)"
5. **If condition yield** -> "yield in a generator if-condition (03.04e
   segment slice)"
6. **While condition/else** -> "yield in a generator while-condition/else"
7. **For iter/else + non-Name target** -> "yield in a generator
   for-iter/else" / "generator for target beyond a name"
8. **Handler split** (per handler stmt, in gen): Assign(_, Yield) ->
   "yield binding in an exception handler"; any other yield-carrying stmt
   -> "yield nested in an exception handler" (top-level ExprStmt(Yield)
   still allowed — GE003 inbody survives)
9. **Orphan break/continue** (gen-loop depth tracked through While/For
   bodies; reset at nested fn scopes) -> "break/continue outside a
   generator loop"
10. **LocalDef / Match inside a gen body** -> "statement beyond the
    generator segment slice"
11. **Lambda body yield** (NOT gen-scoped: any yield in a lambda is a
    SyntaxError in CPython) -> "yield in a lambda body (03.04e segment
    slice)"

## Regression tests (the contract, going forward)

`scan_vet_contract.rs` gained `harden2_gen_planner_gates_reject_at_scan`:
21 rows (19 gated + 2 IN-slice control shapes that must KEEP lowering —
GE002 echo-style while-split and a plain top-level handler yield — so the
gates cannot silently over-reject). Every gated row asserts the rejection
reason names the owning slice. A panic = FAILURE.

**Corpus discipline note:** NO corpus case was added. A scan-rejected
source cannot live in a dumped wasm family (`cli_corpus_dump`
`.expect("lower")`s every case — a gated row would break the corpus gate
itself, and corpus only grows with cases that LOWER). The HARDEN2 sources
are pinned inline in the contract test instead. An H2P1 pin attempt
(CPython-verified, then removed): the shapes are SyntaxError-free and
CPython-green, but not wasm-runnable by construction.

## Verification (this tree, this session, CARGO_TARGET_DIR=$PWD/target-harden2)

- Probe: 21/21 handled post-fix (18 gated + 3 pre-existing rejects),
  LOWER-PANIC 0 (base: 18 panics).
- `cargo test -p ap-codegen-wasm`: 42/42 suites ok (incl. scan_vet_contract
  2/2 tests, cli_corpus_dump, all diag suites).
- not_lowered_inventory sweep: **LOWERS 52 / UNSUPPORTED 168 / other 0 /
  LOWER-PANIC 0** — byte-IDENTICAL per-row vs the base tree (diffed full
  reason lists; stash A/B). Zero rows changed disposition: NO corpus case
  exercises the new gates (the honest-gap premise confirmed). The
  unsupported total vs HARDEN's recorded 159 grew on MAIN before this leaf
  (corpus additions in S016/S016B/S016C + 2a), not by this leaf's gates.
- CLI corpus gate (`scripts/cli_corpus_gate.py`, fresh debug ap-host +
  regenerated dumps): **52/53** — the 1 fail is
  `generators_wasm/GE006-genexp-lazy-protocol`, **PRE-EXISTING at base**
  (stash A/B reproduced the identical mismatch on the unmodified tree).
  GE006/GE007 landed in the 2a leaf gated by ap-conform (native leg) 7/7
  only; the wasm CLI gate was never re-run after them. GE007 IS
  byte-exact on wasm. GE006's wasm-leg laziness gap (the `produce N`
  prints missing — the genexp body evaluates eagerly in the wasm
  emitter's hidden-fn step state) is an EMITTER gap outside this leaf's
  scan-gating ownership; filed for the owning slice.
- controlflow_wasm_gate: 18/18. classes_exceptions_wasm_gate: 17/17.
  for_probe: 5/5. ge_ma_probe: GE001-005 + MA001 PASS. ap-host suite 8/8.
  ap-cli wasm_build_gate 3/3 (the CLI verb path over the new gates).

## Honest gaps / notes

1. **GE006 wasm-leg mismatch is pre-existing and OPEN** (not mine, proven
   by stash A/B). Owner: the 03.04d/e genexp emitter slice (hidden-fn
   laziness). The 2a leaf's "corpus only grows" added a case the wasm gate
   had never measured; this leaf re-ran the gate honestly and records the
   fail rather than hiding it.
2. `gen-assign-tuple-target` and `gen-yield-lambda` now reject with the
   NEW more-precise gen-gate reasons instead of the older generic ones
   (tuple: "unpacking/subscript/attr assign target (post-03.03)"; lambda:
   would-be emit panic) — same disposition class, sharper reason strings.
3. `split-yield-lhs-attr` (`o.k + (yield 2)` with unresolvable `o`) is
   caught by the split-lhs gate BEFORE the unresolvable-free-name pass —
   reason text changed vs base (was "unresolvable free name in fn scope"),
   same disposition class.
4. The `gen stmt beyond the segment slice` catch-all arm (Raise/Global ok;
   With/Assert/AnnAssign/Import...) remains planner-side for shapes the
   scan already rejects generically (`stmt_unsupported`); only LocalDef and
   Match needed explicit gates (the scan accepted them, the planner
   didn't).
5. cargo fmt reflowed one unrelated `matches!` in emit_call (formatting
   only); all other changes are the gates + helpers + tests.

# Leaf 03.04d session record (2026-11-17, tier1-0304d worktree off main@227db2d6)

## What landed (1 commit: eecf9a77)

**Increment A: drain/list type unification.** Three never-executed-path bugs in
the 03.04c prelude, all found by running the first REAL drain probes
(`tests/ge_run_probe.rs`, PROBE_CASE env picks a case):

1. `emit_drain_loop` (prelude_gen_drain.rs) rethrew EVERY non-null catch
   payload — including the StopIteration that IS normal exhaustion. `list()`
   and `sum()` always died with an uncaught StopIteration. Fix: SI-match ->
   exit $out with the accumulator; anything else rethrows.
2. `ref.cast(mut $obj_arr -> RO $obj_arr_ro)` VALIDATES (both are array refs)
   but TRAPS at runtime — unrelated array struct types. The drain tail's
   items cast hit this. ROOT FIX: `$list.items` flipped to the MUT `$obj_arr`
   type (types.rs 13). Readers updated per arm:
   - `list_eq` (prelude.rs ~1272): ArrayGet(T_OBJ_ARR) on both sides.
   - `py_str` list arm: items ride a DEDICATED local 13 typed T_OBJ_ARR
     (lesson 9 corollary: MUT array refs also cannot ride mismatched/oref
     locals — the dict arm's local-11 pattern, now twice).
   - `iter_contains` (prelude_gen.rs): RO local 2 (tuples) + MUT local 7
     (lists), item read branches on RefTest(T_LIST), n via
     If(Result(I32)) { get 2 len } Else { get 7 len }.
   - `iter_of` $list path: items (fresh mut array owned by the $list) IS the
     snapshot — no copy loop; local 3 = items directly, n from local 3.
   - literal `Expr::List` emit (lower.rs, both scopes): ArrayNewFixed on
     T_OBJ_ARR (new_fixed works on mutable arrays; the RO form was forced
     only by the old field type), stashed in the MUT objarr locals.
   Tuple items stay RO (unchanged). probes: `list([1,2])` -> `[1, 2]`,
   `list((1,2))`, `iter([5,6])` partial drain, `sum(range(4))` -> 6 all
   byte-exact rc=0.

**Baselines held at eecf9a77: controlflow_wasm_gate 18/18,
classes_exceptions_wasm_gate 16/16, for_probe 5/5, ap-host 3/3.**

## Gate status vs the leaf exit contract: GE001-GE005 + MA001 NOT MET

Honest state: the corpus cases still fail. Verified probe verdicts (each case
parse->lower->validate->ap-host, vs the expected/ pins):

- GE001: FIRST next() works (0), SECOND resume throws StopIteration(42)
  immediately — the WIP statement-granular flattener parks at a state whose
  content is the code AFTER the whole while, so the resumed state re-runs
  `return 42` without ever re-testing the loop condition.
- GE002: panics at emit — `total + (yield total)` (yield nested in a BinOp)
  reaches `emit_expr`'s unreachable arm (Yield is not a value expr there).
- GE003/GE004/GE005/MA001: blocked behind the same flattener (no
  yield-from/completion bind arms; comps still lower via the OLD non-gen
  path in scan; match has scan but NO emit arm at all).

## WIP reverted (IMPORTANT for the next session)

A residual-tree flattener rewrite (state k = body tree with the first k
yield-site prefixes dropped) was attempted and REVERTED before commit — it
cannot be right for loop-yields: a `while True: x = yield` site stays LIVE
across iterations, so a one-shot prefix-drop kills the construct on resume
and unrolls the loop into a linear path. Two scan-side pieces of that
attempt (comp hidden fn = generator via yield-elt; walrus-via-yield note)
were also reverted with it — scan comp fns are back to `is_generator=false`
with plain element ExprStmt bodies.

## NEXT-SESSION DESIGN (the flatten must be a SEGMENT MACHINE, not residuals)

Structure the step fn as numbered SEGMENTS; $gen.$state = resume segment.
Every segment's wasm Block is OPEN during dispatch (nested Block(0..n-1)
+ br_table, exactly the current dispatch shape), so a segment can `br`
OUTWARD to any later segment's block = forward goto. Loop back-edges need
an enclosing Loop: wrap the segment range [head..end) of each loop in a
`Loop` whose opening sits immediately before segment `head`'s End.

Segment kinds:
- Seq(stmts) — straight-line (no yields; the planner splits sites out),
  falls through to the next segment (physical adjacency = the fall-through).
- Test{cond, yes, no} — If { br yes' } Else { br no' }; both branch depths
  are +1 (inside the If).
- Park{value_expr, next, kind} — spill mirrors, $value = value, state =
  next id, return value. kind: Bare | Bind(slot) | Split(op, lhs, slot).
- Rebind{slot} — resume side of Bind: slot = $gen.$value.
- SplitResume{op, lhs, slot} — resume side of Split: slot = lhs OP $value.
- ForInit{it} — it_slot = iter_of(it) ONCE (it_slot rides $gen.$locals at
  the hidden slot `__ap_yf_it`-style; a For-inside-gen MUST carry its
  iterator in a slot, never local 52 — 52 dies at the park).
- ForNext{target, exit} — try_table guarded iter_next: got -> bind target,
  fall through; SI -> br exit.
- YfNext{done} — yield-from next over the it_slot: got x -> Park(x) whose
  park id = THIS segment (park-to-self); SI -> the completion bind
  (YfDone{slot}: slot = $gen.$value, which gen_send parked there before
  stop_iter threw).
- TryWrap{inner, after, handlers} — the GE003 shape needs try/except whose
  body contains parks: each park inside a try gets its OWN TryWrap re-opened
  on the resume path (split the try body at the first top-level site into
  pre/post regions; post gets a fresh wrapper with the same handlers).
  GE003's exact bodies: [yield 0, raise KeyError] under try + [yield 1,
  yield 2] under try — per-park wrapper re-open.
- Return{value} — state=DONE, $value = value (None if bare), return null.

Planner: build the body into segments with explicit continuations (forward
jumps resolved at emit time by br depth = target_id - current_id - 1,
counting from OUTSIDE any If/try_table wrappers the emitter opened — add
+1 per wrapper entered). GE002's `total + (yield total)` = Park with
kind=Split(Add, Name(total), "total"); its resume = SplitResume — NO
expression-level yield evaluation anywhere.

Call sites: c.send(v)/c.close()/c.throw(e) method arms in the plain
emit_call Attribute path (FN_GEN_SEND/FT_GEN_SEND etc. are all landed and
driver-green since 03.04b/03.04c — only the dispatch arms are missing).

Comps: flip collect_comp_clauses to emit `yield elt` inside the hidden fn
(is_generator=true), then EVERY call site (`<listcomp>` name call) already
routes through emit_generator_call; wrap the result in FN_LIST_DRAIN for
ListComp/Genexp-as-list contexts, FN_SUM_DRAIN rides sum(x for ...), DictComp
= drain into FN_DSET pairs (needs a small dict-build driver or inline
rebuild-append + StructNew(T_DICT)). GE005 pins also need `y := x*10`
(walrus) inside a comp — the Named expr must bind the ENCLOSING scope's
slot (S-02-2; verify the hidden fn's slot table carries it via
collect_local_binds).

Match: scan gates landed (collect_match, 03.04c). Emit = chain of
FN_EQ literal compares, len checks (ArrayLen on list.items/tuple.items),
FN_DGET for str-key mapping + null check, Starred rest = list_drain over
the remaining items, Or = sequential tests, Guard = truthy check after
captures. All in one Test-chain per case with the captures bound by
gen_bind-equivalent name stores.

Debug tooling that worked: PROBE_DUMP=1 + dump_wat_resilient (DIAG_SRC /
WAT_OUT env) for post-crash wat; the identical-text validator trap
("expected (ref null $type), found (ref null $type)" = two DIFFERENT types
rendering identically) is ALWAYS a wrong array-type or local-slot type —
grep the local decls in the wat, not the instructions.

## Machine-policy note

One cargo lane observed; CARGO_BUILD_JOBS=4 used for gate runs.

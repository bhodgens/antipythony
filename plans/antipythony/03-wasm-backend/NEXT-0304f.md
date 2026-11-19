# NEXT-0304f — session record (03.04f, the LAST 03.04 session attempt)

Branch: `tier1-0304f` (worktree `~/.git/worktrees/leaf-0304f`), base = main.
Commit: `ff9035a` — machine-wide try_table + per-fn fixes.

## What landed (verified green)

- **Machine-wide try_table refactor** (emit_gen_step_fn): replaced the
  per-segment $cx/try_table re-opens with ONE machine-wide try_table around
  the dispatch Loop1, catch → close-chain (handler match → prefix /
  no-yield / park-yield dispatch), re-dispatch via a fresh $outer Loop.
  This fixed the class of bugs where body raises inside generator try
  blocks were silently swallowed (g4-raisemid, g11-firstraise, g2d probes
  all green).
- **Staged throw-in ($exn) entry check**: reads $exn → stashes to s_pad →
  clears the field → if non-null, raises inside the armed scope. Fixed the
  double-read bug where the re-read after the clear always saw null
  (close()'s staged GeneratorExit was silently swallowed — g5-close now
  byte-exact: `1\nclosing\nclosed ok\n`).
- **gen_close GE-swallow wrap**: the resume call_indirect wrapped in a
  $cx/try_table; GE escape = silent close OK (park CLOSED), other exc
  rethrown. Call operands moved INSIDE the result-block (03.04f: operands
  pushed outside starved the call_indirect → validation error).
- **GE005 partial (listcomp/genexp generator flip)**: hidden `<listcomp>`
  fns now scan as is_generator=true with `yield elt` bodies, register a
  module slot, and the ListComp/Generator emit lowers to a static
  generator call (emit_generator_call) with a per-module emit cursor
  (EmitCtx.comp_emit_cursor) matching hidden fns in scan order.
- **Walrus emit arm** (Expr::Named) in emit_expr: eval → bind (enclosing
  scope via emit_name_store) → result = value.
- **YfNext SI exhaustion arm**: SI payload extracted from the CAUGHT exc
  (scratch.b→a, not the null success slot), the YfDone bind INLINED at the
  SI site (no YfDone-segment dispatch), trampoline to Loop1 with
  state=done_next.
- **Baselines held** (verified this session on the worktree):
  controlflow_wasm_gate 18/18, classes_exceptions_wasm_gate 17/17,
  for_probe ok (5/5), ap-host 8/8.

## Probe status (ge0304f_bisect.rs, 25 cases — ALL GREEN)

g1-g11 (try/raise/handler shapes incl. first-raise, bare-except,
no-yield-handler, park-resume), f1-f6 (yield-from chain incl. inline-bind
variant), e1-e3 (comp prefix / walrus / genexp-next probes; e3 exposes a
SEPARATE known issue: list(gen) after next(gen) → `[]` — gen state not
preserved across next() on this slice).

## Honest gaps (NOT closed — 2+ attempts each, state recorded)

### GE003 — `closing` + `B end` still missing (MISMATCH)
- `closing`: closer's GeneratorExit handler (no-yield) does not run for
  GE003's module while the IDENTICAL closer (g5-close bisect probe) works.
  The staged-GE entry throw fires (backtrace: exc_raise ← step ←
  gen_close) but the machine catch does not deliver — chain/match never
  runs. g5b (BaseException handler) DOES catch the staged throw and runs
  its handler — the only delta found: GE003's module has OTHER generators
  (raiser, inbody) before closer; suspect per-module shared-global or
  machine-shape interaction, not the chain code itself.
- `B end` (inbody: `yield 'end'` after the try/handler): the seg after the
  HandlerPrefix park-resume is skipped — resume state lands one arm past
  the resume Seq (same class as GE004's missing `sub returned 42`).

### GE004 — `sub returned 42` + `empty gave 7` still missing (MISMATCH)
- The seg following the YfDone bind (print in the delegate; `v = ...`
  binding after `v = yield from empty()`) is skipped. The SI exhaustion
  arm extracts + inlines the bind + trampolines with state=done_next, but
  the re-dispatch lands 1+ arms late (deep-dive in this session traced it
  to the br-depth bookkeeping between the new machine-wide try_table
  shape and the legacy goto_x formulas — the depth tables need a
  principled recompute, not another +1/-1 probe).

### GE005 — comps/genexps partial (probe e1/e2 HOST-ERR; e3 partial)
- Hidden `<listcomp>` gen flips + walrus emit landed, but runtime still
  traps inside the hidden gen step (func 76) on `[x*x for x in range(5)]`;
  `list(gen)` after `next(gen)` returns `[]` (gen resume state not
  preserved on this slice); sum(genexp) needs FN_SUM_DRAIN (not started);
  DictComp needs a pairs-gen + dict-from-pairs driver (not started).

### MA001 — not started this session (emit arm; scan gates landed in
03.04c). Blocked behind the same post-handler dispatch skip (match cases
are arm-dispatched exactly like the GE004 segs).

## Root-cause state for the next session
- The machine-wide try_table refactor is the RIGHT shape (g-probes green)
  — do NOT revert it.
- The goto_x depth formulas (forward/fall/trampoline/done-escape) were
  tuned for the OLD per-segment $cx shape. The NEW shape's depth table
  needs recomputing ONCE from the block nesting: at seg k's body the open
  blocks are B(k+1)..B(n-1) + $sink + $done + Loop1 + tt + $cx + $outer
  (plus wrapper If/$chk when wrapped). The empirical depths found this
  session: SI-arm trampoline br(11) for n=10/k=2/extra=2 reaches Loop1 ✓
  (If=0, $chk=1, B3=2..B9=8, $sink=9, $done=10, Loop1=11). The
  post-handler "skip 1+ arms" symptom is the remaining discrepancy.
- dbg-marker prints inside step fns CLOBBER mirror locals (temp local
  indices collide) — do not debug step fns with buffer prints; use unique
  traps or module-level observables.

## Debug tooling added (kept)
- `ge0304f_bisect.rs`: 25 single-shape probes via PROBE_CASE (g*/f*/e*
  families) — the fastest way to re-enter this work.
- `APX_HOST_DBG_PIPES=1` host env (was reverted; re-add if needed):
  partial stdout on trap.

## Baseline verification (this session, this worktree)
- controlflow_wasm_gate: 18 passed, 0 failed ✓
- classes_exceptions_wasm_gate: 17 passed, 0 failed ✓
- for_probe: ok (5/5) ✓
- ap-host unit: 8 passed ✓


---

# 03.04g APPENDIX — leaf closure record (GE001-GE005 + MA001 ALL GREEN)

Branch: `tier1-0304g` (worktree `~/.git/worktrees/leaf-0304g`), base = main
@ 210be74.

## Exit gate (measured, this worktree)

- ge_ma_probe: GE001 PASS, GE002 PASS, GE003 PASS, GE004 PASS, GE005 PASS,
  MA001 PASS — all six byte-exact through ap-host vs the CPython 3.14.7
  pins.
- ge0304f_bisect: 25/25 g-probes ok.
- controlflow_wasm differential: 18 passed / 0 failed (of 18).
- classes/exceptions wasm differential: 17 passed / 0 failed.
- for_probe: 5/5 rc=0.
- ap-host unit: 8 passed / 0 failed.

## Root causes found (NONE were the goto-depth tables)

The leaf's opening hypothesis (recompute the goto br-depth tables for the
machine-wide try_table shape) was WRONG — the 03.04f formulas verified
correct by recount and by WAT inspection (`br(n-k+1+extra)` trampoline,
SI-arm br(11) for n=10/k=2/extra=2 reaches Loop1). Every open failure had a
planner/prelude root cause:

1. **Step-fn prologue never initialized the print-buffer pointer
   (TEMP_LOCAL=40)** — wasm locals zero-start, so the FIRST print in any
   step fn concatenated at buffer 0 and its output was silently lost.
   Explained GE003 `closing`, GE004 `sub returned 42` / `empty gave 7`,
   and several bisect probes. Fix: init local 40 = PRINT_BUF in the
   step-fn prologue ("2b. print-buffer pointer init (03.04g)").
2. **Park-in-loop-tail ignored `next_override`** — `gen_park_yield` parked
   at `seg.next_override.unwrap_or(park_next)`; a Park seg doubling as a
   loop body tail now correctly carries the loop back-edge (fnx/head), so
   `list(gen)` resumed the LOOP instead of falling to the done escape
   (generators stopped after one element). Fix committed as-is (the
   override wins; the planner guarantees the override is the back-edge).
3. **E_PEND fell to the done escape at top level** — statements following
   an If/handler in a stream were skipped (GE003's missing `B end`).
   Fix: plan_seq per-statement chaining rewrites E_PEND staged by stmt i
   to stmt i+1's first seg; the last stmt's E_PEND stays for loop passes
   / plan-gen.
4. **plan_seq([]) returned last=0 (garbage)** for empty streams — If arms
   with no else got a garbage else-tail id; the REAL pushed empty seg
   never staged E_PEND, so every false if-clause fell to the done escape
   and silently killed for/while+if-yield generators (GE005 evens/pair,
   all hidden comps with an if-clause). Fix: plan_seq([]) pushes its own
   empty seg and returns it as (first, last).
5. **Nested loop back-chain ownership** — the outer loop's back-chain
   unconditionally overwrote the tail seg (an inner loop's Park), and the
   inner ForNext exit was HARDCODED to the after-len (= done). Fix:
   (a) loop exhaustion exits via a CONTINUATION seg carrying E_PEND — the
   enclosing loop pass rewrites it to the OUTER fnx, top-level loops
   resolve to n; (b) back-chains only rewrite None/E_PEND/CONT_PEND
   overrides (an inner loop's Park-tail back-edge wins).
6. **comp hidden-fn cursor reset per module item** — every comp expr in a
   new statement picked hidden fn 0. Fix: one persistent module-ordered
   comp_emit_cursor.
7. **ListComp drained to a bare generator** — the generator object escaped
   into assignments/prints (py_str trap on null $str). Fix: ListComp =
   static gen call + FN_LIST_DRAIN; GeneratorExp stays bare (lazy);
   DictComp = pairs-gen + FN_DICT_DRAIN (new driver, FN_DICT_DRAIN=73,
   FN_FIRST_USER 73->74).
8. **Walrus inside a comp hidden fn** — Named fell through to plain
   emit_expr whose Name loads use plain-fn local numbering. Fix: Named arm
   in gen_emit_expr routing the value through TEMP locals + emit_name_store
   (binds the enclosing scope, S-02-2).
9. **dict_has (FN_DICT_HAS=21) was a STUB returning 0** — mapping match
   patterns never matched. Implemented (linear scan, str keys, mirrors
   dget) with typed locals + explicit casts (the identical-text validator
   trap: (ref null $pyobj) vs (ref null $dict) render as the same text in
   some contexts — declare locals TYPED and cast at use).
10. **FN_DGET / FN_DICT_HAS argument order** — emit pushed key before
    subject; both are (dict, key). Fixed at both call sites.
11. **Match $mend br depth** — the matched-body br used the depth AFTER
    opening the case If (targeted the If itself / outer blocks, ending the
    module). Fix: capture mend_depth after opening $mend and br to it.

## MA001 emit shape (as landed)

Per NEXT-0304d: subject in TEMP_LOCAL_A; ONE $mend Block around the match;
per case ONE test chain — emit_pattern_checks pushes i32 conjuncts folded
with I32And (Literal = FN_EQ; Capture/Wildcard = 1; Sequence = list/tuple
RefTestNullable + items stash (TEMP_LOCAL_OBJARR / _RO — NEVER cast across
array types, lesson 2) + ArrayLen shape check + per-element recursion
through TEMP_LOCAL_B; Mapping = T_DICT RefTest + FN_DICT_HAS per literal
key; Or = I32Or folds); guarded cases bind their captures BEFORE the guard
(the matched-so-far path guarantees the subject); the folded i32 drives
the case If — matched path binds captures (emit_seq_captures: elements via
array.get + emit_name_store; Starred rest = count-preserving copy loop into
a fresh T_OBJ_ARR + StructNew(T_LIST); mapping values via FN_DGET) then
body then `br $mend_depth`; fall-through = next case.

## Notes for the next leaf

- APX_PLAN_DUMP=1 env-gated seg-table dump remains in emit_gen_step_fn
  (prints kind + next_override per seg) — it found every planner bug above;
  keep it.
- probe harness leaf0304g_probe.rs retained (control-flow swap/swallow
  evidence); the scratch probeNN.rs files were removed.
- FN_FIRST_USER is now 74 (FN_DICT_DRAIN=73 appended dense).

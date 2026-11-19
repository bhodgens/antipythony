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

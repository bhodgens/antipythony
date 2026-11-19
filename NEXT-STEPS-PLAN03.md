# Next Steps — Plan 03 pause point (2026-11-18 late, trunk 7826a14 pushed)

## 03.04f LANDED + MERGED + PUSHED (2026-11-18)

- Leaf 03.04f verified first-hand (worktree 7826a14, clean; probe verdicts
  reproduced by the orchestrator), merged to main, pushed.
- LANDED: machine-wide try_table refactor of the step fn (g-probe class of
  silent-swallow bugs fixed), staged-GE entry check (close path byte-exact),
  gen_close GE-swallow wrap, YfNext SI-arm payload extraction from the
  CAUGHT exc + inline YfDone bind, GE005 listcomp/genexp generator flip
  (hidden <listcomp> is_generator=true + module slot + static gen-call
  emit), walrus (Expr::Named) emit arm. 25 bisect probes green
  (ge0304f_bisect.rs). GE001/GE002 still PASS byte-exact.
- Baselines held + re-verified on trunk post-merge: controlflow 18/18,
  classes/exceptions 17/17, for_probe 5/5, ap-host 8/8.
- HONEST GAPS (2+ attempts each, root-caused, NOT closed):
  - GE003: `closing` (closer's staged GE delivered by the machine catch)
    + `B end` (post-HandlerPrefix park-resume seg skipped).
  - GE004: `sub returned 42` + `empty gave 7` (post-YfDone seg skipped).
  - GE005 partial: hidden-gen step traps on first comp; list(gen) after
    next(gen) returns [] (resume-state preservation); FN_SUM_DRAIN not
    started; DictComp pairs driver not started.
  - MA001: match emit not started (blocked behind the same dispatch skip).
- ROOT CAUSE for the next session (NEXT-0304f.md lines 82-95, read FIRST):
  the goto_x br-depth formulas were tuned for the OLD per-segment $cx
  shape. The machine-wide try_table shape's depth table needs ONE
  principled recompute from the block nesting (at seg k: open blocks =
  B(k+1)..B(n-1) + $sink + $done + Loop1 + tt + $cx + $outer + wrappers).
  Empirical anchor verified: SI-arm trampoline br(11) for n=10/k=2/extra=2
  reaches Loop1. The post-handler "skip 1+ arms" symptom is the remaining
  discrepancy. Do NOT revert the machine-wide try_table (g-probes green).
  GOTCHA: dbg prints inside step fns CLOBBER mirror locals — use unique
  traps or module-level observables.

## 03.04g LANDED — LEAF 03.04 CLOSED (2026-11-19, trunk 4d541fc pushed)

- Leaf 03.04g verified first-hand (worktree 99d1923, clean): ge_ma_probe
  6/6 (GE001-GE005 + MA001 ALL PASS byte-exact), ge0304f_bisect 25/25,
  controlflow 18/18, classes/exceptions 17/17, for_probe 5/5, ap-host
  8/8 — all re-run by the orchestrator, then re-run green on trunk
  post-merge (4/4 suite groups + ap-host 8/8).
- Merged 99d1923, pushed 210be74..4d541fc (incl. PROGRESS.md record).
- LEAF 03.04 IS CLOSED. Depth tables were NOT the root cause (03.04f
  formulas verified correct); the 11 actual root causes are in the
  NEXT-0304f.md 03.04g appendix. Known unobservable divergence:
  guard-fail leaves captures bound. Match Value/Class/**rest remain
  fail-closed at scan; dict keys str-only in the DGET/DICT_HAS slice.
- REMAINING Plan 03: (1) bookkeeping — extend the CLI corpus gate to
  generators/match families per NEXT-0305.md steps (mechanical); 
  (2) leaf 03.06 (browser smoke + threading spike); (3) leaf 03.07
  (differential exit gate: full-corpus triple-diff report + Class-X
  burn-down). Then Plan 04 per master.md.

## Next dispatch: leaf 03.04-bookkeeping + 03.06 (can be one agent or two)

Bookkeeping (small): extend scripts/cli_corpus_gate.py to generators_wasm
+ match_wasm families per NEXT-0305.md's steps; wire into the gate list.
03.06 (per leaves/03.06-browser-smoke-threading.md): HTML demo embedding
ap-host compiled to wasm, running a corpus script; wasi-threads spike
(feasibility probe, document either way); decision recorded in the leaf.
03.07 last (needs the full corpus): differential exit gate.

## Standing conventions

- Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4; worktrees +
  logs under ~/.git/worktrees/, NEVER /tmp.
- Stall rule: transcript frozen >20 min -> steer once; second stall ->
  stop + salvage + re-dispatch citing the session-record design docs.
  Provider stalls of 5-15 min self-recover; do not steer under 20.
- Never re-derive: the NEXT-*.md session records ARE the designs.

## 03.04e LANDED + MERGED + PUSHED (2026-11-18)

- Leaf 03.04e verified first-hand by the orchestrator (worktree 8999f937,
  clean), merged to main (76d30b7b), pushed. THE SEGMENT MACHINE FLATTENER
  IS IN AND WORKING.
- GE001 PASS, GE002 PASS — byte-exact through ap-host vs CPython 3.14.7
  pins (verified by the orchestrator via ge_ma_probe in the worktree).
- Baselines re-verified on trunk post-merge: controlflow 18/18,
  classes/exceptions differential green (now 17/17 per the leaf),
  for_probe 5/5, ap-host 8/8.
- En-route REAL bug fixes (pre-existing, now fixed): gen_throw/gen_close
  table-index missing FN_FIRST_USER subtract (latent since 03.04b);
  step-fn locals pad short by 10 shifting the shared temp window (latent
  since 03.04d); c.send/close/throw method dispatch arms; gen class-call
  arm; ForNext/YfNext try_table stack discipline.
- ONE deliberate design divergence (soundness-argued, in NEXT-0304e.md):
  ONE dispatch-Loop TRAMPOLINE around the whole machine instead of
  per-loop Loop wraps (a br_table resume cannot reach bodies inside a
  per-loop Loop label). Verified label-depth formulas recorded.
- HONEST GAPS (leaf 03.04 exit contract NOT fully met): GE003 MISMATCH
  (handler-yield resume path escapes the try wrapper; minimal probe
  recipe in NEXT-0304e.md lines 96-100), GE004 HOST-ERR (first delegate
  resume trap; SI-payload args[0] extraction suspect, lines 101-112),
  GE005 NOT STARTED (comp hidden-fn generator flip + walrus Named arm +
  <listcomp> module-name registration), MA001 NOT STARTED (match emit
  arm; design ready in NEXT-0304d.md lines 118-124).
- CONTINUATION MANUAL: plans/antipythony/03-wasm-backend/NEXT-0304e.md
  (146 lines: segment kinds, label math formulas, 10 en-route fixes,
  per-gap bisect pointers, debug tooling list). Also NEXT-0304d.md
  (segment kinds + match emit design) and NEXT-0305.md (ap-host side).

## Next dispatch: leaf 03.04f (the LAST 03.04 session, expected)

Scope: (1) GE003 bisect per the minimal probe recipe; (2) GE004 SI-
payload extraction fix; (3) GE005 comp hidden-fn generator flip (yield-
elt, is_generator=true, emit_generator_call + FN_LIST_DRAIN/SUM_DRAIN,
DictComp dict-build, walrus Named arm binding ENCLOSING scope per
S-02-2, <listcomp> module-name registration); (4) MA001 match emit per
NEXT-0304d.md lines 118-124. Exit gate: GE001-GE005 + MA001 ALL
byte-exact through ap-host + baselines held (controlflow 18/18,
classes/exceptions 17/17, for_probe 5/5, ap-host 8/8). After 03.04f
closes: extend the CLI corpus gate to generators/match families per
NEXT-0305.md steps, update PROGRESS.md, leaf 03.04 CLOSES.
Then: 03.06 (browser smoke + threading spike) -> 03.07 (differential
exit gate) -> Plan 04 per master.md.

## 03.05 LANDED + MERGED + PUSHED (2026-11-17 late)

- Leaf 03.05 verified first-hand by the orchestrator, then merged to main
  and pushed: ap-host CLI (`ap-host run`, caps/env/args flags, --ttfb,
  --validate-only), versioned aprt ABI stamp + refusal tests, capability-
  scoped env/args (default deny), TTFB harness + scripts/, corpus dump +
  CLI corpus gate (35 pinned cases + 10 probes byte-exact through the
  BINARY, debug AND release). ap-host suite now 8/8 (was 3/3).
- Startup measured by orchestrator too: 38.5-40.4ms TTFB release (target
  <50ms PASS, ~10ms margin; debug ~750ms recorded honestly). Engine build
  + cranelift compile dominates -> future lever = engine cache / AoT.
- Session record: plans/antipythony/03-wasm-backend/NEXT-0305.md
  (7 gotchas + the exact steps to extend the CLI gate to generators/match
  families once 03.04 closes).
- Honest residuals: in-module WASI getter probes not wired (emitter side);
  aprt_version global re-uses G_RT_DONE slot (cosmetic; custom-section
  stamp is authoritative); bignum/format-as-host-imports deviation needs
  orchestrator sign-off ritual per design §4.

## 03.04e (the GE/MA finale) — IN FLIGHT

- Subagent sa-0-0aecd0e5, delegation deleg_17708165.
  Transcript: /Users/caimlas/.hermes/cache/delegation/live/deleg_17708165/task-0.log
  Worktree: ~/.git/worktrees/leaf-0304e (branch tier1-0304e).
- Executing the SEGMENT MACHINE flattener per NEXT-0304d.md (read that
  doc FIRST on resume; also NEXT-0305.md for the ap-host side).
- Exit gate: GE001-GE005 + MA001 byte-exact through ap-host + baselines
  (controlflow 18/18, classes/exceptions 16/16, for_probe 5/5, ap-host
  now 8/8) held.
- On completion: adversarially verify in its worktree (gate battery +
  independent throw-probe; empty-stdout+rc=0 = swallowed bug), merge
  tier1-0304e, re-run gates on trunk, push. Leaf 03.04 then CLOSES;
  extend the CLI corpus gate to generators/match per NEXT-0305.md steps.
- Then: 03.06 (browser smoke + threading spike) -> 03.07 (differential
  exit gate) -> Plan 04 per master.md.

## Standing conventions

- Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4; worktrees +
  logs under ~/.git/worktrees/, NEVER /tmp.
- Stall rule: transcript frozen >20 min -> steer once; second stall ->
  stop + salvage + re-dispatch citing the session-record design docs.
  Provider stalls of 5-15 min self-recover; do not steer under 20.
- Never re-derive: the NEXT-*.md session records ARE the designs.

## History (provenance)

- 03.04b (deleg_462c1174): drivers real, honest NOT-green, merged 6e638ad5.
- 03.04c (deleg_b8ee978c): For/iter wheel + scan gates + probe tooling.
- 03.04d (deleg_84c22842): drain/list type unification (eecf9a77),
  segment-machine design, merged 14307875.
- 03.05 (deleg_f1b0adac): ap-host CLI + versioned ABI + caps + TTFB,
  45/45 CLI gate, merged ca32b13d, verified + pushed.
- 03.04e (deleg_17708165): segment-machine flattener execution, running.

## Where things stand

- Trunk HEAD: 14307875 (merge of tier1-0304d). Adversarially verified by
  the orchestrator BEFORE merge (worktree a2333e19) and re-run green on
  trunk AFTER merge:
  - controlflow_wasm_gate 18/18
  - classes_exceptions_wasm_gate 16/16
  - for_probe 5/5
  - ap-host 3/3
- Landed by 03.04d (eecf9a77): drain/list type unification — $list.items
  rides MUT $obj_arr; 3 never-executed-path bugs fixed (drain SI-rethrow,
  ref.cast mut->RO validates-but-traps, MUT array refs need typed locals).
  Probes list/tuple/iter/sum all byte-exact rc=0.
- GE001-GE005 + MA001 still NOT byte-exact (honest state, verified per-case
  by the leaf): GE001 first next() works, second resume throws SI(42);
  GE002 yield-in-BinOp emit panic; GE003-005/MA001 blocked behind the
  flattener.
- THE CONTINUATION DESIGN (execute directly, do not re-derive):
  plans/antipythony/03-wasm-backend/NEXT-0304d.md (133 lines) — the
  SEGMENT MACHINE flattener (numbered segments, open dispatch Blocks,
  forward br goto, Loop wraps per loop range, segment kinds Seq/Test/
  Park/Rebind/SplitResume/ForInit/ForNext/YfNext/TryWrap/Return, planner
  br-depth arithmetic incl. +1 per wrapper), send/close/throw method arms,
  comp-fn-as-generator flip, match emit chain, GE005 walrus note.
  CRITICAL: the residual-tree flattener was attempted and REVERTED — do
  NOT retry it (loop-yields stay live across iterations; prefix-drop
  kills the construct).
- New lessons (NEXT-0304d.md): ref.cast between unrelated array types
  validates but traps (type identity = the array STRUCT); MUT array refs
  need dedicated typed locals (lesson 9 corollary, hit twice); drain-style
  try_table loops SI-match BEFORE rethrow. Identical-text validator trap
  = grep the local DECLS in the wat, not the instructions.

## On resume, in order

1. Dispatch leaf 03.04e (deleg → new subagent) with NEXT-0304d.md as the
   execution manual + the lessons above. Exit gate: GE001-GE005 + MA001
   byte-exact through ap-host + the four baselines held.
2. Worktree ~/.git/worktrees/leaf-0304e (branch tier1-0304e) off current
   main. Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4.
3. On completion: ADVERSARIALLY VERIFY in its worktree —
   - generators_wasm_gate + match_wasm_gate (if added) + all four baselines
   - independent throw-probe: each GE/MA case LOWERED + byte-exact
   - watch for empty-stdout+rc=0 (swallowed control-flow bug signature)
   then merge tier1-0304e, re-run gates on trunk, push, update PROGRESS.md.
4. Leaf 03.04 then CLOSES. Next: 03.05 (ap-host WASI + runtime imports;
   gate = FULL corpus green in wasm mode + startup <50ms measured),
   then 03.06 (browser smoke + threading spike), then 03.07 (differential
   exit gate), then Plan 04 per master.md.
5. Stall rule (proven across 3 sessions): transcript frozen >20 min →
   steer once (lean responses, commit per increment, finish honestly into
   a session record); second stall → stop + salvage + re-dispatch citing
   the session-record design docs. Provider stalls of 5-15 min self-recover;
   do not steer under 20.

## History (provenance)

- 03.04b (deleg_462c1174): drivers real, honest NOT-green, merged 6e638ad5.
- 03.04c (deleg_b8ee978c): For/iter wheel + scan gates + probe tooling,
  merged with tier1-0304d merge. One 84-min provider stall revived by steer.
- 03.04d (deleg_84c22842, 11h43m): increment A landed (drain/list type
  unification), flattener rewrite attempted + honestly reverted, segment
  machine designed, baselines held, merged 14307875.
- First 03.04b attempt (deleg_f9fbcd48): died at 395s provider stall in
  study; salvage created the context-packing dispatch convention.

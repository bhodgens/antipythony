# Next Steps — Plan 03 pause point (2026-11-17 ~23:30, trunk 51a93f62 + 03.05 merge)

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

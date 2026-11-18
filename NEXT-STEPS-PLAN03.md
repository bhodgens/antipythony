# Next Steps — Plan 03 / leaf 03.04e pause point (2026-11-17 ~18:20)

Supersedes the 03.04d version. Trunk pushed to origin at 14307875.

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

# Next Steps — Plan 03 / leaf 03.04d pause point (2026-11-17 ~06:30)

Supersedes the 03.04c version of this doc.

## Where things stand

- Trunk HEAD: merge of tier1-0304c (b99d6e8e + 5226f69c + 8fb4a7bb +
  ad37e875). Adversarially verified by the orchestrator BEFORE merge:
  controlflow_wasm_gate 18/18, classes_exceptions_wasm_gate 16/16,
  for_probe 5/5, ap-host 3/3 — all re-run green in the worktree, and
  re-run green on trunk after the merge.
- Landed cumulative: gen-driver prelude (03.04b), scan gates for
  generators/For/Match/comps, Stmt::For emit byte-exact (range/list/
  tuple, else/break), range/list/sum/next/iter builtins, drains FN 71/72,
  stop_iter bare-vs-valued fix, for_probe differential, debug tooling
  (dump_validate_locate / dump_wat_full / dump_wat_offset).
- CONTINUATION DESIGN (execute directly, no re-derivation):
  plans/antipythony/03-wasm-backend/NEXT-0304c.md (74 lines) — generator
  step-fn CFG flattener, send emit, yield-from decision rule, comp emit,
  match emit, new gates to add, baselines to hold, 6 hard-won gotchas
  (emit-order-is-index-contract; empty-stdout+rc=0 = swallowed bug).
- Leaf 03.04d (the EMIT arms finale) IN FLIGHT:
  subagent sa-0-ecde769f, delegation deleg_84c22842.
  Transcript: /Users/caimlas/.hermes/cache/delegation/live/deleg_84c22842/task-0.log
  Worktree: ~/.git/worktrees/leaf-0304d (branch tier1-0304d).
  EXIT GATE: GE001-GE005 + MA001 byte-exact through ap-host + baselines held.

## On resume, in order

1. Check sa-0-ecde769f (delegate_task action='list'). If finished:
   ADVERSARIALLY VERIFY in its worktree:
   - cargo test -p ap-codegen-wasm --test generators_wasm_gate
     --test match_wasm_gate --test controlflow_wasm_gate
     --test classes_exceptions_wasm_gate --test for_probe
   - cargo test -p ap-host
   - Independent throw-probe: each of GE001-005/MA001 must report LOWERED
     and byte-exact (NOT-LOWERED must be gone).
   Watch for empty-stdout+rc=0 (swallowed control-flow bug signature).
2. If gates verify: merge tier1-0304d to trunk, re-run all five gate
   suites + ap-host on TRUNK, update PROGRESS.md session record.
   Leaf 03.04 then closes → dispatch 03.05 (ap-host WASI + runtime
   imports; gate = FULL corpus green in wasm mode + startup <50ms
   measured).
3. If gates fail or agent died: verify uncommitted work, absorb or
   discard per NEXT-STEPS.md convention, re-dispatch citing
   NEXT-0304c.md + PROGRESS.md lessons (they carry the full design).
4. If still running: leave alone (one heavy cargo lane). Stall rule:
   transcript frozen >20 min → steer once (lean responses, commit per
   increment, finish honestly into a session record); second stall →
   stop + salvage + re-dispatch.
5. After 03.04 closes: 03.05 → 03.06 (browser smoke + threading spike)
   → 03.07 (differential exit gate) → Plan 04 packaging per master.md.
6. Machine policy binding: ONE heavy cargo lane; CARGO_BUILD_JOBS=4;
   worktrees + gate logs under ~/.git/worktrees/, NEVER /tmp.

## History (for provenance)

- 03.04b (deleg_462c1174): drivers real, honest NOT-green, merged 6e638ad5.
- 03.04c (deleg_b8ee978c): 28.5k s session, one 84-min provider stall
  revived by steer, one 420s probe timeout self-handled (timeout 300 wrap).
  Increments A/B/C + NEXT-0304c.md landed; GE/MA emit honestly NOT landed.
- First 03.04b attempt (deleg_f9fbcd48) died at 395s provider stall in
  study phase; salvage produced the context-packing convention that has
  worked for every dispatch since.

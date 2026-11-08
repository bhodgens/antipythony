# NEXT-STEPS 2026-11-07 (post issue-#13 closure)

## State
- Issue #13 CLOSED: codec engine 1056/1056 byte-exact vs CPython 3.14;
  probes 530/530 + e2e PROBE-GREEN + s037 green (commits a8841dc,
  0c14fb1, 958c903).
- Plan 03: 03.02a landed in worktree ~/.git/worktrees/leaf-0302 @1b02ae8
  (verified); split recorded; 03.02b next dispatch.

## Queued: XPASS ledger flip wave (01.02 owns the mechanism)
Full-suite XPASS = 93 > ceiling 74 (tests/XPASS_CEILING, set 2026-10-20).
[FRESH RUN 2026-11-07 post-commit: 1987 passed, 0 failed, 6 skipped,
670 xfailed, 93 xpassed — pytest gate GREEN, ceiling breach confirmed.]
Breakdown: 64 x tests/t11_stdlib (reason string: "awaits 02.08 v1 stdlib
subset" - 02.08 landed long ago, markers stale), 28 across
test_a1_rustpython_backfill (A-1 landed 2026-10-25) + others. This is
drift accumulated over the 10-25..11-07 landing waves; today's codec wave
did not create it (t11/t07/t08 xpasses were already 64+ in the pre-wave
run). Protocol: remove stale xfail markers so tests become unmarked
passes (corpus only grows; never weaken). Each flip needs a deterministic
pass confirmation (3 consecutive green runs) per the 01.02 mechanism.
After the flip wave the ceiling should drop to the true residual count.

## Queued: full workspace gate
`cargo test --workspace` NOT yet run on the post-#13 tree (one heavy
cargo lane per the machine lock policy). Run before any push decision.
Push: branch is ahead of origin; user decides.

## Plan 03 queue
1. Dispatch 03.02b (prelude runtime + value/arithmetic lowering) from
   worktree leaf-0302 @1b02ae8. Brief MUST cite: wasmtime-47 passive
   segment requirement; aprt.abort_msg anyref deviation (03.05 review).
2. 03.02c after 03.02b (control flow + calls + closures).
3. Merge 03.02a+worktree wave to trunk with 03.02b/c.

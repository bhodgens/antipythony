# ROBUSTNESS-CI - PROGRESS (session continuity record)

Campaign not yet dispatched. Read this file before dispatching any leaf;
write to it (merge, never overwrite) after each leaf commits.

## Dispatch log

- 2026-10-19 Wave A dispatched (deleg_64ca82e8): 00.01 (sa-0-844d32e8,
  ci.yml core jobs), 00.02 (sa-1-5258ef0a, reference discovery +
  AP_REQUIRE_REFERENCE), 02.02 (sa-2-712a5f6e, importer segment guard),
  03.03 (sa-3-cfb8af51, baselines + AGENTS.md hygiene + hooks note).
  Sibling Plan 02 threading wave is LIVE in the same worktree (dirty:
  interp.rs, builtins*.rs, classes.rs, methods.rs, exc_hier.rs, gcsupport.rs,
  lib.rs, new threading.rs/threading_call.rs). Ownership lines enforced in
  every brief: leaf agents confined to .github/, tests/, importer.rs,
  AGENTS.md/docs only. importer.rs verified NOT in the sibling dirty set.

## Open decisions

1. setup-python@v5 "3.14" runner availability unverified (master.md
   Deferred) - 00.01 step 0 resolves; record the answer here.
2. ci.yml clippy -D warnings timing: 00.01 ships informational, 02.01 flips
   strict. Confirm no drift between them.
3. Branch 06 is hard-gated on Plan 02 (antipythany tree) completion of
   02.08-full p2 commit + review; check plans/antipythany/02-native-backend/
   PROGRESS.md before ANY dispatch here.
4. Today's real pass counts (rust/pytest/XPASS ceiling baseline, corpus
   slice rates) were never re-run by the review agent - first leaf needing a
   baseline re-measures and records it here.

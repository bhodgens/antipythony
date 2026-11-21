# NEXT-T121C — session record (leaf T121c, 2026-11-20)

Leaf: T121c — re-baseline the T121 scaling gate (user decision C:
re-baseline BEFORE fix-vs-accept). Branch tier1-t121c off main 213e2ac,
worktree ~/.git/worktrees/leaf-t121c. MEASUREMENT-ONLY (no interpreter code
changed; harness script + corpus env-var sizing + docs only).

## Commits
- a3c67bf: harness scripts/t121_rebaseline.py + corpus T10_REBASELINE_N
  sizing knob (default 60000 = pin-identical).
- a91dd9c: measured numbers (3 raw JSONs) + --profile flag + release-gate
  print fix.
- (this commit): T121-REBASELINE.md verdict + this session record.

## Verdict (full detail in T121-REBASELINE.md)
- RELEASE profile: sized leg (N=750000, serial 1.397s) ratio **1.05x**;
  old leg (N=60000, serial 0.113s) 1.10x. The >=3x gate is NOT achievable
  on release at any workload size — workload-size hypothesis CLOSED.
- DEBUG profile: sized leg 3.05x, old leg 3.06x — the debug 3x is real but
  measures unoptimized compute, not scaling; NOT a valid gate profile.
- Gate recommendation: re-baseline to >=1.0x on RELEASE+sized leg (measured
  1.05x); keep debug as print-only. Lock-free line-bookkeeping follow-up is
  CONFIRMED with measured payoff: ~75% of the parallel leg is overhead,
  expected ~3.9x post-fix (ceiling 4.19x on 4 cores) — >=3x release becomes
  reachable only after that leaf.

## Gates (verified this session)
- cargo test -p ap-run --release: 196 passed / 0 failed / 1 ignored, rc=0
  (CARGO_TARGET_DIR=$PWD/target-0304c; see GOTCHA below).
- pytest tests/t10_threading (APX_IMPL=release): 97 passed / 10 skipped /
  1 failed = the >=3x scaling gate itself (1.05x) — the leaf's subject.
- Corpus T121 pins byte-identical (default mode).

## GOTCHA (new, environment not code)
In a FRESH worktree, bare `cargo test -p ap-run --release` FAILS
fix_pass_differential with "spawn fenced cmd: No such file or directory":
the fenced suites resolve `<target-dir>/debug/ap-run-eval`, but
.cargo/config.toml's build.target-dir (target-0304c) is NOT in the test
binary's env, so the test's CARGO_TARGET_DIR branch misses and the
env!("CARGO_MANIFEST_DIR")/../../target/debug fallback doesn't exist.
Run gates as: CARGO_TARGET_DIR=$PWD/target-0304c cargo test -p ap-run
--release. (First seen here; fix belongs to a repo-tooling leaf if it
annoys CI.)

## Follow-ups filed (for orchestrator)
1. Harness-leaf: point the pytest gate at release + sized leg, assert >=1.0x,
   print debug (T121-REBASELINE.md §Recommendation 1).
2. Lock-free per-frame line bookkeeping leaf (bisect follow-up 1) — now with
   measured payoff ~3.9x expected; needs traceback-lineno-semantics gate.
3. 2/8-core thread-count sensitivity (deferred; needs a second corpus case
   with variable NSH — pins currently fix NSH=4).

## Session honesty notes
- Quiet-window waiting dominated the session (~6h wall): external go-test
  lanes + mediaanalysisd spiked load to 10-38 repeatedly; the harness's
  per-round load guard aborted 8 partial runs (by design). Release leg ran
  at loads 1.3-2.3; debug leg at 1.6-2.2 (one round accepted at 2.2 under
  the relaxed 2.5 threshold).
- One release parallel sample (1.644s) is a visible outlier vs its
  1.335-1.376 siblings; kept in the raw JSON (min-based best-of unaffected).

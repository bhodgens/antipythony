# Branch 01 - Test Integrity

Goal: make green mean green. Today 126 stale XPASS markers under
strict=False report success even if the behavior regresses (finding #5),
ap-gc and xtask have zero in-crate tests (finding #10), and
bench_report.json churns as a fake gate (finding #12).

## Leaves

### 01.01 - XPASS ceiling gate
- Add `tests/XPASS_CEILING` (one integer: today's xpass count from a real
  `pytest tests/ -q` run - re-measure, do not trust the quoted 126).
- Add `tests/test_xpass_ceiling.py`: parses pytest output (or a JSON report
  run), fails when XPASS count exceeds the ceiling.
- Wire into ci.yml fast-lints job (needs built bins? no - ceiling lint runs
  from a `--collect-only`-style pass or reuse the conformance job's summary;
  pick the cheaper: a small script invoked post-suite).
Gate: ceiling file committed with measured number; lowering the file with
fewer xpasses passes; adding a stale xfail that XPASSes above ceiling fails.

### 01.02 - STALE-XFAIL-LEDGER burn-down (ongoing protocol, not one agent)
- Follow `plans/antipythany/02-native-backend/STALE-XFAIL-LEDGER.md`: each
  owning antipythany leaf (01.02/01.03/01.04/02.05) flips its tests to
  `strict=True` when it lands.
- FIRST concrete flip: T-15 "CI runner config existence"
  (`tests/t15_release/test_t15_release.py:44-58`) - after 00/01 lands,
  un-xfail it to strict real-run (finding #20 closes here).
- Record each flip in the ledger with date + commit; XPASS ceiling (01/01)
  mechanically proves the count only shrinks.
Gate: ledger entries with commits; ceiling count strictly lower than
01/01's baseline after each flip wave.

### 01.03 - ap-gc unit tests
- New `crates/ap-gc/tests/gc_invariants.rs` + `#[cfg(test)]` units in
  `gcmod.rs`: cycle collection, allocation accounting, weakref bridge
  ordering (`crates/ap-gc/src/weakbridge.rs` global-state design is the
  stated risk), contention-abort behavior (collector must ABORT mid-mutation,
  never skip - project rule).
- Author tests from `docs/spec/` GC clauses + `docs/gc-mmtk-swap.md`
  invariants where applicable.
Gate: `cargo test -p ap-gc` green with >0 tests previously (crate had none);
deliberate break of abort-on-contention turns a test red.

### 01.04 - xtask tests
- `crates/xtask/` gets `#[cfg(test)]`: notices determinism (header at
  `crates/xtask/src/main.rs:26` CLAIMS sorted deterministic output - assert
  byte-identity across two runs), --check exit codes (0 clean, 1 stale)
  against a fixture tree (tempdir copy of Cargo.lock + manifest set) - do
  NOT mutate repo notices in tests.
Gate: `cargo test -p xtask` green.

### 01.05 - Bench trend-only policy
- `tests/t08_tier1/bench_report.json`: confirm gitignored (AGENTS.md says it
  was the churn culprit); if still tracked, `git rm --cached` + .gitignore.
- Document in tests/README.md: bench numbers are trend artifacts; never a
  PR gate. When 02.05 lands, the >=1.5x gate runs only on fixed hardware
  (self-hosted/ci-tagged), ratio-only.
Gate: bench_report.json no longer appears in `git log --name-only` diffs for
post-leaf commits; README rule present.

## Dispatch notes

01.03/01.04 parallel-safe (different crates). 01.01 needs 00/01's
fast-lints job to host its gate but the file/pytest work proceeds first.
01.02 is a protocol leaf: dispatch per flip, not once.

## Test Gate

<!-- Tests-first gate (hierarchical-planning skill v1.3.0, patched 2026-10-24).
     Coverage matrix derived mechanically from leaf task headers; the control
     agent (orchestrator, in-session) refines rows and fills test names at
     Phase 0 dispatch. No implementation leaf dispatches until every row is
     VALIDATED. Delta protocol: lesser of plan/tests updates to the greater.
     Recipe: skill hierarchical-planning -> references/tests-first-gate.md -->

| # | Feature (from plan) | Source | Unit tests | E2E tests | Status |
|---|---------------------|--------|-----------|-----------|--------|
| F1 | Branch 01 - PROGRESS | PROGRESS.md | (pending) | (pending) | PENDING |

Statuses: PENDING | DELTA | VALIDATED

Delta log: (none yet)

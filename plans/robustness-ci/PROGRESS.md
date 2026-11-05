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
- 2026-10-19 Wave A REVIEWED + COMMITTED by orchestrator: b94976c (plan tree
  + audit), 717bc9c (00.01 ci.yml), 7c8358d (00.02 ref discovery), 6dae48c
  (02.02 guard), 6b67db0 (03.03 hygiene); README/docs in separate commit.
  Fresh orchestrator probes on 02.02: 6 invalid/valid import shapes byte-
  exact vs live 3.14. Full pytest re-run launched (proc_4d7a28999525,
  /tmp/ap_pytest_waveA.txt) - 01.01 ceiling baseline awaits its tally.
- 2026-10-19 Wave B dispatched (deleg_311e77fe): 00.03 (sa-0, conformance
  matrix job), 02.01 (sa-1, workspace.lints + grandfather allowances),
  01.03 (sa-2, ap-gc in-crate tests), 01.04 (sa-3, xtask tests), 03.01
  (sa-4, golden .meta provenance + backfill).
- 2026-10-20 Wave B REVIEWED + COMMITTED: 4c8d08b (02.01 lints), 3635a6f
  (01.03 ap-gc 23 tests), 1c16009 (01.04 xtask 8 tests), 8db313c (03.01
  232 metas + 475-check lint), a881906 (00.03 conformance matrix). All
  verified by orchestrator: PyYAML parse ci.yml, cargo test -p ap-gc -p
  xtask re-run green, corpus_traceability 475/0 re-run green, importer
  guard 6-shape fresh probes byte-exact. SIBLING NOTE: 02.01's commit
  briefly absorbed threading sibling's pub mod lines into ap-run/src/
  lib.rs; stripped via hash-object staging (git show HEAD verified 0
  threading refs; sibling's worktree diff intact).
- 2026-10-20 Wave C dispatched (deleg_7982682d): 05.01 (sa-0, clap
  migration 4 bins), 01.01 (sa-1, XPASS ceiling + pytest-side reference
  discovery), 04.01 (sa-2, proptest re fuzz), 04.02 (sa-3, proptest
  repr fuzz), wave-A-gap-followup (sa-4, ap-cli/ap-obj differential seam
  + ap-conform env step-0). Conflict lines: sa-4 excluded from
  ap-cli/src/bin/* + all Cargo.toml edits; 04.x must not fix sre.rs/
  float_repr.rs (routed findings only); nobody touches .github/.
- HELD queue: 04.03 nightly workflow (after 04.01/02 land), 05.02
  ap-parse-check (awaiting sibling Plan-01-owned bin decision? no - own
  it Wave D), 05.03-05.05, 03.02 stdlib-provenance (Wave D), 02.01 CI
  -D warnings flip + attribution.yml retirement (after FIRST REMOTE
  GREEN of ci.yml - requires user push), 01.05 bench untrack (sibling
  staged bench_report.json), 01.02 flips ongoing, 06/* gated on Plan 02.
- Push/remote: repo has origin/main; user has not asked to push - the
  remote-green gate for ci.yml awaits a user push decision. HELD for conflict/dependency: 03.02 (xtask main.rs vs 01.04),
  05.01 (Cargo.toml vs 02.01), 01.01 (needs pytest tally), 01.05
  (bench_report.json staged by sibling Plan 02 agent - untracking would
  collide), 04/* (await Wave A/B settle + re engine stable).

- 2026-10-20 Wave C REVIEWED + COMMITTED: 1bd757d (05.01 clap), 0801ba5
  (00.02 followup), 0137c64 (01.01 ceiling 74 + pytest discovery),
  5de2caa (04.01+04.02 fuzz - FOUND 9 engine bugs, routed to Plan 02),
  12fabd1 (shim removal + ceiling wiring by orchestrator).
- 2026-10-20 Wave D dispatched (deleg_ce51264f) + REVIEWED + COMMITTED:
  846f377 + 22e73e8 (05.02 seam bin + T-01 flip: 113 real runs / 25
  known-fail; F-3 fixture bug fixed by orchestrator), 5b39f29 (03.02
  stdlib-provenance + 05.04 cli-docs; 3 UNDECLARED vendored deviations
  discovered), 6c899b1 (05.03 rustdoc gate), e899c9e (04.03 nightly
  fuzz), a2da3a4 (orchestrator CI wiring: provenance + cargo-doc +
  cli-docs gates; fast-lints python pinned 3.14.7).
  Orchestrator notes: (1) first flip attempt used xfail(strict=True) on
  passing tests - wrong direction (strict-xfail-on-green = XPASS fail);
  corrected to marker-removal, matching ledger d79508d. (2) clippy -D
  warnings measured 49 errors worktree-wide - strict flip stays a
  burn-down item, NOT flipped. (3) 05.02 F-1/F-2 = Plan 02 product
  surface (verify-stage placement rejection, ruff-vs-CPython column
  drift, two suites' mutual py2_print pin disagreement) - relay.

## Open decisions

1. setup-python@v5 "3.14" runner availability unverified (master.md
   Deferred) - 00.01 step 0 resolves; record the answer here.
2. ci.yml clippy -D warnings timing: 00.01 ships informational, 02.01 flips
   strict. Confirm no drift between them.
3. Branch 06 is hard-gated on Plan 02 (antipythony tree) completion of
   02.08-full p2 commit + review; check plans/antipythony/02-native-backend/
   PROGRESS.md before ANY dispatch here.
4. Today's real pass counts (rust/pytest/XPASS ceiling baseline, corpus
   slice rates) were never re-run by the review agent - first leaf needing a
   baseline re-measures and records it here.

## wave-A-gap followup (2026-10-20)
00.02's two deferred gaps closed: ap-cli `stdlib_modules_differential.rs` +
ap-obj `cpython_differential.rs` converted to the shared `tests/common`
reference-discovery seam (copies of ap-run's helper; probes byte-unchanged),
and `AP_CONFORM_REFERENCE_PYTHON` added as version-checked step 0 of
ap-conform's `find_reference_interpreter` (python3 fallback kept per brief).
All fail/skip/env-ladder gates green (details: 00-ci-core/PROGRESS.md
"wave-A-gap followup"). Rust differential suites are now uniformly on the
seam; pytest-track Homebrew-hardcode env conversion remains 01-branch work.

## HELD state - orchestrator decision 2026-11-05 (reconciliation; supersedes 2026-10-20 hold)

- T1/T2 campaign state moved a lot since 2026-10-20: T1 crawl reaches
  PARSE + HARNESS-DONE RC=0 (a7ff45b), T2 default-config gate re-green
  after the AST-campaign regression (d91eaf8, verified independently by
  orchestrator worktree build). Items live in the repo; the ~187-file
  vendoring wave HAS been declared over time (check now reports 10
  problems, not 187).
- Held-queue item 2 (02.09 sub-leaf p1 object-state audit): DISPATCHED
  2026-11-05 (deleg_e8b74b75, sa-0-583c502c) as a read-only worktree
  audit leaf -> docs/0209-object-state-audit.md + one pathspec commit.
- Held-queue item 3 (02.10 research note): LANDED earlier
  (docs/02-10-scheduler-design.md, 2026-11-03).
- Held-queue item 1 (stdlib-provenance --fix): RE-MEASURED 2026-11-05
  at fa2912a + live sibling WIP: FAIL (498 files, 10 problems) — stale
  declarations for vendored_stdlib/codecs.py + encodings/__init__.py +
  encodings/aliases.py + _ast.py (the T1 items leaf and the W114 AST
  sibling are editing those VERY files right now, uncommitted).
  BLOCKED-ON: T1-items leaf + W114 sibling commits. Do NOT re-declare
  before they land (the hashes churn under you; declaration goes stale
  by construction). Once both commit: `cargo run -p xtask --
  stdlib-provenance --fix`, hand-declare the drifted files
  (deviated_sha256 refresh), `--check` to OK, commit in one pathspec
  pass. This is the orchestrator's own task, sequenced into the
  post-wave integration pass.
- Items 4 unchanged (push-gated CI green — still awaits user push
  decision; attribution retirement; clippy -D burn-down 49 errors).
- Branch 06 (decomposition): still gated on Plan 02 completion per
  open-decision 3; Plan 02 has 02.05/02.06/02.08/02.09 open.
- Wave E not dispatched (06/* gate); no other robustness-ci leaves
  dispatchable this session (A-D committed 2026-10-20, queue above).

## HELD state - orchestrator decision 2026-10-20 (user directive)

- WAITING for the Plan 02 / T1 scrapy agent (other session) to FINISH
  before dispatching anything further. No subagent dispatches pending on
  this campaign's side.
- Held queue (in order once T1 lands):
  1. `xtask stdlib-provenance --fix` + declare any real deviations: the T1
     wave dropped ~187 new vendored_stdlib files (datetime/decimal/email/
     http/logging/charset_normalizer/parsel/...) with no DEVIATIONS.toml
     entries; `--check` currently FAILS (237 files, 187 problems) and the
     fast-lints CI job goes red on their first commit+push. Measured
     2026-10-20 against their working tree.
  2. 02.09 sub-leaf p1: object-state audit (findings-only, quarantined;
     RefCell census 300+ sites across 17 files as of cb84991-era tree,
     actively moved by 858f6bc threading-race fix - re-measure on land).
  3. 02.10 scheduler research note (no ownership overlap; can also go in
     the same wave).
  4. Post-T1 CI items unchanged: push-gated remote green, attribution.yml
     retirement, clippy -D warnings flip (49 errors tracked).
- Evidence T1 burned my routed findings already: ae115be/97f8ef9 fix
  RF-1/RF-4/RF-6/F0402-1; gc fence green against 858f6bc race fix.

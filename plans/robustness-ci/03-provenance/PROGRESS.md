# Branch 03 - Provenance: progress log

## 03.03 IMPLEMENTED NOT COMMITTED (leaf 03.03, 2026-10-19 → 2026-10-20 00:20 MDT, at HEAD 5f1e132)

### Fresh baselines measured (not copied from the audit)

| Command | Result | Notes |
|---|---|---|
| `git log --oneline \| wc -l` | **94** commits | audit said "80" in AGENTS.md (stale), "91+" since Oct 15 |
| `cargo test --workspace` | **289 passed / 0 failed / 1 ignored** (44 result lines) | exit 0; 162 compiler warnings present (e.g. unused_parens in ap-run/src/sre.rs) |
| `pytest tests/ -q` | full run **in flight at leaf end** (~50% after ~60 min; 2154 tests collected; so far 0 F / 0 E, 3 s skipped, 280 xfail, 0 xpass) | differential suites spawn `target/debug/antipythany run` with 60 s timeouts per corpus case - slow by design. Run handed off to the parent (proc_97ef8710f9f5, writes /tmp/ap_pytest.txt, appends PYTEST_EXIT=). Last committed known-good: 1272/0 (see plans/antipythany/02-native-backend/PROGRESS.md). |
| `cargo run -p xtask -- notices --check` | **exit 0** - "OK (151 third-party dependencies, up to date)" | |
| `git config --get core.hooksPath` | `/Users/caimlas/.githooks` | finding #15 confirmed: global hooksPath (LFS only: commit-msg, post-checkout, post-commit, post-merge, pre-push) silently disables repo-local hooks |
| corpus inventory | 8 families, **270** .py cases, **696** expected files (148 .out) | classes 33, exceptions 15, fstrings 71, gc 17, stdlib 48, threading 18, tier1-transparency 19, values 47 (+2 loose root cases) |

### Sibling-in-flight caveat
Workspace is mid-02.07-threading: uncommitted changes across
crates/ap-run/src (new threading.rs, threading_call.rs) + 10 modified src
files. The cargo gate measured GREEN at that state (289/0), so those
numbers are valid for "HEAD + sibling src". The pytest run was still
running when the leaf finalized; the AGENTS.md block records the last-good
committed number plus this caveat rather than a guessed final line.

### Edits made (only files this leaf owns)
- **AGENTS.md**:
  (a) ROBUSTNESS-CI line: "PLANNED, nothing dispatched" -> "WAVE A
      DISPATCHED 2026-10-19 (00.01, 00.02, 02.02, 03.03)".
  (b) Replaced stale counts line ("80 commits. rust 247/0, pytest 1272/0.
      Corpus 150/197") with a "Status block verified at 5f1e132 2026-10-19
      by leaf 03.03" block carrying the measured numbers above + the
      sibling/pytest-in-flight caveat + pointer to plans/**/PROGRESS.md as
      pass-rate source of truth.
  (c) One convention bullet under Execution protocol: repo gates run in
      CI, not git hooks (global core.hooksPath=~/.githooks makes repo-local
      hooks no-ops; if ever wanted, `git config core.hooksPath .githooks`
      in onboarding), pointing at docs/development.md.
- **docs/development.md**: new "### Why CI gates, not git hooks"
  subsection in the Gates section with the same facts + finding #15
  attribution. (Pre-existing line "Current baseline: rust 289/0" already
  matched my fresh measurement - left untouched.)

### Honest gaps
- Full pytest final summary line not captured in-session (run exceeds the
  leaf's wall-clock; 0 failures through ~50%). Whoever commits this wave
  should append the final `N passed` line to the table above once
  /tmp/ap_pytest.txt finishes, or re-run `pytest tests/ -q` once.
- Corpus pass-RATE (the old "150/197") was not re-measured - running the
  differential ap-conform scorer separately duplicates the hour-long pytest
  run; per the leaf brief, counts now point at PROGRESS.md as source
  (latest recorded there: 158/215 at 380f28e+dc16788, threading 0/18
  in flight).
- master.md coverage-table row 03.03 status not flipped (orchestrator
  owns master.md; leaf brief restricts me to AGENTS.md + docs/development.md
  + this PROGRESS.md).
- Nothing committed (per dispatch: sibling work is intermixed in
  crates/ap-run/src; orchestrator must commit AGENTS.md +
  docs/development.md + plans/robustness-ci/ separately from sibling src).

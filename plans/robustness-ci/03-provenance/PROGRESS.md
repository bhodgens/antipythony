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

---

## 03.01 IMPLEMENTED NOT COMMITTED (leaf 03.01, 2026-10-20)

### What landed
- **tests/tools/_gen_meta.py** (new): meta-only provenance recorder — the
  preferred "write ONLY .meta, never touch pins" design. Scans every
  `tests/corpus/**/expected/` dir, writes one `expected/<case>.meta` per
  case stem (ONE meta covering the whole .out/.err/.rc or
  .stdout/.stderr/.exit triple — per-case single-meta choice, simpler than
  per-file). Captures `sys_version` by actually running the reference
  binary, `date`, `generator_sha256`, `family`, `generator`. gc-family
  metas attribute to the canonical pin generator (tests/t07_gc/_gen_expected.py)
  via FAMILY_GENERATORS; everything else attributes to the recorder, with
  an in-doc honesty note that the original recording was ad hoc on 3.14.7
  while C-01 pins 3.14.0 (debt now mechanically visible: every meta says
  3.14.7).
- **tests/t07_gc/_gen_expected.py** (updated): the one corpus pin generator
  in tests/ now writes `<stem>.meta` alongside each pin triple (same
  schema; hashes itself). Other generator-shaped scripts checked and NOT
  pin generators for corpus expected/:
  t04 gen_expectations.py (writes fixtures/expected_t04.json which ALREADY
  embeds python_version+executable+generator path; test_fixture_reference_is_pinned_3147
  already lints it), fstrings tools/gen_cases.py (only writes case .py
  sources), t09/t16 suites write no expected files.
- **tests/corpus_traceability/test_golden_provenance.py** (new lint,
  PLACED INSIDE tests/corpus_traceability/ so the existing fast-lints job
  (`pytest tests/corpus_traceability`, ci.yml:52) picks it up with ZERO
  ci.yml edits — orchestrator: no include-glob change needed). Checks:
  every pin triple has sibling .meta; valid JSON with exactly the 5
  contract keys; sys_version startswith "3.14"; date/sha formats;
  generator_sha256 == live sha of the recorded generator path (editing a
  generator without re-recording metas = CI red); no orphan metas; coverage
  floor >=200 cases; PLUS a byte-identity SAMPLE test (1 case per family,
  reference re-run vs pinned bytes, family runner convention incl.
  _mask_root) proving backfill moved no pins.

### Backfill + byte-identity proof
- Snapshot first: all expected/ dirs copied to /tmp/leaf0301_snap_1792483428
  (696 pin files). Ran `python3.14 tests/tools/_gen_meta.py` (meta-only,
  pins never opened). Re-hash: 696/696 sha256 lines identical before/after
  ("BYTE-IDENTITY PROOF: 696 pin files, zero changes");
  `git diff tests/corpus` shows ZERO tracked changes; per-dir
  `diff -r snapshot vs live -x '*.meta'` clean. **pin_changes_detected=false.**
- 232 .meta files written (228 corpus cases + 4 tier1 bench pins:
  probes 15 + bench 4 + classes 33 + exceptions 15 + fstrings 35 + gc 17 +
  stdlib 48 + threading 18 + values 47).

### Gates executed (all /opt/homebrew/bin/python3.14)
- `pytest tests/corpus_traceability -q` → **475 passed** (1 pre-existing +
  474 new provenance parametrized checks incl. 8 byte-identity samples).
- Delete-one-meta → 2 failures (sibling-meta + coverage floor); restored →
  green. Negative gate proven.
- Edit-generator (append drift line to _gen_meta.py) → 215 failures
  (generator-hash freshness); restored → green.
- `pytest tests/t07_gc -k 'not ap_conform'` → 113 passed 2 xfailed (updated
  gc generator compiles + suite unaffected).
- t05/t06/t10/t11/t08 reference-self-check tests unaffected (they never
  enumerate expected/ dir contents; verified t05 98 passed, fstrings 105
  passed, t09 14 passed on fixture-side tests).
- Nothing in crates/ reads expected/ dirs (grep: no read_dir), .meta files
  cannot disturb Rust gates.

### Honest gaps
- Coverage math: 232 metas cover 232 case-triples = **696 pin files**
  under tests/corpus (classes 33, exceptions 15, fstrings 35, gc 17,
  stdlib 48, threading 18, tier1 probes 15 + bench 4, values 47 = 232
  cases; the 696 inventory matches the dispatch context exactly).
  t09_binary `cases/expected/` (39 files) and the two loose root corpus
  .py (no pins) are OUTSIDE tests/corpus, so they get no metas —
  per the master.md contract line (`tests/corpus/<fam>/expected/<case>.meta`).
  Flag for orchestrator if t09-inline pins should be pulled into scope later.
- Generators for the 7 non-gc families never existed on disk (pins were
  hand-recorded per test-track leaves), so "update every generator"
  resolved to: gc generator updated + the new meta-only recorder used for
  backfill (the task's own preferred design). If future slices want
  full pin-regen capability per family, that's new generator authoring,
  not this leaf.
- Nothing committed (dispatch rule). Lint placement recorded above so the
  orchestrator can confirm ci.yml fast-lints covers it without edits.

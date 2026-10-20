# Branch 03 - Provenance

Goal: machine-recorded truth for goldens, vendored files, and repo state.
Findings #11 (735 pins recorded on 3.14.7 vs contract's 3.14.0, no record of
which), #13 (vendored_stdlib drift invisible), #14 (AGENTS.md stale),
#15 (global core.hooksPath kills repo hooks).

## Leaves

### 03.01 - Golden .meta provenance
- Every `gen_expectations.py` (find them: `grep -rl gen_expectations tests/`
  + `tests/t07_gc/_gen_expected.py`) writes, per regenerated case,
  `expected/<case>.meta`: {sys.version, date, generator script sha256}.
- New lint in `tests/corpus_traceability/` (or its own
  `tests/test_golden_provenance.py`): every `expected/*.out|.err|.rc` must
  have a sibling `.meta`; missing meta = CI fail.
- Backfill: regenerate ALL families once with the updated generators
  (pins must reproduce byte-identical on the same interpreter - a changed
  pin during backfill is a FINDING, stop and report it). Record the
  interpreter actually used (3.14.7 today; the 3.14.0 re-verification debt
  stays open per compat-contract C-01, but now mechanically visible via
  meta.version fields).
Gate: lint green after backfill; deleting one .meta turns CI red; zero pin
bytes changed by backfill (git diff proves it).

### 03.02 - xtask stdlib-provenance
- `cargo run -p xtask -- stdlib-provenance [--check]`:
  1. Read `vendored_stdlib/DEVIATIONS.toml` (NEW file, authored from the
     documented deviations in commit 1cb6250 + LICENSES/README-vendored-stdlib.md):
     per-file {upstream path, upstream sha256 OR "deviation" + note}.
  2. Compare each vendored file against the local CPython Cellar Lib
     (path per AGENTS.md skill note: resolve via
     `python3.14 -c "import m; print(m.__file__)"`).
  3. --check exits 1 on UNDECLARED drift (changed vs declared-deviation
     list). Undeclared = accidental = the thing this gate exists to catch.
- Wire into ci.yml deny job or its own step.
Gate: --check green today against the 71 tracked files; touching a vendored
file without a DEVIATIONS entry fails.

### 03.03 - AGENTS.md hygiene + hooks note
- AGENTS.md status block: add "verified at <sha> <date>" line per the
  review's recommendation; counts stay prose but point at PROGRESS.md as
  source. Update the block NOW to reality (re engine landed 7a144b9,
  91+ commits) - re-run `cargo test --workspace` and `pytest tests/ -q` to
  quote fresh numbers, do not copy stale ones.
- Add one convention line: repo uses CI gates, not git hooks (global
  core.hooksPath=~/.githooks silently disables repo-local hooks on this
  machine, finding #15); if hooks ever wanted, onboarding must set
  `git config core.hooksPath .githooks`.
- Add a `docs/development.md` pointer to that section.
Gate: AGENTS.md block matches live runs; no file other than AGENTS.md and
docs/development.md touched.

## Dispatch notes

03.01 is the big one (735 files backfill). 03.02 depends on nothing. 03.03
should run LAST in this branch so its re-measured counts include 03.01/02.

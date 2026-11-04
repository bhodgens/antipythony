# Rename campaign: antipythany -> antipythony

One-shot cross-cutting rename. Not a plan tree with dispatchable leaves: it is
ordered steps with commit boundaries and gates, executable in one session
(~1-2 h agent time) by the orchestrator. Estimated 2026-11-02.

## Inventory (measured 2026-11-02 at f0e8c93)

| Cluster | Files | Notes |
| --- | --- | --- |
| vendored_stdlib/ | 98 | "antipythony deviation shim" headers; runtime NotImplementedError texts |
| tests/ | 51 | binary-name invocations (t09_binary x8), env vars, doc strings |
| plans/ | 42 | prose; plus the DIR `plans/antipythony/` itself |
| docs/ | 28 | prose |
| crates/ | 25 | ap-cli bin name `antipythony` + main.rs usage text (34 hits); ap-pack env prefix ANTIPTYHONY_* |
| root | 8 | README, AGENTS.md, deny.toml, rust-toolchain.toml, NEXT-STEPS.md, bughunt json, apx_t11_part_importlib_metadata |
| LICENSES/ | 1 | CARVE-OUT (see decisions) |
| GitHub repo | - | origin = git@github.com:bhodgens/antipythony.git; .github/workflows/ci.yml has 1 hit |

Case variants: 18 files carry ANTIPYTHONY/Antipythony; 15 carry the
ANTIPTYHONY env prefix (ANTIPTYHONY_UV, plus ANTIPTYHONY refs in ap-run
importer/builtins modules - 6 in importer.rs alone).

Total distinct spellings to sweep: antipythony (lc), Antipythony,
ANTIPYTHONY, ANTIPTYHONY.

## Naming record (owner confirmation, 2026-11-02)

- Target name `antipythony` is UNVERIFIED-BY-OWNER / now confirmed:
  "currently an unused name, zero collisions." A `"antipythony"` web search
  (run 2026-11-02) returned no exact-string project; the only near-hits are
  `antipathy` (the English word - the INTENDED soundalike) and `Antiphony`
  (different spelling, unrelated consulting firm). Collision check: CLOSED,
  no further verification needed before step 6.
- Pronunciation: `antipythony` is pronounced like "antipathy"
  (/ænˈtɪpəθi/). This is an intentional homophone, not a typo of
  "antipathy." The README must state this (see Step 1 README addition).

## Decisions (owner-confirmed)

- D1 Scope: in-tree code + docs + GitHub repo + local dir rename.
  Recommended: ALL FOUR. Recommended deferral: .hermes/ session artifacts and
  git history (never rewritten).
- D2 Env var prefix: hard-rename ANTPYTHANY_UV -> ANTIPTYHONY_UV (full spelling;
  the ANTPYTHON short form is rejected - it sits inside the PYTHON* namespace
  and collides with tooling env vars). No fallback alias: no shipped binary
  exists yet, zero external users.
- D3 Runtime strings vs pinned goldens: the NotImplementedError texts
  ("antipythony cryptography shim: ...") only execute on paths where CPython
  would succeed, so any corpus case pinning them already records a Class-X
  divergence. Renaming them = golden regeneration WITH a documented
  correction note (protocol-sanctioned; the note records the rename, this is
  not weakening). Gate G1 below counts them first.
- D4 LICENSES/pon-license-grant-email.txt: FROZEN. It is primary license
  evidence naming the project as it was when the grant was made (2026-10-16).
  Rewriting it would falsify the record. Add a one-line README note in
  LICENSES/ instead ("project was named antipythany at grant time").
- D5 Directory rename (~/git/antipythany -> ~/git/antipythony): LAST step,
  after everything else is committed and pushed - it invalidates every
  absolute path sibling agents and this session hold.

## Step 0 - Precondition gate (do first, nothing else touches the tree)

0.1 Commit or stash the in-flight wave work: `git status` shows 6 modified
    ap-run files + 3 untracked vendored_stdlib dirs. `crates/ap-run/src/
    builtins_modules.rs` carries 2 ANTIPTYHONY hits INSIDE a dirty file -
    the rename must not commit a half-done wave. Commit the wave (or stash
    with marker), re-verify `git status --short` is clean except the
    untracked vendoring dirs, which get committed as part of step 2.
0.2 Sweep the pinned-golden question (D3):
    `grep -rl "antipythony" tests/*/expected/ tests/corpus/*/expected/ 2>/dev/null`
    - record the count as N_GOLDEN before touching anything. N_GOLDEN = 0
    means D3 is automatic (no golden churn); N_GOLDEN > 0 means each affected
    expected file is regenerated from live CPython behavior for the
    non-divergent bytes + the new name string, with a correction note in the
    sibling .meta or header comment per corpus rule.
0.3 Record the residual baseline: `grep -ri antipythony . --exclude-dir=.git
    --exclude-dir=target --exclude-dir=LICENSES -l | wc -l` -> call it N_BASE.
    Final verification (G4) compares against N_BASE, not zero.

## Step 1 - Identity files (commit 1)

Files: README.md, AGENTS.md, deny.toml, rust-toolchain.toml, NEXT-STEPS.md,
.github/workflows/ci.yml, bughunt-audit-findings.json, and the stray root
file apx_t11_part_importlib_metadata (check contents first - likely a
leftover probe; delete if untracked garbage, rename hits if it is
referenced).

Step 1 also inserts the pronunciation note into README.md. Under the
`# ANTIPYTHONY` title, add one line:

    Pronounced like "antipathy" (/ænˈtɪpəθi/) - an intentional homophone,
    not a misspelling of "antipathy".

Full mapping table for the whole campaign:

| old | new |
| --- | --- |
| antipythany | antipythony |
| Antipythany | Antipythony |
| ANTIPYTHANY | ANTIPYTHONY |
| ANTPYTHANY | ANTIPTYHONY (per D2; no ANTPYTHON short form) |

Gate G_A: `cargo build --workspace` exit 0 (root tomls parse), `rg -li
antipythony` count down by exactly the step-1 file count.

## Step 2 - Crates (commit 2)

- crates/ap-cli/Cargo.toml: `name = "antipythany"` bin stanza -> antipythony
  (the `antipythany` binary disappears; t09 suite must follow, G_B).
- crates/ap-cli/src/main.rs (34 hits) + lib.rs: usage strings, help text.
- crates/ap-pack/src/{uv,cache,lib}.rs: ANTPYTHANY_UV -> ANTIPTYHONY_UV +
  the 4 packaging.md references (docs/ in commit 3).
- crates/ap-run/src/{importer,interp,builtins_modules}.rs: ANTPYTHANY refs
  (6+1+2) - verify each is the env var, not a CPython-visible name, before
  replacing (C-visible strings = golden risk; D3 applies).
- Vendor dirs from step 0: commit the 3 untracked vendored_stdlib items
  (aiohttp/web/, brotli.py, sqlite3/) in their own small commit first so
  the rename commit is pure text.
Gate G_B: `cargo build --workspace` + `cargo test -p ap-cli -p ap-pack`
green + `cargo test -p ap-run --test import_differential
--test import_guard_differential` green.

## Step 3 - vendored_stdlib (commit 3, script it)

98 files: sed the 4-form mapping across `vendored_stdlib/` EXCLUDING any
file that pins upstream-identical bytes for a pristine provenance entry
(xtask stdlib-provenance --check will catch exactly those - run it in the
SAME commit; pristine files that fail the check get hand-written
`needs_decl`/deviated entries, per the standing provenance protocol, and
their headers may legitimately differ from upstream CPython text, which is
what they already say).
Also: xtask stdlib-provenance DEVIATIONS.toml - regenerate via `--fix`,
script the needs_decl entries (sha256 each file, upstream_path = "" for
shims). Same wave, never deferred to next review.
Gate G_C: `xtask stdlib-provenance --check` exit 0.

## Step 4 - docs/ + plans/ (commit 4)

- docs/*.md (28 files) + docs/spec/*: prose mapping, including
  IMPLEMENTATION-STATUS.md and path-to-targets.md.
- `git mv plans/antipythony plans/antipythony`; then sed the moved tree
  (42 files) + every reference to the old path (AGENTS.md, docs, this file).
  plans/robustness-ci untouched (name-neutral).
- This file: its own title line.
Gate G_D: `rg -l "plans/antipythony/"` returns zero; `rg -li antipythony
plans/ docs/` returns zero.

## Step 5 - tests/ (commit 5)

51 files: mapping + the t09 binary-name invocations now point at
`antipythony`. Gate G_E: `pytest tests/t09_binary -q` + `pytest
tests/t11_stdlib -q` + the N_GOLDEN regeneration (from step 0.2) run
through `ap-conform` - regenerated goldens must pass with correction
notes present.

## Step 6 - GitHub repo + push (step 6)

6.1 `gh repo edit bhodgens/antipythony --rename antipythony` (issues,
    PRs, history preserved by GitHub; the old URL 301s).
6.2 `git remote set-url origin git@github.com:bhodgens/antipythony.git`.
6.3 `git push` - the 4 pre-rename commits (f0e8c93..HEAD) go out BEFORE
    the rename commits land, or ride along - either is safe; do not push a
    half-rename tree.
Gate G_F: `gh api repos/bhodgens/antipythony` returns 200; origin/main
tracks.

## Step 7 - local dir rename + verification (final step, D5)

7.1 `mv ~/git/antipythony ~/git/antipythony` (outside git; zero effect on
    history).
7.2 Re-run G4 residual sweep from the new dir:
    `rg -li antipythony -g '!.git' -g '!target' -g '!LICENSES' .` ->
    every remaining hit is a deliberate artifact (LICENSES carve-out note,
    this file's mapping table, N_BASE residual list) or a failure.
7.3 Full gate pass: `cargo test --workspace` (background, -j 2 per the
    SIGKILL note), `pytest tests/ -q` solo, `cargo run -p xtask -- notices
    --check`.
7.4 Update ~/.hermes skills referencing ~/git/antipythony (the two
    antipythony skills) and any cron/delegation paths - list them in the
    completion note so the next session starts from the new dir.

## Explicit non-goals

- Git history: never rewritten. Old commits keep the old name.
- GitHub issues #2-#9 bodies: cosmetic only; not part of the gate.
- .hermes/audits/ and delegation transcripts: session artifacts, leave.
- CPython-visible behavior beyond the D3 golden regeneration: the rename
  changes emitted text only where our output already diverges from
  CPython (shim NotImplementedError texts, our own usage/help strings).
  No Class-S surface moves.

## Recon 2026-11-02: four couplings the original plan under-specified

Measured on the clean tree (764422b). N_BASE = 360 files (excl .git/
target/LICENSES). These are NOT pure text swaps; handle each explicitly.

- C1 (path-coupled goldens, dir-rename gate): 4 corpus goldens embed the
  ABSOLUTE repo path `/Users/caimlas/git/antipythony` (f33_bytes_adjacent_
  error.err, s016_collections_abc_isinstance.err, s023_ast_parse_dump_
  unparse.err, T109-exception-in-thread.stderr). CPython emits the same
  path from the same cwd, so these are PATH-consistent, not product-name
  divergences - they change only when the DIR moves (Step 7), NOT when the
  name text is sed'd. Step 7 must regenerate these 4 + `tests/t07_gc/
  _gen_expected.py` (hardcoded GC path line 10 + cwd line 19) + `tests/
  test_a12_mined_corpus.py` (hardcoded realpath golden line 108), each
  re-verified against live python3.14 from the NEW dir, with a documented
  correction note. A golden regeneration that the dir move did not
  actually change (path still identical) is left untouched.
- C2 (t14 manifest-identity): `crates/ap-pack/src/manifest.rs:74-75`
  defines `MANIFEST_FILE = "antipythony.toml"` / `LOCK_FILE =
  "antipythony.lock"` - the user-facing scaffold artifact. `tests/t14_cli/
  fixtures/expected_antipythony.{toml,lock}` + test_t14_cli.py:205/219/311
  assert those exact filenames. This is a FUNCTIONAL rename (binary's
  `new`/`add` output changes), not text. Rename const + 2 fixture files
  + 3 test refs; gate `pytest tests/t14_cli -q` after a rebuild.
- C3 (cache-dir default): `~/.cache/antipythony` (cache.rs:21, importer.
  rs:144, docs/packaging.md:25). Rename -> `~/.cache/antipythony`; the
  old cache dir becomes an orphaned dev artifact (no migration needed,
  low-risk). Keep the 3 sites consistent.
- C4 (`<antipythony>` marker, CPython-visible): interp.rs lines 11318/
  11612/12462 emit the literal `<antipythony>` as a placeholder module
  name. 0 goldens pin it (verified), and 3 test .py files reference it in
  DOC-STRINGS/comments only (t05, t06, t10) - no assertion. Safe to sed;
  the 3 test docstring refs change text-only, no gate impact.

## Execution (subagent model, clean tree, no live siblings)

- Subagents do in-place text edits ONLY. NO `git add`, NO `git commit`,
  NO `cargo build` except the single scoped build at the end of the
  crates lane. They REPORT their touched-file list + diff summary; the
  orchestrator runs every gate and owns every commit at the step boundary
  (parallel-lane collision rule).
- Lane file ownership (disjoint): LANE-A crates/** + LANE-B
  vendored_stdlib/** + LANE-C tests/** + LANE-D docs/** + plans/** +
  root + LICENSES (carve-out note). One cargo build, one xtask
  provenance check; LANE-C/LANE-D verify by grep only.
- Steps 6-7 (GitHub repo rename + LOCAL DIR MOVE + C1 golden regen) are
  SINGLE-ACTOR and irreversible: run inline by the orchestrator, NOT by a
  subagent. The dir move invalidates every absolute path a subagent or
  this session holds - it is the last thing that happens.
- After the tree-level rename (steps 1-5) lands green, steps 6-7 are a
  separate 2-step tail. Between them, push the rename commits.


## Failure modes to check at review

- t09 suite asserting the old binary name after G_B (suite + binary must
  land in the same gate, both in commit 2/5 - t09 is in commit 5, so G_B
  additionally runs `cargo build --workspace` and a manual
  `target/debug/antipythony --help` smoke).
- Provenance check red after step 3: the pristine/deviated split is the
  mechanism, not an exception - fix entries in the same commit.
- Sibling session holding absolute paths: step 7 is LAST precisely for
  this; announce the dir move in the completion note before it runs.

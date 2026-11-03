# Rename campaign: antipythany -> antipythony

One-shot cross-cutting rename. Not a plan tree with dispatchable leaves: it is
ordered steps with commit boundaries and gates, executable in one session
(~1-2 h agent time) by the orchestrator. Estimated 2026-11-02.

## Inventory (measured 2026-11-02 at f0e8c93)

| Cluster | Files | Notes |
| --- | --- | --- |
| vendored_stdlib/ | 98 | "antipythany deviation shim" headers; runtime NotImplementedError texts |
| tests/ | 51 | binary-name invocations (t09_binary x8), env vars, doc strings |
| plans/ | 42 | prose; plus the DIR `plans/antipythany/` itself |
| docs/ | 28 | prose |
| crates/ | 25 | ap-cli bin name `antipythany` + main.rs usage text (34 hits); ap-pack env prefix ANTPYTHANY_* |
| root | 8 | README, AGENTS.md, deny.toml, rust-toolchain.toml, NEXT-STEPS.md, bughunt json, apx_t11_part_importlib_metadata |
| LICENSES/ | 1 | CARVE-OUT (see decisions) |
| GitHub repo | - | origin = git@github.com:bhodgens/antipythany.git; .github/workflows/ci.yml has 1 hit |

Case variants: 18 files carry ANTIPYTHANY/Antipythany; 15 carry the
ANTPYTHANY env prefix (ANTPYTHANY_UV, plus ANTPYTHANY refs in ap-run
importer/builtins modules - 6 in importer.rs alone).

Total distinct spellings to sweep: antipythany (lc), Antipythany,
ANTIPYTHANY, ANTPYTHANY.

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
  ("antipythany cryptography shim: ...") only execute on paths where CPython
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
    builtins_modules.rs` carries 2 ANTPYTHANY hits INSIDE a dirty file -
    the rename must not commit a half-done wave. Commit the wave (or stash
    with marker), re-verify `git status --short` is clean except the
    untracked vendoring dirs, which get committed as part of step 2.
0.2 Sweep the pinned-golden question (D3):
    `grep -rl "antipythany" tests/*/expected/ tests/corpus/*/expected/ 2>/dev/null`
    - record the count as N_GOLDEN before touching anything. N_GOLDEN = 0
    means D3 is automatic (no golden churn); N_GOLDEN > 0 means each affected
    expected file is regenerated from live CPython behavior for the
    non-divergent bytes + the new name string, with a correction note in the
    sibling .meta or header comment per corpus rule.
0.3 Record the residual baseline: `grep -ri antipythany . --exclude-dir=.git
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
antipythany` count down by exactly the step-1 file count.

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
- `git mv plans/antipythany plans/antipythony`; then sed the moved tree
  (42 files) + every reference to the old path (AGENTS.md, docs, this file).
  plans/robustness-ci untouched (name-neutral).
- This file: its own title line.
Gate G_D: `rg -l "plans/antipythany/"` returns zero; `rg -li antipythany
plans/ docs/` returns zero.

## Step 5 - tests/ (commit 5)

51 files: mapping + the t09 binary-name invocations now point at
`antipythony`. Gate G_E: `pytest tests/t09_binary -q` + `pytest
tests/t11_stdlib -q` + the N_GOLDEN regeneration (from step 0.2) run
through `ap-conform` - regenerated goldens must pass with correction
notes present.

## Step 6 - GitHub repo + push (step 6)

6.1 `gh repo edit bhodgens/antipythany --rename antipythony` (issues,
    PRs, history preserved by GitHub; the old URL 301s).
6.2 `git remote set-url origin git@github.com:bhodgens/antipythony.git`.
6.3 `git push` - the 4 pre-rename commits (f0e8c93..HEAD) go out BEFORE
    the rename commits land, or ride along - either is safe; do not push a
    half-rename tree.
Gate G_F: `gh api repos/bhodgens/antipythony` returns 200; origin/main
tracks.

## Step 7 - local dir rename + verification (final step, D5)

7.1 `mv ~/git/antipythany ~/git/antipythony` (outside git; zero effect on
    history).
7.2 Re-run G4 residual sweep from the new dir:
    `rg -li antipythany -g '!.git' -g '!target' -g '!LICENSES' .` ->
    every remaining hit is a deliberate artifact (LICENSES carve-out note,
    this file's mapping table, N_BASE residual list) or a failure.
7.3 Full gate pass: `cargo test --workspace` (background, -j 2 per the
    SIGKILL note), `pytest tests/ -q` solo, `cargo run -p xtask -- notices
    --check`.
7.4 Update ~/.hermes skills referencing ~/git/antipythany (the two
    antipythany skills) and any cron/delegation paths - list them in the
    completion note so the next session starts from the new dir.

## Explicit non-goals

- Git history: never rewritten. Old commits keep the old name.
- GitHub issues #2-#9 bodies: cosmetic only; not part of the gate.
- .hermes/audits/ and delegation transcripts: session artifacts, leave.
- CPython-visible behavior beyond the D3 golden regeneration: the rename
  changes emitted text only where our output already diverges from
  CPython (shim NotImplementedError texts, our own usage/help strings).
  No Class-S surface moves.

## Failure modes to check at review

- t09 suite asserting the old binary name after G_B (suite + binary must
  land in the same gate, both in commit 2/5 - t09 is in commit 5, so G_B
  additionally runs `cargo build --workspace` and a manual
  `target/debug/antipythony --help` smoke).
- Provenance check red after step 3: the pristine/deviated split is the
  mechanism, not an exception - fix entries in the same commit.
- Sibling session holding absolute paths: step 7 is LAST precisely for
  this; announce the dir move in the completion note before it runs.

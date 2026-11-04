# Development Guide

How to work on antipythony: environment, gates, workflow, and the
project's plan-tree execution protocol.

## Environment

- Rust 1.94.0, installed via Homebrew on this machine (no rustup).
  `rust-toolchain.toml` pins 1.94.0; Homebrew cargo ignores the file but
  CI must match the version. Do not upgrade ad hoc: the ruff crates are
  pinned 0.0.4 because 0.0.5+ exceeds that MSRV (get-size2 pinned
  `=0.10.1` for the compact_str split). Revisit both at a toolchain bump.
- Reference interpreter: `python3.14` on PATH (Homebrew Cellar install).
  Differential suites hardcode `/opt/homebrew/bin/python3.14`; anything
  that runs "CPython" must verify `--version` is a real 3.14.
- pytest for the conformance suite (`pytest tests/` from the repo root).
- No installs without the project owner's explicit OK (standing rule in
  `AGENTS.md`).

## Gates (what "green" means)

Run these separately. Never chain a gate behind a pipe into a commit -
pipes mask exit codes and a red suite once rode into a commit this way.

```
cargo build --workspace
cargo test --workspace            # Rust unit + integration (differential) tests
cargo fmt --check
pytest tests/                     # full conformance suite
cargo run -p xtask -- notices --check   # THIRD_PARTY_NOTICES drift gate
```

Current baseline: rust 289/0, pytest 1272 passed / 0 failed (plus
skips/xpasses tracked in `plans/antipythony/tests/TRACKING.md`).

### Why CI gates, not git hooks

These gates run in CI, not as local git hooks, on purpose: this machine
has a global `core.hooksPath=~/.githooks` (git-lfs hooks only; verify with
`git config --get core.hooksPath`), which silently overrides and disables
any repo-local `.git/hooks` — a pre-commit hook added here would be a
no-op that *looks* like it runs. If repo hooks are ever wanted, onboarding
must explicitly set `git config core.hooksPath .githooks` per clone;
until then, treat CI (`.github/workflows/ci.yml`) as the only gate that
actually fires. (robustness-review finding #15; documented by leaf 03.03.)

## Repository conventions

- Commit by explicit pathspec. Never `git add -A` after a gate whose exit
  code you did not see. Machine-regenerated reports
  (`tests/t08_tier1/bench_report.json`) are gitignored - gitignore such
  files the day they are created.
- `AGENTS.md` is this repo's cross-session memory. Update its "Current
  status" block whenever project state changes; keep OPEN items current -
  a stale OPEN item sends agents re-investigating solved problems.
- Per-plan `PROGRESS.md` is the session-continuity record: read it before
  dispatching into a plan, write (merge, never overwrite) after each leaf
  commits.
- License headers: every crate carries the SPDX line. Before any
  pon-derived commit: LICENSES/ entries plus attribution headers on
  adapted files. Nuitka is AGPL - subprocess tool only, never vendored.

## Plan-tree execution protocol

Work is organized as a hierarchical plan tree under
`plans/antipythony/` (see `master.md` for the contract):

1. Implementation leaves fit one 256K-context agent; split along a named
   expose/gate boundary when a leaf overruns.
2. Tests are written from the spec pack (`docs/spec/`) by DIFFERENT
   agents, in a separate pass, never from implementation source. Test
   agents get no `crates/` reads and no git history.
3. Nothing dispatches from Plans 01+ until the spec pack (00.05) exists -
   it is the single semantics source.
4. A leaf's completion gate is its named test leaf green, verified by the
   orchestrator with fresh five-line spec probes against python3.14 - the
   leaf's own green suite misses exactly the constructs it never tested.
5. Two implementer agents in parallel: ownership is quarantined by FILE
   (one agent owns interp.rs/frame.rs, another builtins.rs + its new
   modules); the workspace-level gate is the orchestrator's, run after
   both land. State ownership lines in every dispatch.
6. When two leaves' work must meet, sequence the merge into the SECOND
   leaf's dispatch rather than letting two agents edit a shared region.
7. Every leaf report carries an HONEST gap list (what it did not
   implement, with repro ids). Gaps are queue entries, not shame; a
   hidden gap is the only failure. Copy gaps into the plan PROGRESS.md
   queue.
8. When sources disagree: pinned fixtures and live reference behavior
   beat the leaf brief's paraphrase. Record the delta in PROGRESS.md.
9. Precedence for divergence: a spec amendment is a NUMBERED entry
   (rationale + narrowed name lists, original clause ids preserved).
   Never let code silently diverge from spec.

## Useful paths

- Plan tree: `plans/antipythony/{master.md, 00..04, 10, tests/}`
- Design notes: `docs/` (gc-mmtk-swap, pon studies, import machinery,
  path-to-targets)
- Audits/reports: `.hermes/audits/` (gitignored)
- Delegation transcripts (append-only forensic records of agent runs,
  including full file contents later lost):
  `~/.hermes/cache/delegation/live/<id>/task-N.log`

## Adding a conformance corpus case

1. Pick the family dir under `tests/corpus/`.
2. Write `<case>.py` - one statement per line where practical; start the
   file with `# Cites: S-NN-NN, C-NN` citing real spec clauses.
3. Generate expected bytes with the family's `gen_expectations.py` (runs
   live python3.14). Never hand-write a pin.
4. Run the traceability lint and the differential runner; the new case
   must pass or be a named queue entry.
5. Never weaken an existing case to make a gate pass; the corpus only
   grows.

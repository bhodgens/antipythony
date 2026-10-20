# Branch 00 - CI Core

Goal: a real GitHub Actions pipeline mirroring the local gate stack. Today
only `.github/workflows/attribution.yml` exists; 91+ commits landed with no
remote build/test/lint gate (review findings #1 #2 #3 #4 #6 #19 #20).

## Design

Jobs (sketched in the review at `## Proposed CI pipeline`; refine it):

- `fast-lints` - no compile: pytest `tests/corpus_traceability` + T-01 rule
  lints, notices drift check. Seconds.
- `rust` - pinned 1.94.0: fmt --check, clippy -D warnings, build, test,
  cargo doc. Needs CPython 3.14 on PATH (differential tests).
- `deny` - `cargo deny check` (master.md:45 promise, finding #2).
- `conformance` - ubuntu (PR + main) and macos-14 (main only; ~10x runner
  cost) matrix; builds bins, exports APX_IMPL/APX_CLI, runs pytest suite.
- attribution.yml retires once its job is inside ci.yml; until then leave it.

## Leaves

### 00.01 - ci.yml core jobs (fast-lints, rust, deny)
Files: `.github/workflows/ci.yml` (new), retire `.github/workflows/attribution.yml`
last. Steps:
0. VERIFY setup-python@v5 python-version "3.14" works on ubuntu-latest for
   this repo's account (unverified pre-condition, master.md Deferred). If it
   fails, use astral-sh/setup-uv with a managed 3.14; record choice in
   PROGRESS.
1. toolchain: dtolnay/rust-toolchain@master toolchain "1.94.0" components
   rustfmt,clippy; add `rustc -vV` assertion step.
2. jobs per master contract: Swatinem/rust-cache@v2; `cargo fmt --all --check`;
   `cargo clippy --workspace --all-targets -- -D warnings` (NOTE: -D warnings
   lands in 02/01's policy; if the tree is not clippy-clean today, 00.01
   ships this job with `-D warnings` only on `cargo fmt` and files clippy as
   02/01's gate - do NOT water down to warning-free-by-deletion);
   `cargo build --workspace` then `cargo test --workspace` in one job
   (integration tests path-math into target/debug);
   `cargo run -p xtask -- notices --check`; `cargo deny check` via
   taiki-e/install-action.
3. `AP_REQUIRE_REFERENCE: "1"` env at workflow level.
Gate: push a branch, gh run watch shows all jobs green; each job proven to
fail when its gate is broken (one deliberate red per job: stale notices,
bad fmt). Honest gap list required (e.g. clippy deferred, fuzz deferred).

### 00.02 - Reference discovery + AP_REQUIRE_REFERENCE
Unify three inconsistent python3.14 resolutions (finding #3 #4):
- Rust tests: `crates/ap-run/tests/re_differential.rs:12`,
  `classes_differential.rs:15`, `method_surface_differential.rs:13`,
  `metaclass_enum_differential.rs:17`, `import_differential.rs:20`,
  `fix_pass_differential.rs:20`, `crates/ap-parse/tests/corpus.rs` - replace
  `const REF` hardcodes and python3 fallbacks with a shared helper
  (new `crates/ap-run/tests/common/mod.rs` or a small `ap-test-support`
  path-dependency) implementing: env AP_CONFORM_REFERENCE_PYTHON -> PATH
  python3.14 -> `--version` string check (reuse the logic at
  `crates/ap-conform/src/lib.rs:90-96`). When AP_REQUIRE_REFERENCE=1 and no
  reference found: panic with a clear message (fail), not skip.
- pytest suites: honor the same env var; default fallback keeps the
  Homebrew path only when no env is set.
Gate: `AP_REQUIRE_REFERENCE=1 cargo test -p ap-run --test re_differential`
fails loudly with PATH scrubbed of python3.14; passes normally. All suites
still green locally without the env var.

### 00.03 - Conformance matrix job
Adds the pytest differential job (ubuntu on PR+main, macos-14 on main only).
Build workspace first; export APX_IMPL + APX_CLI; run `pytest tests/ -q`;
report counts as job summary (values/fstrings/stdlib per-slice from
ap-conform JSON mode). Gate: workflow green on both OSes with reference
version check passing legitimately (thanks to 00/02).

### 00.04 - Branch protection recommendations (human-facing doc)
Write `docs/ci-branch-protection.md`: required checks list (rust, deny,
fast-lints, conformance), no force-push to main, PR review requirement;
`gh api` commands for the user to apply. No code.

## Dispatch notes

- 00.01 first; 00.02 can run parallel (touches tests/, not workflows/).
- 00.03 depends on 00.01 + 00.02. 00.04 any time.
- Do not touch crates/ap-run/src/* here (sibling Plan 02 waves own it).

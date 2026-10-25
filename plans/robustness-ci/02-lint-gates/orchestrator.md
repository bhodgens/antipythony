# Branch 02 - Lint Gates

Goal: policy + one cheap defensive fix. Findings #7 (149 unwrap panic
surfaces), #8 (no lint policy at all), #16 (importer path-segment guard).

## Leaves

### 02.01 - workspace.lints + clippy rollout
Files: root `Cargo.toml` (add `[workspace.lints]`), per-crate Cargo.toml
`[lints] workspace = true`, new `clippy.toml`.
- `[workspace.lints.rust]` minimal (broken doc links handled in 05/03).
- `[workspace.lints.clippy]`: `unwrap_used = "warn"`, `dbg_macro = "deny"`,
  `todo = "warn"`. Per-crate allowance: crates that cannot be clean today
  (ap-obj 88 unwraps, ap-run 61) carry
  `#[allow(clippy::unwrap_used)]`-style crate-level allowances with a
  TODO comment naming this leaf's number as the burn-down owner - the point
  is NO NEW unwraps in clean crates (ap-parse, ap-ir, ap-conform, xtask,
  ap-cli) while ap-obj/ap-run stay grandfathered.
- Do NOT mass-rewrite the 149 sites (review is explicit).
- Classify the unwraps FIRSTHAND (the review only spot-checked): one pass
  `grep -n "unwrap()" crates/ap-obj/src crates/ap-run/src` -> tally into
  PROGRESS.md so future leaves know the real number.
- ci.yml clippy step then flips from informational to `-D warnings`
  (coordinate with 00/01: if 00/01 deferred the step, land it here).
Gate: `cargo clippy --workspace --all-targets` exits 0 with the allow-lists;
a NEW unwrap() in ap-parse trips the warning; fmt gate still green.

### 02.02 - Importer segment guard
File ownership: `crates/ap-run/src/importer.rs` ONLY (sibling Plan 02 waves
may still touch interp.rs - this leaf must not).
- In `find_submodule`/`find_on_path` (importer.rs:185-206): reject module
  name segments equal to "", ".", ".." or containing "/" or "\\ before any
  path join. Raise `ImportError` with CPython's exact
  "ImportError: invalid module name" bytes (probe live python3.14 with
  `__import__('')` etc. FIRST to pin the real message - do not trust the
  review's paraphrase).
- Add corpus case(s) `tests/corpus/stdlib/` citing spec 04 clauses + a
  Rust differential probe in `crates/ap-run/tests/import_differential.rs`
  (append-only; respect any sibling edits to that file - if dirty, put the
  probes in the leaf's own new test file instead).
Gate: guard probes byte-exact vs python3.14; full corpus unchanged or
higher.

## Dispatch notes

02.01 touches every Cargo.toml - run when no sibling leaf is mid-commit on
manifests. 02.02 is single-file; parallel-safe against 02.01 except both
want a green cargo clippy - sequence 02.02 first if the gate collides.

## Test Gate

<!-- Tests-first gate (hierarchical-planning skill v1.3.0, patched 2026-10-24).
     Coverage matrix derived mechanically from leaf task headers; the control
     agent (orchestrator, in-session) refines rows and fills test names at
     Phase 0 dispatch. No implementation leaf dispatches until every row is
     VALIDATED. Delta protocol: lesser of plan/tests updates to the greater.
     Recipe: skill hierarchical-planning -> references/tests-first-gate.md -->

| # | Feature (from plan) | Source | Unit tests | E2E tests | Status |
|---|---------------------|--------|-----------|-----------|--------|
| F1 | Branch 02 - Lint Gates: PROGRESS | PROGRESS.md | (pending) | (pending) | PENDING |

Statuses: PENDING | DELTA | VALIDATED

Delta log: (none yet)

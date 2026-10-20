# Branch 05 - CLI + Docs Automation

Goal: discoverable testability CLIs and automated documentation. Findings
#17 (hand-rolled bins, missing ap-parse-check seam), #18 (no rustdoc gate),
plus the review's docs roadmap items (clap-generated CLI reference, mdBook).

## Leaves

### 05.01 - clap migration of the 4 existing bins
Files: `crates/ap-cli/src/bin/ap_run_eval.rs`, `ap_obj_eval.rs`,
`crates/ap-parse/src/bin/ap_ir_dump.rs`, `ap_verify_dump.rs`; Cargo.toml
dev/deps (clap already in-tree via ap-conform - license clean).
- Preserve EXACT stdout/stderr contracts the suites pin (the bins' output
  IS the test seam; clap only changes arg parsing, never output bytes).
  Add `--help` + version; keep positional `<file.py>` unchanged.
- Danger: clap's own error text lands on stderr for bad args; ensure the
  pinned-error paths suites rely on are untouched, or scope the migration
  to arg parsing ahead of the pinned output region.
Gate: full pytest suite green with no golden edited; `--help` present on all
4 bins.

### 05.02 - ap-parse-check seam bin (closes TRACKING gap)
- T-01 suite reads env `AP_PARSE_CHECK` but the bin does not exist
  (tests/TRACKING.md decision #3, confirmed absent from every Cargo.toml).
- Implement `ap-parse-check <file.py>` in ap-parse: parse + print
  CPython-format diagnostics (the surface Plan 01 already produces - this
  is a bin wrapper, not new semantics). Contract: the T-01 suite's
  expectations define the output shape; read
  `plans/antipythany/tests/T-01-parse-diagnostics.md` first.
- Wire into ci.yml fast-lints: T-01 suite flips xfail->run.
Gate: T-01 suite green in real-run mode; zero xfail left for the seam
reason.

### 05.03 - rustdoc gate + missing_docs policy
- ci.yml rust job: `cargo doc --no-deps -p ap-obj -p ap-ir -p ap-parse
  -p ap-run -p ap-conform` with `RUSTDOCFLAGS=-D warnings` (broken intra-doc
  links fail). Fix or scope-ignore pre-existing link errors with
  `#[allow(...)]` + note - report the tally.
- `#![warn(missing_docs)]` crate-level on ONLY the three public surfaces
  (ap-obj, ap-ir, ap-conform); internals stay doc-free per review roadmap
  item 2.
Gate: doc job green; new public item without docs trips the warning in the
3 crates.

### 05.04 - Generated CLI reference
- `xtask cli-docs`: render `--help` from each clap Command
  (Command::render_help) into `docs/testability-clis.md`; CI step regenerates
  and diffs (same drift-gate pattern as notices). Depends on 05/01.
Gate: docs/testability-clis.md committed; stale-render check fails on drift.

### 05.05 - mdBook site (HOLD)
- mdBook over docs/: src/SUM.md indexes spec pack 01-06, compat-contract,
  conformance.md, architecture.md, development.md, path-to-targets; design
  notes (pon-study, gc-mmtk) in an appendix. Deploy GitHub Pages from a CI
  job. HOLD per master.md: start only after Plan 03 docs settle (spec 07
  lands). Do not begin in this campaign without orchestrator sign-off.

## Dispatch notes

05.01 -> 05.02 -> 05.04 dependency chain (clap first). 05.03 independent.
05.05 gated. None of these touch ap-run/src (sibling-safe).

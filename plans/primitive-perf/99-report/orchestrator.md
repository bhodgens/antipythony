# 99-report orchestrator

Final chart + report. Consumes the FINAL bench_report.json.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-final-report.md | `docs/primitive-perf-report.md` + chart | all groups COMPLETE (or deferred-approved) |

## Dispatch Protocol

1. After all groups: run `run_bench.py` one final time (fresh numbers,
   merged tree, release build).
2. Dispatch 01: produce the report per the leaf spec.
3. Master exit gate: report exists, chart readable, every row PASS or
   documented deferral.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-final-report.md | PENDING |

## Interface Contracts

- bench_report.json schema is frozen in this document (see above).
- File-ownership rules from master.md apply to every child.

## Review Checklist

- [ ] child deliverables exist and match their leaf spec
- [ ] frozen schema/contracts respected
- [ ] no out-of-scope file edits
- [ ] tracking table updated after each child transition

## Coding Conventions

- Python: no tabs in f-strings; probes < 80 lines; deterministic workloads.
- Rust: no new crate deps without master sign-off; aarch64 guards + scalar fallback for SIMD code.
- Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4.

## Integration Test Plan

- Byte-exactness: ap-conform values + stdlib slices green on the merged tree after each group.
- Perf: full `run_bench.py` re-run refreshes the shared bench_report.json (single-writer, serialized by master).

## Test Gate

Report group: gate = chart numbers regenerate from bench_report.json (not transcribed); every master P-row covered; honest-gaps section present.


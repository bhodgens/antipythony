# 00-harness orchestrator

Build the benchmark harness and the antipythony-vs-CPython baseline
survey. Everything downstream reads this group's `bench_report.json`.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-harness-runner.md | `tests/bench/primitive/run_bench.py` + probe files + JSON schema | none |
| 02-baseline-survey.md | run all primitives both interpreters, emit baseline `bench_report.json` + audit notes | 01 |

## Dispatch Protocol

1. Dispatch 01. Review in-session (runner runs green on a smoke slice:
   base64 + hexlify at 48 KiB, both interpreters, JSON emitted).
2. Dispatch 02. Review: bench_report.json has a row for every primitive
   in master.md's slice; every row carries cpython_s, antipythony_s,
   ratio; audit notes name per-primitive implementation location
   (file:line) in our tree and the known SOTA target.

## Test Gate

Harness is tooling, not shipped code: gate = the runner runs green on
the smoke slice and emits schema-valid JSON (validated in-session by the
orchestrator). No pytest leaves required.

## Interface contract (frozen)

`bench_report.json`:
```json
{
  "generated": "ISO-8601 timestamp",
  "machine": "Mac M2 Ultra",
  "python_ref": "3.14.x",
  "antipythony_build": "release",
  "rows": [
    {
      "id": "P01",
      "primitive": "base64.b64encode",
      "size": "48KiB",
      "cpython_s": 0.00006,
      "antipythony_s": 0.00004,
      "cpython_gbs": 0.79,
      "antipythony_gbs": 1.17,
      "target_gbs": 24.0,
      "target_source": "CPython PR #162443 (NEON)",
      "ratio_vs_cpython": 1.48,
      "ratio_vs_target": 0.049,
      "verdict": "FAIL",
      "impl_site": "crates/ap-run/src/builtins_modules.rs:12816"
    }
  ]
}
```
Target gbs only set where an upstream SOTA number exists (base64 PR);
otherwise `target_source`: "beat CPython as shipped", `target_gbs` =
cpython_gbs.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-harness-runner.md | PENDING |
| 02-baseline-survey.md | PENDING |

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


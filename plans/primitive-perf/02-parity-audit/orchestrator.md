# 02-parity-audit orchestrator

Audit the remaining primitive families. This group is AUDIT-FIRST: it
writes findings docs, never edits implementation code (avoids file
collisions with 01-encode-simd, which owns builtins_modules.rs). Fix
leaves are spun out by master.md from the audit findings.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-hash-and-compress.md | P04 zlib/bz2/lzma + P05 hashlib | baseline |
| 02-str-and-bytes.md | P06 str methods + P07 bytes/int conversions | baseline |
| 03-sort-dict-json.md | P08 sort + P09 dict + P10 json | baseline |

## Dispatch Protocol

All three in parallel after 00-harness. Each leaf: read the bench
rows for its primitives, read our implementation, compare against
CPython's algorithm + the Rust ecosystem's best crate for the job
(hashbrown/fxhash for dict, rayon-free pdqsort in std, serde_json
numbers as reference points), and write a findings doc per family:

`plans/primitive-perf/02-parity-audit/audits/<family>.md` containing:
- per-primitive: our impl site, CPython's algorithm (file + function
  name), the Rust-crate SOTA, our measured ratio, gap analysis
- verdict per primitive: ALREADY-PARITY / FIXABLE (with a concrete
  plan: which algorithm, est. gain, est. effort) / NOT-WORTH (with
  reason)
- ONLY concrete FIXABLE items become fix leaves (master dispatches
  them after 01-encode-simd's file ownership frees up)

## Test Gate

Audit leaves produce documents; gate = every bench row for the
family's primitives is explained in the audit (no unexplained gaps).

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-hash-and-compress.md | PENDING |
| 02-str-and-bytes.md | PENDING |
| 03-sort-dict-json.md | PENDING |

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


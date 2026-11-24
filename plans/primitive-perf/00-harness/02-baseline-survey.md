# 02-baseline-survey — run the full baseline

## DISPATCH INSTRUCTION

Any agent: execute this leaf. Do NOT commit; run, write the audit doc,
report.

Parent: plans/primitive-perf/00-harness/orchestrator.md
Depends: 01-harness-runner.md COMPLETE.
Estimated context: ~35K.

## Tasks

### T1 — build antipythony release

```
CARGO_BUILD_JOBS=4 cargo build --release -p ap-cli -p ap-conform
```
into a worktree-local target dir (machine policy: one heavy lane).

### T2 — full baseline run

`python3 tests/bench/primitive/run_bench.py --antipythony <release bin>
--python python3.14 --out tests/bench/primitive/bench_report.json`

### T3 — audit notes

Create `plans/primitive-perf/00-harness/baseline-notes.md`:
- For each row: our implementation site (file:line), one-line
  implementation summary (scalar loop / crate call / interpreter walk),
  and the SOTA target with source (PR #162443 numbers for base64;
  "beat CPython" for the rest).
- Rank all FAIL rows by gap size (ratio_vs_target ascending) — this
  ordering feeds master.md's dispatch decisions.
- Flag any row where antipythony ERRORS (feature gap, not perf).

## Self-verification checklist

- [ ] bench_report.json has a row for every probe × size
- [ ] every FAIL row has impl_site + SOTA target in baseline-notes.md
- [ ] ranking table present

## Review checklist (orchestrator)

- [ ] spot-check 2 impl_site references (read the cited lines)
- [ ] SOTA targets sourced (PR link or explicit "beat CPython")
- [ ] ranking consistent with the JSON

# 01-final-report — chart + report

## DISPATCH INSTRUCTION

Any agent: produce the final report. Do NOT commit. Parent:
plans/primitive-perf/99-report/orchestrator.md. ~40K.

## Tasks

### T1 — final measurement
Fresh `run_bench.py` full run on the merged tree, release build. This
is THE number set for the report.

### T2 — chart
Create `docs/primitive-perf-chart.svg` (hand-written SVG, no deps):
horizontal bars per primitive, antipythony GB/s vs CPython GB/s vs
target GB/s (three bars per row, log scale if the spread demands).
If SVG proves fiddly, a precisely-aligned ASCII chart inside the
report is acceptable — prefer SVG.

### T3 — report
Create `docs/primitive-perf-report.md`:
- Method summary (harness, best-of-5, machine, builds)
- The chart embedded + the raw table (every row: cpython GB/s,
  antipythony GB/s, ratio vs CPython, ratio vs target, verdict)
- Per-group narrative: what was optimized, what the fix was, before/
  after numbers
- Honest gaps section: every FAIL/deferral with the measured gap and
  the reason (effort/risk/dependency-posture)
- Reproduce section (exact commands)

## Self-verification checklist
- [ ] chart numbers == bench_report.json numbers (regenerate, don't transcribe)
- [ ] every master.md P-row covered
- [ ] honest gaps section present even if all-PASS

## Review checklist (orchestrator)
- [ ] spot-check one chart bar against the JSON
- [ ] exit gate: no unexplained FAIL rows

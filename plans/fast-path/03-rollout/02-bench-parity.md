# 02-bench-parity — closing full bench + report

## DISPATCH INSTRUCTION

Worktree /Users/caimlas/.git/worktrees/leaf-fp-bench (branch
tier1-fp-bench, from current main). Own commits, explicit pathspecs.
ONE heavy cargo lane; CARGO_BUILD_JOBS=4; CARGO_TARGET_DIR=$PWD/
target-fp-bench everywhere.

Parent: plans/fast-path/master.md. DEPENDS: 03-rollout/01 MERGED
(bytecode default live). ~50K context.

## Deliverable

1. **Closing full bench** in the new default mode (bytecode/adaptive):
   all 64 rows, loadavg-gated, single writer of bench_report.json.
2. **Exit-gate check** (master.md): dict.getitem/build, list.sort,
   str.join/split, json.loads — none below 1.0x; simple mean >= 1.5x.
   Rows still failing carry documented deferrals naming the structural
   floor.
3. **Report**: docs/fast-path-report.md — method (bytecode
   architecture, dispatch design, caches), per-group before/after
   (tree-walk vs bytecode vs CPython), honest gaps, reproduce section.
4. **Chart**: regenerate primitive-perf-chart.svg from the new JSON
   (gen_chart.py — numbers regenerated, never transcribed).
5. **README figures**: update the perf section with the bytecode-mode
   numbers.

## Gates

- All 8 diff gates + conform (values/stdlib, both modes) + cargo
  suites green on the final tree.
- Full corpus pytest leg (the pytest seam now points at the redirect
  target dir).

## Files you own

docs/fast-path-report.md, docs/primitive-perf-chart.svg,
tests/bench/primitive/bench_report.json, README perf section,
docs/benefits-vs-cpython.md sync.

## Self-verification checklist
- [ ] chart numbers == JSON numbers (regenerated)
- [ ] exit-gate rows checked and documented
- [ ] README + benefits doc updated
- [ ] all gates green on the final tree

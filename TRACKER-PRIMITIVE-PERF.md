# TRACKER-PRIMITIVE-PERF — campaign tracker (orchestrator session 2026-11-24)

Plan tree: `plans/primitive-perf/` (master.md is the contract).
Worktree: `/Users/caimlas/.git/worktrees/leaf-primperf` (branch `tier1-primperf`,
branched from main @ b1c89a1). ALL leaf work happens there. Merges go back to
main via the orchestrator after first-hand gate re-runs.

## Machine / environment pins

- Mac M2 Ultra, aarch64, NEON. ONE heavy cargo lane; CARGO_BUILD_JOBS=4.
- Leaf target dir: `target-primperf` inside the worktree.
- CPython reference: /opt/homebrew/bin/python3.14 (version-string checked).
- antipythony release binary: `~/.git/worktrees/leaf-primperf/target-primperf/release/antipythony`
- Disk guard fired at session start: 28G free -> cargo clean on merged leaf
  worktrees reclaimed ~28G (56G free at dispatch). Do not re-clean without
  re-measuring.

## Frozen contracts

- bench_report.json schema: plans/primitive-perf/00-harness/orchestrator.md
  ("Interface contract (frozen)"). Single-writer rule: only ONE agent at a
  time ever refreshes bench_report.json (orchestrator serializes).
- File ownership: 01-encode-simd group owns
  crates/ap-run/src/builtins_modules.rs (b64/hexlify arms only) + new
  crates/ap-run/src/simd_encode.rs. 02-parity-audit leaves are AUDIT-ONLY
  (write only plans/primitive-perf/02-parity-audit/audits/*.md). Fix leaves
  from audit findings are spun out AFTER 01-encode-simd frees the file.

## In-flight / queue

| # | leaf | owner surface | status | delegation |
|---|---|---|---|---|
| W1 | 00-harness/01-harness-runner.md | tests/bench/primitive/* (new files, no cargo) | IN FLIGHT | deleg_7a6d3a8f |
| W1 | release build of ap-cli in leaf worktree | target-primperf | IN FLIGHT (orchestrator background proc) | proc_196de983a269 |
| W2 | 00-harness/02-baseline-survey.md | bench_report.json + baseline-notes.md | QUEUED (needs W1 + release binary) | - |
| W3 | 01-encode-simd/01-base64-simd.md | builtins_modules.rs b64 arms + simd_encode.rs | QUEUED (needs W2) | - |
| W4 | 01-encode-simd/02-hexlify-simd.md | binascii hex arms + simd_encode.rs | QUEUED (needs W3 reviewed) | - |
| W3 | 02-parity-audit/01-hash-and-compress.md | audits/hash-compress.md ONLY | QUEUED (needs W2; dispatch parallel with W3) | - |
| W3 | 02-parity-audit/02-str-and-bytes.md | audits/str-bytes.md ONLY | QUEUED (needs W2) | - |
| W3 | 02-parity-audit/03-sort-dict-json.md | audits/sort-dict-json.md ONLY | QUEUED (needs W2) | - |
| W5 | 99-report/01-final-report.md | docs/primitive-perf-report.md + chart.svg | QUEUED (needs W3+W4 merged) | - |

## Dispatch waves

- W1: harness leaf (Python-only) + orchestrator-side release build, parallel.
- W2: baseline survey (single agent; owns the bench_report.json writer role).
- W3: base64-simd leaf + all 3 audit leaves in parallel (file-disjoint:
  base64 owns builtins_modules.rs b64 arms; audits write only .md docs).
- W4: hexlify-simd after base64 is REVIEWED (shares simd_encode.rs).
- W5: final report after W3+W4 merged to trunk and gates re-run.
- Audit FIX leaves (if any FIXABLE verdicts): dispatched by master after
  01-encode-simd file ownership frees up (i.e. after W4 merges).

## Merge gates (verify-before-merge, every implementation leaf)

- Differential byte-exactness gate FIRST (leaf's diff_*.py), then perf.
- ap-conform values + base64/hex stdlib corpus slices green.
- `cargo test -p ap-run --release` green in the leaf's target dir.
- Orchestrator re-runs headline gates first-hand in the worktree before
  merge; re-runs on trunk after merge (fresh binary rebuild).
- Never weaken a corpus case. No new crate deps without user sign-off
  (memchr/serde_json proposals from audit leaves = user decision items).

## Completion definition (exit gate)

docs/primitive-perf-report.md exists with the chart; every P-row in
bench_report.json verdict PASS (antipythony >= upstream SOTA target) or
carries a documented deferral with the measured gap. Final full bench
re-run on merged trunk produces the report numbers. AGENTS.md status
block + this tracker updated at close-out.

## NOT in scope

- JIT/tier-1 performance (plan measures tier-0 native builtin speed by design).
- Wasm leg of the corpus gates (native-only perf campaign; wasm rows unchanged).
- Vendoring serde_json/memchr without explicit user approval.

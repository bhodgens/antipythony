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
| W1 | 00-harness/01-harness-runner.md | tests/bench/primitive/* (new files, no cargo) | DONE — orchestrator-verified (schema OK, 6 smoke rows, reproducible) | deleg_7a6d3a8f |
| W1 | diff gates diff_b64.py + diff_hex.py (upfront gate authoring) | tests/bench/primitive/diff_*.py only | DONE — verified (hex 1832/1832 GREEN; b64 RED w/ 8 scoped API divergences, routed into base64 leaf brief) | deleg_1f905825 |
| W1 | audits: hash-compress / str-bytes / sort-dict-json (analysis-only, ratio placeholders) | 02-parity-audit/audits/*.md ONLY | DONE — verified (764 lines total; found 2 to_bytes/from_bytes correctness bugs, 3 sort correctness bugs, P10 baseline discrepancy which orchestrator reconciled) | deleg_43751b52 |
| W1 | release build of ap-cli in leaf worktree | target-primperf | DONE (ap-cli + ap-conform release, smoke-verified) | proc_196de983a269 |
| W2 | 00-harness/02-baseline-survey.md | bench_report.json + baseline-notes.md | DONE — orchestrator-verified (50 rows, 17P/25F/8E, ranking consistent, cites read) | deleg_6a6a9f3c |
| W3 | 01-encode-simd/01-base64-simd.md | builtins_modules.rs b64 arms + simd_encode.rs | MERGED to main (dd3eda0) — gates re-verified on trunk: diff_b64 2132/2132, diff_hex 1832/1832, values 50/50, stdlib 48/48, cargo release green. encode 16.8-18.7 GB/s (target 24 = honest gap: dispatch overhead + M2-vs-M5 hardware), decode 10.0-10.9 GB/s (target 14, gate >=10 MET) | deleg_58d14553 |
| W4 | 01-encode-simd/02-hexlify-simd.md | binascii hex arms + simd_encode.rs | MERGED to main (a0dc0f8) — gates re-verified on fresh trunk binary: diff_hex 1976/1976, diff_b64 2132/2132. hexlify 2.06 GB/s PASS (1.23x cpy), unhexlify 6.53 GB/s PASS (3.81x cpy); 10x aspiration = interpreter-arm ceiling honest gap | deleg_8e87fec8 |
| W3 | 02-parity-audit/01-hash-and-compress.md | audits/hash-compress.md ONLY | DONE — merged (3cb3a55); headline fixes: sha2 asm-aarch64 flag (existing dep, est 4-8x), crc32fast (in lock, est 10-30x); bz2/lzma = NotImplementedError shims (new-dep user decisions) | deleg_43751b52 |
| W3 | 02-parity-audit/02-str-and-bytes.md | audits/str-bytes.md ONLY | DONE — merged (3cb3a55); 2 correctness bugs (to_bytes zero-fill, from_bytes truncation >16B), int(str) no small-int fast path, memchr = user dep decision | deleg_43751b52 |
| W3 | 02-parity-audit/03-sort-dict-json.md | audits/sort-dict-json.md ONLY | DONE — merged (3cb3a55); json is NATIVE (dumps 0.97x, loads 0.17-0.24x w/ opt leaf sketch); dict ALREADY-PARITY; 3 sort correctness bugs; int-loop pathology ~35us/elem filed (explains P09 timeouts) | deleg_43751b52 |
| W5 | 99-report/01-final-report.md | docs/primitive-perf-report.md + chart.svg | MERGED to main (1e25df5; branch commits 9a47546/72e912d) — chart==JSON verified; orchestrator corrected attribution: the 8 audit-found bugs are FILED-not-fixed (verified reproducing on trunk) | deleg_838ea88d |
| W6 | ppfix-perf: sha2 asm-aarch64 + zlib one-shot decompress | ap-run Cargo.toml features + zlib decomp/hashlib arms | MERGED to main (af172e8) — trunk gates: fix_sha_zlib green, values 50/50 + stdlib 48/48, cargo 18/18. zlib.decompress 0.38-0.52x -> 1.02-1.33x PASS x4; sha256 0.14x -> 0.88x (~6x; residual vs OpenSSL crypto-ext = backend-class gap, new-dep decision if we want the last 12%) | deleg_ac90809e |
| W6 | ppfix-int: to_bytes/from_bytes bugs + hex limb path | builtins_modules.rs int arms + builtins.rs hex/oct/bin | MERGED to main (6bb2ab9) — trunk-verified: fix_int green, both bugs closed first-hand, values 50/50 + stdlib 48/48 | deleg_ac90809e |
| W6 | ppfix-sort: 3 mixed-type/key=None sort bugs | builtins.rs sorted + methods.rs/builtins_modules.rs sort arms | MERGED to main (b6afd8c) — trunk-verified: fix_sort 42/42, sorted-mixed now raises exact TypeError, values 50/50 + stdlib 48/48; bisect: Int pathology = superlinear listcomp/append, NOT compare dispatch | deleg_ac90809e |
| W6 | ppfix-random: implement random.Random class | native random module surface | IN FLIGHT (worktree leaf-ppfix-random) | deleg_ac90809e |
| W6b | ppfix-json: json.loads/dumps bulk scan + escape tables | json region of builtins_modules.rs (~:7361-7843, region-disjoint from W6) | MERGED to main (55adf5a) — trunk-verified: fix_json 127/127, dumps 1.02x/1.13x PASS both sizes (0.92x gap closed), loads kernels 1.4-2.2x faster than CPython (residual = container-mint floor, wave-2); values 50/50 + stdlib 48/48 | deleg_cd8e6817 |
| W6b | ppfix-dictprobe: p09 probe restructure + per-op dict ratios | tests/bench/primitive/probes/p09_dict.py + report ONLY | MERGED to main (2b205cd) — 6 P09 ERROR rows unblocked (12 real rows, zero ERROR); KEY FINDING: Int interpreter-loop pathology is ~97% of every dict row (loop alone ~4500x CPython, d[k] only 13.5x, d.get bound-method dispatch +5.2us/call); wave-2 target = Fix C (Int loop), NOT dict structure (ALREADY-PARITY confirmed) | deleg_cd8e6817 |

## Dispatch waves

- W1: harness leaf (Python-only) + orchestrator-side release build, parallel.
- W2: baseline survey (single agent; owns the bench_report.json writer role).
- W3: base64-simd leaf + all 3 audit leaves in parallel (file-disjoint:
  base64 owns builtins_modules.rs b64 arms; audits write only .md docs).
- W4: hexlify-simd after base64 is REVIEWED (shares simd_encode.rs).
- W5: final report after W3+W4 merged to trunk and gates re-run.
- Audit leaves may run ahead of W2: they write analysis with
  'PENDING baseline (W2)' ratio placeholders; orchestrator fills measured
  numbers at review. Audit leaves write ONLY their audits/*.md file.
- Gate-authoring leaf (diff_b64/diff_hex) runs ahead of the SIMD leaves
  so they inherit a validated RED→GREEN differential gate.
- Audit FIX leaves (if any FIXABLE verdicts): dispatched by master after
  01-encode-simd file ownership frees up (i.e. after W4 merges).

## Merge gates (verify-before-merge, every implementation leaf)

- Build the full gate dependency set before running gates:
  `cargo build --release -p ap-cli -p ap-host -p ap-aot-crt
  -p ap-aot-crt-hybrid` + debug probe bins in the leaf's target dir.
  The hybrid gate needs ap-host + BOTH CRT staticlibs; CI never built
  the release probe, so a leaf that skips this reports false reds.

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

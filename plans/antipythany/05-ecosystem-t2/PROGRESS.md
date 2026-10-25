# Plan 05 (ecosystem T2) - PROGRESS

## Leaf A-1 (RustPython donor adaptations, wave 1) - DISPATCHED 2026-10-24
- deled: impl sa-0-2b6f0c5e + separate-authorship test track sa-1-09518180
  (tests/test_a1_rustpython_backfill.py, xfail-first).
- Baseline: b5ca2fd (wip snapshot of prior in-flight sibling work; full diff
  also at /tmp/antipythany-wip-snapshot-20261024.diff).
- Scope: 6 items per leaves/a-1-rustpython-adaptations-w1.md (errno table,
  binascii qp/uu/hqx, bytes hex/fromhex sep, stat S_IMODE/S_IFMT,
  os.pipe/pipe2 + kill/killpg arms, os.utime ns/dir_fd).
- Gates on completion: cargo workspace 0F, pytest 1912+, corpus
  non-decreasing, xtask notices --check exit 0, differential cases in
  stdlib_modules_differential.rs.
- Orchestrator review protocol on delivery: fresh 5-10 probes vs python3.14
  chosen from the inventory's user-visible contract (not the agent's own
  probes), then commit by explicit pathspec.

## Wave 2 authored - A-2 (2026-10-24)
- leaves/a-2-rustpython-adaptations-w2.md: hashlib sha3/blake2/shake
  (algorithm layer only, no PyHasher), zlib one-shot via flate2, os
  dup/dup2/fstat/dir_fd, os.uname structseq, realpath symlinks, select
  error types. Excludes all needs-redesign items per donor policy.
- Test-mining track dispatched (sa-0-d1405236): mines donor Lib/test/ into
  tests/test_a12_mined_corpus.py, pinned against live python3.14.
- A-2 impl BLOCKED ON A-1 landing (same-file ownership:
  builtins_modules.rs).

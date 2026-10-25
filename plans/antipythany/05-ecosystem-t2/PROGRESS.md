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

## Test-mining track LANDED (sa-0-d1405236, commit 4440dcb)
- tests/test_a12_mined_corpus.py: 32 cases, 15 pass / 17 xfail on the
  pre-A-1 binary (verified by orchestrator run).
- 13 findings; orchestrator VERIFIED firsthand: (F1) silent hard-exit
  rc=0 truncated-stdout on unimplemented call_os arms (os.open, os.symlink
  probed) instead of AttributeError; (F2) native binascii.Error from
  unhexlify escapes try/except (rc=1, catch-block skipped) while manual
  raises are catchable; (F3) crc32(data, init) ignores init (1356327615
  vs CPython 3575297757); (F4) bytearray.fromhex returns bytes-typed
  object (b'\\xab' bytes vs bytearray(b'\\xab')); (F5) stat.filemode
  '?rwxr-xr-x' vs '?rwsr-xr-x' - setuid/setgid/sticky bits dropped.
- Other reported findings NOT yet firsthand-verified: b16decode casefold
  divergence, bytes.hex non-ASCII sep error text, os.kill EPERM ->
  PermissionError mapping + OSError str format, normpath('//a/b'),
  realpath(strict=True) ignored, utime 3-tuple accepted, hashlib object
  model gaps, errno count 56 vs 107.
- Donor-lag findings (not bugs in our tree): donor test_base64 kwargs
  (padded=/ignorechars=) do not exist in 3.14.7; donor test_binascii hqx
  functions (a2b_hqx/b2a_hqx/rle*) were REMOVED in CPython 3.14 - only
  crc_hqx remains. LEAF A-1 ITEM 2 SCOPE NOTE: qp/uu/crc_hqx still valid;
  hqx rle/a2b/b2a functions are 3.14-removed - implement only crc_hqx,
  not the removed hqx family (pin against live 3.14, not the donor file).
- Findings F1/F2 route to an interp-owned fix leaf (builtins_modules.rs
  region is A-1-owned until it lands); the byte-surface findings
  (F3/F4/F5) overlap A-1 items and ride into the A-1 review.

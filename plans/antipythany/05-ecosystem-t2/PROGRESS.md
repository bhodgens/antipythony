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

## A-1 REVIEW COMPLETE - LANDED (commit cfe2465, reviewed 2026-10-25)
- NOTE: the commit was made by the repo's control agent, not this session;
  contents match the reviewed state. Orchestrator verification FIRSTHAND:
  cargo workspace 392 passed / 0 failed (incl. gc_weakref_unittest_battery
  now GREEN via the interp.rs type-vs-anything __eq__ identity fix);
  pytest 1951 passed / 6 skipped / 660 xfailed / 82 xpassed exit 0;
  notices --check exit 0; errno count 107 == CPython 107.
- 10 fresh probes vs python3.14: bytes.hex sep forms EXACT, fromhex error
  position EXACT, errno alias/reverse-dict EXACT, stat S_IMODE/S_IFMT/UF_
  EXACT, os.pipe roundtrip OK, binascii qp roundtrip + crc_hqx EXACT.
- 3 RESIDUALS verified, routed into A-2 brief: bytearray.fromhex returns
  bytes-typed object (F4 unresolved); os.utime ns= loses precision
  (...123 reads back ...256 - f64 rounding in utimensat arg path);
  os.kill(1,0) str(e) '(1, Operation not permitted)' vs '[Errno 1] ...'
  (exc_hier py_str, finding only - interp-owned).
- Also still open: stat.filemode setuid/sticky ('?rwxr-xr-x' vs
  '?rwsr-xr-x'); F1 silent hard-exit on unimplemented call_os arms
  (os.open/os.symlink probed) - both routed to the A-2 agent's finding
  queue / a later surface leaf.
- WAVE 2 DISPATCHED (sa-0-4098dc0b): hashlib sha3/blake2/shake, zlib
  one-shot via flate2, os dup/dup2/fstat/dir_fd, os.uname, realpath
  symlinks + '//' preservation, select error types, plus the residuals.

## A-2 REVIEW COMPLETE - LANDED (commit 6fb081b, reviewed 2026-10-25)
- Orchestrator verification FIRSTHAND: cargo workspace 401 passed /
  0 failed (wave-1 was 392; +9 from new differentials); pytest
  1951 passed / 653 xfailed / 89 xpassed / 0 failed; notices --check
  exit 0 (178 deps). Committed by explicit pathspec (5 files, 2680
  insertions). Attribution markers verified (7x "adapted from
  RustPython" in builtins_modules.rs).
- Fresh probes vs python3.14, ALL EXACT: sha3_256(b"abc") digest,
  shake_128 variable-length hexdigest, algorithms_guaranteed,
  zlib roundtrip + adler32/crc32 incl. incremental seed,
  zlib.error text, realpath dangling-symlink + strict FileNotFoundError
  + '//tmp//a2r' collapse, os.dup write/read through the dup'd fd,
  os.fstat st_mode, os.uname fields, utime ns= EXACT (...123 readback).
- Residuals re-checked: kill(1,0) now PermissionError errno 1 (str
  format still pending, A-3); bytearray.fromhex still bytes-typed
  (needs ap-obj bytearray representation - A-3 gap, NOT assigned);
  F2 except-Exception escape CONFIRMED still open for zlib.error
  AND binascii.Error (A-3 item 2).
- A-2 agent-reported gaps routed: blake2 keyed params
  NotImplementedError (small follow-up leaf candidate); kqueue object
  excluded per policy (selectors falls back to PollSelector - behavior
  equivalent); platform.uname processor '' vs 'arm' (documented);
  zlib compressed bytes differ (valid-stream variance, roundtrip pinned).
- stdlib-provenance --check baseline FAIL (121 pre-existing problems)
  NOT touched by this campaign - includes stale select.py/zlib.py
  DEVIATIONS now shadowed by native modules; vendored-tree cleanup is a
  separate queue item.
- A-3 DISPATCHED (sa-0-bdc24419): 5 items - arm-miss hard-exit,
  except-Exception MRO walk, OSError str format, utime arity,
  Opaque type naming + stat_result sequence protocol.
- Remaining open items after A-3: bytearray ap-obj representation
  (interp-owned, needs leaf brief); blake2 keyed params; vendored
  select/zlib DEVIATIONS cleanup.

## Previously-unverified mining findings - NOW VERIFIED (2026-10-25)
- b16decode MISSING entirely: AttributeError on both impl and CPython
  3.14.7 ('module binascii has no attribute b16decode' - mining report
  wrong that CPython has it; the casefold divergence is moot at 3.14).
  CORRECTION 2026-10-25 (later probe): b64decode/b16decode/b16encode are
  ALL absent from CPython 3.14 - the aliases were removed in 3.14. Our
  impl lacking them is CORRECT, not a gap. Only a2b_base64/b2a_base64
  remain as a surface (A-4 item 2 rewritten accordingly).
- os.kill(1,0): impl raises OSError(1) where CPython raises
  PermissionError(1) - subclass taxonomy missing in call_os error
  mapping. Route with the exc_hier py_str finding.
- realpath(strict=True) silently IGNORED on impl (returns the path where
  CPython 3.14 raises FileNotFoundError errno 2) - already covered by
  A-2 item 5.
- os.utime(p, (1,2,3)) ACCEPTED by impl where CPython raises TypeError
  ("'times' must be either a tuple of two ints or None") - wave-1 agent
  fixed positional times but not tuple-length validation. Routed to A-2
  residuals.

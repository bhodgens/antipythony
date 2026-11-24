# 01-encode-simd orchestrator

Optimize the SIMD-class encode/decode primitives. These are the only
rows with a concrete upstream SOTA target far beyond CPython-as-shipped.

## File ownership (HARD)

This group owns EXACTLY:
- `crates/ap-run/src/builtins_modules.rs` — the b64/hexlify arms only
  (functions `b64_encode`, `b64_decode`, `call_binascii` hex arms, and
  the `base64`/`binascii` module builder call targets)
- `crates/ap-run/src/methods.rs` — the `(Value::Str/Bytes, "encode")`
  codec arms IF a leaf needs them (b64 is not routed there; expect no
  change)
- new file `crates/ap-run/src/simd_encode.rs` (shared SIMD helpers)

No other file. `02-parity-audit` writes only audit .md files; if it
finds b64/hexlify issues they come back here as new leaves.

## Child index

| leaf | what | depends |
|---|---|---|
| 01-base64-simd.md | P01+P02: port CPython PR #162443's algorithm to Rust | 00-harness baseline |
| 02-hexlify-simd.md | P03: nibble-SIMD hexlify/unhexlify | 01 (shares simd_encode.rs) |

## Dispatch Protocol

1. 01 first (establishes simd_encode.rs + the differential gate pattern).
2. 02 after 01 REVIEWED (reuses the helpers + gate pattern).
3. Byte-exactness gate ORDER per leaf: differential FIRST, perf SECOND.
   A perf win that breaks a corpus case is a revert.

## Byte-exactness gate (every leaf, before perf claims)

- `tests/corpus/values/V016-encode-decode-errors.py` + the base64/hex
  stdlib corpus cases green via ap-conform.
- Per-primitive differential probe: random 100k-call comparison vs
  CPython (results AND exception text), mirroring PR #162443's own
  differential test.
- Edge matrix: every length 0..512, invalid byte at every position,
  custom alphabets (a2b_base64 with bytes-arg), validate= kwarg.

## Completion Tracking Table

| leaf | status |
|---|---|
| 01-base64-simd.md | PENDING |
| 02-hexlify-simd.md | PENDING |

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

## Test Gate

Every optimization leaf carries a differential byte-exactness gate BEFORE perf work (leaf docs T1). Coverage matrix:

| primitive | differential probe | corpus gate | status |
|---|---|---|---|
| P01/P02 base64 | diff_b64.py | V016 + base64 corpus cases | PENDING |
| P03 hexlify | diff_hex.py | hex corpus cases | PENDING |


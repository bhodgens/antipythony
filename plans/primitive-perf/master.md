# PRIMITIVE-PERF — master plan

**Goal:** antipythony must match or beat "best possible Python performance"
(CPython 3.14 with the fastest available implementation per primitive:
C-optimized builtin, or the SIMD/state-of-the-art approach where CPython
has an open PR — e.g. base64 NEON PR #162443) on a fixed primitive slice.
Today our native builtins are scalar byte-loops; CPython beats or ties us
on most, and the SIMD-class primitives (base64, hexlify) would beat us by
20-50x once upstream lands.

**Machine:** Mac M2 Ultra (24-core, 64 GB), aarch64, NEON-capable.
**Reference:** python3.14 (PATH pin, version-string checked).
**Implementation:** antipythony RELEASE build, tier-0 (tree-walker) —
primitive benchmarks measure NATIVE CODE speed, not the JIT loop.
**Byte-exactness gate:** every optimized primitive must stay byte-exact
vs CPython 3.14 (ap-conform corpus + differential harness). Perf work
never trades semantics.

## Method contract (all leaves follow this)

- Harness: `tests/bench/primitive/` — ONE runner script
  (`run_bench.py`) + per-primitive probe files + JSON output
  (`bench_report.json`). The runner times, best-of-N with warmup, and
  emits: per-primitive (cpython_s, antipythony_s, ratio, target_ratio,
  verdict). Target ratio = the UPSTREAM-SOTA number where a specific PR
  establishes it (base64: 24 GB/s encode / 14 GB/s decode per PR
  #162443), else ratio >= 1.0 (beat CPython-as-shipped).
- Timing: `time.perf_counter()` loops inside the probe (both
  interpreters run the SAME python probe file — measures native call
  speed, zero harness skew). Best-of-5, min recorded (min = least noise).
- Sizes: 48 KiB and 1 MiB per data primitive (short inputs show neither
  scalar nor SIMD behavior).
- Verdict per primitive: PASS if antipythony >= target, FAIL with the
  measured gap otherwise.
- Machine policy: ONE heavy cargo lane, CARGO_BUILD_JOBS=4.

## Primitive slice (leaf ownership)

| # | primitive | current state | upstream SOTA target |
|---|---|---|---|
| P01 | base64 b64encode/b2a_base64 | scalar loop, 1.17 GB/s | 24 GB/s (NEON PR #162443) |
| P02 | base64 b64decode/a2b_base64 | scalar loop, 0.40 GB/s | 14 GB/s (NEON PR #162443) |
| P03 | hexlify/unhexlify | scalar loop | ~same SIMD family (Muła/Lemire nibble) — target 10x CPython |
| P04 | zlib/bz2/lzma compress|decompress | delegates? (audit first) | CPython C = floor; miniz-oxide parity |
| P05 | hashlib sha256/md5 | md-5/sha1 crates | parity with CPython OpenSSL |
| P06 | str methods: find/replace/split/join | byte loops | memchr-class (target: parity or better) |
| P07 | bytes/int conversions: int.from_bytes, hex, int(str) | audit | parity |
| P08 | sorted()/list.sort | audit | parity (pdqsort-class) |
| P09 | dict large-build + lookup | audit | parity |
| P10 | json.dumps/loads large payload | audit | serde-class target: 5-10x CPython |

Leaves audit first, THEN optimize only where antipythony loses. A
primitive already >= CPython ships its PASS row and stops.

## Child Index

See the tree below; dispatch order and group notes follow each node.

## Tree

```
plans/primitive-perf/
  master.md                          # this file
  00-harness/
    orchestrator.md
    01-harness-runner.md             # build run_bench.py + probe skeleton + JSON schema
    02-baseline-survey.md            # run ALL primitives antipythony-vs-cpython, emit baseline bench_report.json
  01-encode-simd/                    # the big wins first
    orchestrator.md
    01-base64-simd.md                # P01+P02 port of PR #162443 algorithm (NEON via core_arch or chunked u64/u32)
    02-hexlify-simd.md               # P03 nibble SIMD
  02-parity-audit/
    orchestrator.md
    01-hash-and-compress.md          # P04+P05
    02-str-and-bytes.md              # P06+P07
    03-sort-dict-json.md             # P08+P09+P10
  99-report/
    orchestrator.md
    01-final-report.md               # chart (SVG/ASCII) + markdown report vs targets
```

## Dispatch protocol

1. `00-harness` first (harness + baseline survey) — everything else reads
   its bench_report.json to pick work.
2. `01-encode-simd` + `02-parity-audit` dispatch in parallel after 00.
   Groups are file-disjoint: 01 owns builtins_modules.rs b64/hexlify
   arms; 02 is AUDIT-ONLY first (write audit docs, not code) — any fix
   leaves it finds come back through master as follow-up leaves so the
   two groups never edit the same file concurrently.
3. `99-report` last: consumes final bench_report.json, produces the
   chart + report at `docs/primitive-perf-report.md` +
   `tests/bench/primitive/chart.svg` (or ASCII in-doc).
4. Every implementation leaf: byte-exact differential gate FIRST
   (ap-conform values/stdlib slices + per-primitive differential probe),
   then perf. Never weaken a corpus case to pass.

## Interface contracts

- Runner JSON schema (frozen): see 00-harness/orchestrator.md.
- Optimization touches: `crates/ap-run/src/builtins_modules.rs` only
  (01 group); `02` group writes ONLY `plans/primitive-perf/02-parity-audit/audits/*.md`.
- Both groups re-run the full `tests/bench/primitive/run_bench.py` to
  refresh the shared `bench_report.json` — serialized by master between
  groups (single writer rule), never concurrent.

## Completion Tracking Table

| node | status |
|---|---|
| 00-harness | COMPLETE (harness + baseline + diff gates, all verified) |
| 01-encode-simd | COMPLETE (base64 + hexlify NEON, merged dd3eda0/a0dc0f8, byte-exact gates green) |
| 02-parity-audit | COMPLETE (3 audit docs merged 3cb3a55; fix leaves = follow-up queue) |
| 99-report | COMPLETE (docs/primitive-perf-report.md + chart.svg merged 1e25df5) |

## Exit gate

`docs/primitive-perf-report.md` exists with the chart; every P-row in
`bench_report.json` verdict PASS (antipythony >= upstream SOTA target)
or carries a documented, user-approved deferral with the measured gap.

## Review Checklist

- [ ] in-session review after every child return
- [ ] single-writer rule enforced on bench_report.json
- [ ] exit gate: report exists, every row PASS or documented deferral

## Coding Conventions

- Python probes: deterministic, < 80 lines, no tabs in f-strings.
- Rust: aarch64 guards + scalar fallbacks; no new deps without sign-off.
- Machine policy: ONE heavy cargo lane; CARGO_BUILD_JOBS=4.

## Integration Test Plan

- Byte-exact differential gates BEFORE perf claims in every optimization leaf.
- ap-conform values + stdlib slices green on merged tree at group completion.
- Final full bench re-run produces the report chart.

## Test Gate

- Harness group gate: runner green on smoke slice + schema-valid JSON (in-session validation).
- Audit group gate: every bench row for the family explained in the audit doc.
- Optimization leaves carry their own differential gates (see leaf docs).


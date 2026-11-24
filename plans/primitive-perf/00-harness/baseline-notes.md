# Baseline survey notes — leaf 00-harness/02-baseline-survey

Full run, all 9 probes × both sizes (48KiB / 1MiB), on the verified release
binaries. No `--only`, no `--sizes` filter. Load window: loadavg 5.18 at
launch (< 10 policy).

- Report: `tests/bench/primitive/bench_report.json` — 50 rows
  (17 PASS / 25 FAIL / 8 ERROR), generated 2026-11-24T20:47Z,
  CPython ref 3.14.7, antipythony build = release.
- Binaries: `target-primperf/release/antipythony`,
  `target-primperf/release/ap-conform` (orchestrator-verified); CPython
  ref `/opt/homebrew/bin/python3.14`.
- Row semantics: `target_gbs` = PR #162443 NEON numbers for
  base64.b64encode (24 GB/s) / b64decode (14 GB/s); every other row's
  target = CPython-as-measured ("beat CPython as shipped"). FAIL rank
  below uses `ratio_vs_target` ascending = biggest gap first.

## Per-row impl sites (grep'd + line-READ in this worktree)

All sites in `crates/ap-run/src/`. "scalar loop" = hand-rolled Rust loop
over bytes; "crate call" = delegates to a Rust crate; "interpreter walk"
= value-level dispatch inside the evaluator.

| id | primitive | impl site (file:line) | impl summary | SOTA target |
|----|-----------|----------------------|--------------|-------------|
| P01 | base64.b64encode | builtins_modules.rs:12716 (dispatch) → `fn b64_encode` :12816 | scalar bit-accumulator loop, byte-at-a-time into Vec | 24 GB/s, CPython PR #162443 (NEON) |
| P01 | base64.b64decode | builtins_modules.rs:12840 `fn b64_decode` | scalar per-nibble table decode loop | 14 GB/s, CPython PR #162443 (NEON) |
| P01 | binascii.b2a_base64 | builtins_modules.rs:12899 → b64_encode :12816 | b64_encode + trailing-newline kwarg handling | beat CPython (0.785 GB/s @48K) |
| P01 | binascii.a2b_base64 | builtins_modules.rs:12945 | b64_decode path w/ strict CPython-c port error surface | beat CPython (0.453 GB/s @1M) |
| P03 | binascii.hexlify | builtins_modules.rs:12887 | scalar loop, `format!("{b:02x}")` per byte (slow path); fast `bytes_to_hex_str` at :13480 exists but NOT wired here | beat CPython (1.66–1.72 GB/s) |
| P03 | binascii.unhexlify | builtins_modules.rs:13062 | utf8-lossy→String, whitespace filter pass, then `char::to_digit` pair loop | beat CPython (1.71–1.75 GB/s) |
| P04 | zlib.compress | builtins_modules.rs:12057 `build_zlib` (compress arm ~:12178) | crate call: flate2 `Compression::new(level)` | beat CPython (PASS both sizes) |
| P04 | zlib.decompress | builtins_modules.rs:12202 | crate call via flate2 `write::ZlibDecoder` (streaming-writer shape, not one-shot slice decode) | beat CPython (FAIL 0.37–0.50x) |
| P05 | hashlib.sha256 | builtins_modules.rs:11751 | crate call: RustCrypto sha2, but output via `format!("{:x}")` hex-string then re-bundled | beat CPython (FAIL 0.12–0.14x) |
| P05 | hashlib.md5 | builtins_modules.rs:11733 | crate call: RustCrypto md5, same digest→hex shape | beat CPython (FAIL 0.91–0.94x, near-parity) |
| P06 | str.find | builtins_modules.rs:17196 | borrow receiver + `str::find` (memchr-backed stdlib call) | beat CPython (PASS 2.94–4.42x) |
| P06 | str.replace | builtins_modules.rs:17175 | stdlib `str::replace` on borrowed receiver | beat CPython (PASS @1M 1.82x; FAIL @48K 0.88x) |
| P06 | str.split | builtins_modules.rs:17061 | borrowed-recv split, per-piece Value allocation (D9 borrow fix) | beat CPython (FAIL 0.45–0.50x) |
| P06 | str.join | methods.rs:1519 | generic-iterable path: `iterable_of_public` + per-item PyString walk (not the str-only bulk join) | beat CPython (FAIL 0.19–0.22x) |
| P06 | str.upper | methods.rs:1517 | stdlib `str::to_uppercase` | beat CPython (PASS 2.59–4.13x) |
| P07 | int.from_bytes | builtins_modules.rs:19189 | receiver-slot classmethod, byte-shift accumulate loop | beat CPython (PASS 1.48–1.78x) |
| P07 | int.to_bytes | methods.rs:1658 | per-byte shift/mask loop into Vec | beat CPython (PASS 1.47–1.64x) |
| P07 | int(decimal_str) | builtins.rs:702 `builtin_int` → `parse_int_str` :851 → `Int::parse_base10` | crate/Int-core base-10 parse with digit-limit guard | beat CPython (PASS @48K 2.24x; FAIL @1M 0.73x) |
| P07 | hex(big_int) | builtins.rs:2601 `builtin_hex` | u128 fast path + `bigint_to_base` fallback (declared-linear, beats CPython's quadratic) | beat CPython (PASS 44–928x) |
| P08 | sorted/list.sort | builtins.rs:2867 `builtin_sorted` (+ `sort_by_cmp_wrappers`) | decorate-sort-undecorate with `py_lt` comparator | n/a — ERROR (below) |
| P09 | dict.build/lookups | (runtime dict core, ap-obj PyDict + interp.rs getitem path) | n/a — ERROR (below) |
| P10 | json.dumps | builtins_modules.rs:7843 `json_dumps` | recursive tree-walk serializer, Value-matched arms | beat CPython (0.97x — FAIL; CORRECTED by orchestrator: original "PASS" claim was ungrounded, P10 rows were dropped by a runner size-filter bug, since fixed; measured 2026-11-24) |
| P10 | json.loads | builtins_modules.rs:7786 `json_loads` | hand-written recursive-descent parser | beat CPython (0.17-0.24x — FAIL, 4-6x slower than CPython; CORRECTED same way — matches audits/sort-dict-json.md Fix sketch) |

Impl-site spot-checks (orchestrator checklist item): ≥2 cites above were
confirmed by reading the actual lines this session — b64encode dispatch
(:12716–12719, scalar `b64_encode` call), hexlify (:12887–12894,
`format!("{b:02x}")` per-byte loop), sha256 (:11751–11755, sha2 Digest),
str.find (:17196–17230, borrowed `window.find`), int.to_bytes
(methods.rs:1658–1675, byte-shift loop), zlib decompress (:12202–12222,
flate2 write::ZlibDecoder).

## FAIL ranking (ratio_vs_target ascending — feeds dispatch order)

1. **P03 binascii.hexlify 0.0195** (48KiB; 0.02 @1MiB) — 1.66–1.72 →
   0.033 GB/s. The :12887 arm uses `format!("{b:02x}")` per byte while a
   table-based `bytes_to_hex_str` (:13480, RustPython-adapted) already
   exists in the same file unwired. Cheapest known fix in the tree.
2. **P01 base64.b64decode 0.0275** (0.0291 @1MiB) — vs the 14 GB/s NEON
   target: scalar table decode, needs SWAR/NEON-style wide decode.
3. **P01 base64.b64encode 0.047** (0.0524 @1MiB) — vs 24 GB/s target:
   scalar bit-accumulator; same SWAR/wide-store shape as decode. NOTE:
   antipythony already BEATS CPython here (1.13–1.26x) — the gap is only
   vs the PR #162443 SOTA line.
4. **P05 hashlib.sha256 0.1179** (0.1449 @1MiB) — 2.46 → 0.29 GB/s,
   8.5x off CPython. sha2 crate itself is fast; suspect the digest
   output path (`format!("{:x}")` + hex re-parse into bytes) and/or
   per-call Value/Bytes boxing dominating at 48KiB–1MiB block sizes.
5. **P03 binascii.unhexlify 0.1268** (0.1265 @1MiB) — extra lossy-utf8 +
   whitespace-filter pre-passes before the digit loop.
6. **P06 str.join 0.1857** (0.2208 @1MiB) — 14.2 → 2.64 GB/s, 5.4x off.
   Routes through the generic-iterable path (methods.rs:1519); a
   homogeneous list-of-str bulk join fast path would close most of it.
7. **P04 zlib.decompress 0.3734/0.4372** @1MiB (0.4685/0.5039 @48KiB) —
   flate2 writer-based decode; CPython uses zlib one-shot with its own
   allocation strategy. Pure Rust-crate shape issue.
8. **P06 str.split 0.4466** (0.5022 @1MiB) — per-piece Gc allocation.
9. **P01 binascii.a2b_base64 0.5068** (0.5019 @1MiB) — half of CPython;
   strict-port decode path.
10. **P07 int(decimal_str) 0.7333** @1MiB only (PASS @48KiB 2.24x) —
    loses CPython only on the largest parse; likely
    `_pylong.py`-style chunked algorithms vs our `Int::parse_base10`.
11. **P06 str.replace 0.8831** @48KiB only (PASS @1MiB 1.82x) —
    near-parity, small-size overhead.
12. **P01 binascii.b2a_base64 0.9027** @48KiB; **P05 hashlib.md5
    0.9116/0.9401** — near-parity rows, lowest dispatch priority.

## ERROR rows — feature gaps, not perf (audit-group findings)

- **P08 sorted(list[int]) + list.sort, all 4 rows**: antipythony fails
  at probe setup, exact stderr:
  ```
  File ".../tests/bench/primitive/probes/p08_sort.py", line 25, in make_data
      rng = random.Random(seed)
  AttributeError: 'module' object has no attribute 'Random' (module 'random')
  ```
  Feature gap: `random.Random` class missing from the native `random`
  module surface (module imports; `Random` doesn't exist). Sort itself
  never measured. Not fixed per leaf instructions.
- **P09 dict.build + dict.lookups, all 4 rows**: `TIMEOUT after 120s`
  at every size, including 48KiB. The probe is whole-file
  (module-level `KEYS_1M = [i * 2654435761 % (1 << 62) for i in
  range(1_000_000)]` list-comp + full workload before the first JSON
  line emits), so a single slow stage — likely the 1M-element
  list-comp of modulo ops in the tree-walking interpreter — starves
  all four rows even though the measured dict ops themselves are
  small at the 48KiB tier. Classification: slow-not-hung per the
  smoke-slice finding; likely interpreter-loop overhead on the
  1M-iteration comprehension + generator-free list construction, not
  dict hashing per se. Follow-up for the audit group: either extend
  the harness timeout for P09 (runner change, out of this leaf's
  scope) or hand-measure the dict ops standalone.

## Checklist

- [x] bench_report.json has a row for every probe × size (50 rows =
      9 probes × ~2 sizes × primitives; every CPython-measured
      primitive has a row, PASS/FAIL/ERROR)
- [x] every FAIL row has impl_site + SOTA target above
- [x] ranking table present and consistent with the JSON
      (recomputed from bench_report.json this session)
